//! `--record DIR`: keeps what the window shows, for looking at a fault afterwards.
//!
//! Every drawn frame is copied from the window's own surface, so the images are what was on
//! screen, without the mouse pointer. Reading back and encoding happen beside the drawing, so
//! that recording disturbs the timing as little as it can: when they fall behind, a frame loses
//! its image, and the log says so. `frames.jsonl` gets one line per drawn frame and per input,
//! written as it happens, so it survives a crash. When the images outgrow their limit the oldest
//! are deleted, so the end of a long session is always there.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use crate::wtime::Instant;

/// Frames being copied by the GPU at one time.
const MAX_IN_FLIGHT: usize = 4;
/// Frames waiting for an encoder.
const QUEUE: usize = 16;
const ENCODERS: usize = 2;

/// The images on disk, oldest first, and what they may add up to.
struct DiskBudget {
    limit: u64,
    used: u64,
    kept: VecDeque<(PathBuf, u64)>,
    deleted: u64,
}

impl DiskBudget {
    fn new(limit: u64) -> Self {
        Self { limit, used: 0, kept: VecDeque::new(), deleted: 0 }
    }

    /// Notes a new image and gives back the old ones that now have to go. The newest always stays.
    fn add(&mut self, path: PathBuf, bytes: u64) -> Vec<PathBuf> {
        self.kept.push_back((path, bytes));
        self.used += bytes;
        let mut out = Vec::new();
        while self.used > self.limit && self.kept.len() > 1 {
            let (old, size) = self.kept.pop_front().expect("more than one image is kept");
            self.used -= size;
            self.deleted += 1;
            out.push(old);
        }
        out
    }
}

/// Removes the row padding and the alpha, and puts red first.
fn tight_rgb(data: &[u8], width: u32, bytes_per_row: u32, blue_first: bool) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((data.len() / 4) * 3);
    for row in data.chunks(bytes_per_row as usize) {
        for p in row[..(width * 4) as usize].chunks_exact(4) {
            let (r, b) = if blue_first { (p[2], p[0]) } else { (p[0], p[2]) };
            rgb.extend_from_slice(&[r, p[1], b]);
        }
    }
    rgb
}

fn write_png(path: &Path, rgb: &[u8], width: u32, height: u32) -> Result<u64> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let file = std::io::BufWriter::new(File::create(path)?);
    PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Adaptive).write_image(
        rgb,
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(fs::metadata(path)?.len())
}

/// Makes the folder, and refuses one in use, so that nothing already there is overwritten.
fn prepare(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir.join("frames")).with_context(|| format!("creating {}", dir.display()))?;
    let used = fs::read_dir(dir)?.filter_map(Result::ok).any(|e| e.file_name() != "frames")
        || fs::read_dir(dir.join("frames"))?.next().is_some();
    if used {
        bail!("{} is not empty; --record needs a new folder for each recording", dir.display());
    }
    Ok(())
}

fn image_name(number: u64) -> String {
    format!("frames/{number:06}.png")
}

struct Image {
    name: String,
    width: u32,
    height: u32,
    blue_first: bool,
    bytes_per_row: u32,
    data: Vec<u8>,
}

fn encoder_loop(queue: Arc<Mutex<Receiver<Image>>>, dir: PathBuf, budget: Arc<Mutex<DiskBudget>>, complained: Arc<AtomicBool>) {
    loop {
        // The lock is only held while waiting, not while encoding.
        let received = queue.lock().expect("no encoder panics while waiting").recv();
        let Ok(image) = received else { return };
        let rgb = tight_rgb(&image.data, image.width, image.bytes_per_row, image.blue_first);
        let path = dir.join(&image.name);
        match write_png(&path, &rgb, image.width, image.height) {
            Ok(bytes) => {
                let old = budget.lock().expect("no encoder panics while counting").add(path, bytes);
                for file in old {
                    let _ = fs::remove_file(file);
                }
            }
            Err(e) => {
                if !complained.swap(true, Ordering::Relaxed) {
                    eprintln!("recording: could not write {}: {e:#}", path.display());
                }
            }
        }
    }
}

/// A frame's place in the readback, between asking for its copy and the GPU having done it.
pub struct Slot {
    number: u64,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: u32,
}

impl Slot {
    /// Adds the copy of the finished picture to the frame's commands.
    pub fn copy(&self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
    }
}

struct InFlight {
    slot: Slot,
    mapped: Receiver<Result<(), wgpu::BufferAsyncError>>,
}

pub struct Recorder {
    dir: PathBuf,
    device: wgpu::Device,
    started: Instant,
    blue_first: bool,
    /// Frames drawn so far.
    number: u64,
    /// Where the pointer last was: images do not show it.
    pub cursor: Option<(f64, f64)>,
    free: Vec<wgpu::Buffer>,
    in_flight: VecDeque<InFlight>,
    images: Option<SyncSender<Image>>,
    encoders: Vec<JoinHandle<()>>,
    budget: Arc<Mutex<DiskBudget>>,
    log: File,
    lost: u64,
}

impl Recorder {
    /// Starts a recording in a new folder. `format` is the window surface's.
    pub fn start(dir: &Path, limit_gib: f64, device: wgpu::Device, format: wgpu::TextureFormat) -> Result<Self> {
        use wgpu::TextureFormat as F;
        let blue_first = match format {
            F::Bgra8Unorm | F::Bgra8UnormSrgb => true,
            F::Rgba8Unorm | F::Rgba8UnormSrgb => false,
            other => return Err(anyhow!("--record cannot read a window surface of format {other:?}")),
        };
        prepare(dir)?;
        let log = File::create(dir.join("frames.jsonl")).context("creating the frame log")?;
        let budget = Arc::new(Mutex::new(DiskBudget::new((limit_gib * (1u64 << 30) as f64) as u64)));
        let complained = Arc::new(AtomicBool::new(false));
        let (images, queue) = mpsc::sync_channel(QUEUE);
        let queue = Arc::new(Mutex::new(queue));
        let encoders = (0..ENCODERS)
            .map(|_| {
                let (queue, dir, budget, complained) = (queue.clone(), dir.to_path_buf(), budget.clone(), complained.clone());
                std::thread::spawn(move || encoder_loop(queue, dir, budget, complained))
            })
            .collect();
        println!("recording to {} (the newest {limit_gib} GiB of images are kept)", dir.display());
        Ok(Self {
            dir: dir.to_path_buf(),
            device,
            started: Instant::now(),
            blue_first,
            number: 0,
            cursor: None,
            free: Vec::new(),
            in_flight: VecDeque::new(),
            images: Some(images),
            encoders,
            budget,
            log,
            lost: 0,
        })
    }

    /// Starts a frame. `None` when all readback buffers are in use, and this frame gets no image.
    pub fn begin(&mut self, width: u32, height: u32) -> (u64, Option<Slot>) {
        self.number += 1;
        self.collect();
        let bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let size = (bytes_per_row * height) as u64;
        self.free.retain(|b| b.size() == size);
        let buffer = match self.free.pop() {
            Some(b) => b,
            None if self.in_flight.len() < MAX_IN_FLIGHT => self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("recording readback"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            None => {
                self.lost += 1;
                return (self.number, None);
            }
        };
        (self.number, Some(Slot { number: self.number, buffer, width, height, bytes_per_row }))
    }

    /// Asks for the pixels, once the frame's commands have been submitted.
    pub fn submitted(&mut self, slot: Slot) {
        let (done, mapped) = mpsc::channel();
        slot.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = done.send(result);
        });
        self.in_flight.push_back(InFlight { slot, mapped });
    }

    /// Passes on the frames the GPU has finished, oldest first. True while some are still out.
    pub fn collect(&mut self) -> bool {
        if self.in_flight.is_empty() {
            return false;
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
        while let Some(front) = self.in_flight.front() {
            match front.mapped.try_recv() {
                Err(TryRecvError::Empty) => break,
                outcome => {
                    let done = self.in_flight.pop_front().expect("the front was just seen");
                    self.finish(done.slot, matches!(outcome, Ok(Ok(()))));
                }
            }
        }
        !self.in_flight.is_empty()
    }

    fn finish(&mut self, slot: Slot, mapped: bool) {
        let data = mapped.then(|| slot.buffer.slice(..).get_mapped_range().ok().map(|view| view.to_vec())).flatten();
        if mapped {
            slot.buffer.unmap();
        }
        let image = data.map(|data| Image {
            name: image_name(slot.number),
            width: slot.width,
            height: slot.height,
            blue_first: self.blue_first,
            bytes_per_row: slot.bytes_per_row,
            data,
        });
        let sent = match (image, &self.images) {
            (Some(image), Some(images)) => match images.try_send(image) {
                Ok(()) => true,
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
            },
            _ => false,
        };
        if !sent {
            self.lost += 1;
            self.line(&json!({ "lost_image": slot.number }));
        }
        self.free.push(slot.buffer);
    }

    /// The log's line for a drawn frame, with the time and the frame's number and image added.
    pub fn log_frame(&mut self, number: u64, has_image: bool, mut state: Value) {
        state["frame"] = json!(number);
        state["t"] = json!(self.started.elapsed().as_secs_f64());
        state["image"] = if has_image { json!(image_name(number)) } else { Value::Null };
        state["cursor"] = json!(self.cursor);
        self.line(&state);
    }

    /// The log's line for an input, with the time added.
    pub fn log_input(&mut self, input: Value) {
        let t = self.started.elapsed().as_secs_f64();
        self.line(&json!({ "t": t, "after_frame": self.number, "input": input }));
    }

    /// One write for each line, so that a crash leaves whole lines behind.
    fn line(&mut self, value: &Value) {
        let _ = self.log.write_all(format!("{value}\n").as_bytes());
    }
}

impl Drop for Recorder {
    /// Waits for what is still on its way, so that the last frames, the fault's, are kept.
    fn drop(&mut self) {
        while !self.in_flight.is_empty() {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            self.collect();
        }
        self.images = None;
        for encoder in self.encoders.drain(..) {
            let _ = encoder.join();
        }
        let b = self.budget.lock().expect("the encoders have ended");
        println!(
            "recorded {} frames to {}: {} images kept ({} MB), {} deleted over the limit, {} without an image",
            self.number,
            self.dir.display(),
            b.kept.len(),
            b.used / (1 << 20),
            b.deleted,
            self.lost,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orbis-recorder-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_oldest_images_go_first() {
        let mut b = DiskBudget::new(100);
        assert!(b.add("1".into(), 60).is_empty());
        assert!(b.add("2".into(), 40).is_empty());
        assert_eq!(b.add("3".into(), 50), vec![PathBuf::from("1")]);
        assert_eq!((b.used, b.deleted, b.kept.len()), (90, 1, 2));
    }

    #[test]
    fn the_newest_image_stays_even_over_the_limit() {
        let mut b = DiskBudget::new(10);
        assert!(b.add("1".into(), 500).is_empty());
        assert_eq!(b.add("2".into(), 500), vec![PathBuf::from("1")]);
        assert_eq!(b.kept.len(), 1);
    }

    #[test]
    fn padding_and_alpha_go_and_red_comes_first() {
        // Two rows of one pixel, each padded to 8 bytes.
        let data = [1, 2, 3, 255, 9, 9, 9, 9, 4, 5, 6, 255, 9, 9, 9, 9];
        assert_eq!(tight_rgb(&data, 1, 8, false), [1, 2, 3, 4, 5, 6]);
        assert_eq!(tight_rgb(&data, 1, 8, true), [3, 2, 1, 6, 5, 4]);
    }

    #[test]
    fn an_image_reads_back_as_written() {
        let dir = temp_dir("png");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        let rgb = [10, 20, 30, 40, 50, 60];
        assert!(write_png(&path, &rgb, 2, 1).unwrap() > 0);
        assert_eq!(image::open(&path).unwrap().to_rgb8().into_raw(), rgb);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_in_use_is_refused_and_left_alone() {
        let dir = temp_dir("used");
        prepare(&dir).unwrap();
        prepare(&dir).expect("a folder still empty can be used");
        let log = dir.join("frames.jsonl");
        fs::write(&log, "old").unwrap();
        assert!(prepare(&dir).is_err());
        assert_eq!(fs::read_to_string(&log).unwrap(), "old");
        fs::remove_file(&log).unwrap();
        fs::write(dir.join("frames/000001.png"), "old").unwrap();
        assert!(prepare(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
