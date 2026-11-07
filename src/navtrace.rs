//! `--simulate-nav`: drives the camera through the same mouse routing as the window (wheel and
//! Alt-wheel at a pointer position) and logs what the view is made of after each step. A
//! diagnostic for bugs that appear only while navigating, so it can run with tiles still loading.
//!
//! Script: comma-separated steps.
//!   `pX:Y`  put the pointer there        `z+N`  wheel N notches (positive zooms in)
//!   `a+N`   Alt-wheel N notches (tilts)  `fN`   run N frames of 1/30 s without waiting for tiles
//!   `s`     run until everything loaded  `l`    log now        `d`  also list every drawn tile
//!   `uR`    survey, for each zoom level, how far its mesh stands above the finest loaded ground
//!           within R metres of the view point
//!   `g:NAME` save the picture as NAME.png in the `--nav-shots` folder
//! A line is logged after every `f`, `s` and `l`.

use crate::camera::Camera;
use crate::geo::{ecef_to_geodetic, TileKey};
use crate::nav::Mods;
use crate::render::Renderer;
use crate::{Args, Scene};
use anyhow::{anyhow, bail, Context, Result};
use glam::DVec2;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum Step {
    Pointer(f64, f64),
    Wheel(f64),
    AltWheel(f64),
    Frames(u32),
    Settle,
    Log,
    List,
    /// Survey the height differences between tile levels within this many metres of the view point.
    Survey(f64),
    Grab(String),
}

pub fn parse(script: &str) -> Result<Vec<Step>> {
    let num = |s: &str| s.parse::<f64>().map_err(|_| anyhow!("--simulate-nav: bad number in {s:?}"));
    let mut out = Vec::new();
    for tok in script.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        out.push(match tok {
            "s" => Step::Settle,
            "l" => Step::Log,
            "d" => Step::List,
            _ if tok.starts_with('u') => Step::Survey(num(&tok[1..])?),
            _ if tok.starts_with("g:") => Step::Grab(tok[2..].to_string()),
            _ if tok.starts_with('p') => {
                let (x, y) = tok[1..].split_once(':').ok_or_else(|| anyhow!("--simulate-nav: pointer is pX:Y"))?;
                Step::Pointer(num(x)?, num(y)?)
            }
            _ if tok.starts_with("z") => Step::Wheel(num(tok[1..].trim_start_matches('+'))?),
            _ if tok.starts_with("a") => Step::AltWheel(num(tok[1..].trim_start_matches('+'))?),
            _ if tok.starts_with('f') => Step::Frames(tok[1..].parse().context("--simulate-nav: fN needs a whole number")?),
            _ => bail!("--simulate-nav: unknown step {tok:?}"),
        });
    }
    Ok(out)
}

const DT: f64 = 1.0 / 30.0;

/// The largest change of the camera's height between two frames, up and down, and where.
#[derive(Default)]
struct Jumps {
    last: Option<f64>,
    up: (f64, usize),
    down: (f64, usize),
    frames: usize,
    lift: (f64, usize),
}

impl Jumps {
    fn note(&mut self, h: f64, lifted: f64) {
        self.frames += 1;
        // A rise in a frame where the ground held or lifted the camera is the ground's doing; the
        // amount `lifted` alone also counts a descent that the ground stopped.
        if let Some(last) = self.last.filter(|_| lifted > 0.0) {
            if h - last > self.lift.0 {
                self.lift = (h - last, self.frames);
            }
        }
        if let Some(last) = self.last {
            let d = h - last;
            if d > self.up.0 {
                self.up = (d, self.frames);
            }
            if -d > self.down.0 {
                self.down = (-d, self.frames);
            }
        }
        self.last = Some(h);
    }
}

fn frame(scene: &mut Scene, r: &mut Renderer, size: (u32, u32), jumps: &mut Jumps) {
    let aspect = size.0 as f64 / size.1 as f64;
    scene.update(r, DT, (aspect, size.1 as f64));
    scene.set_panels(r, size.0, size.1, 1);
    scene.place_labels(r, size, 1, DT);
    r.flush_uploads();
    let h = ecef_to_geodetic(scene.nav.cam.basis().pos).2;
    jumps.note(h, scene.nav.lifted);
    std::thread::sleep(Duration::from_millis(15));
}

/// Renders the scene to the target and returns its RGBA pixels.
fn grab(r: &mut Renderer, scene: &mut Scene, target: &wgpu::Texture, size: (u32, u32)) -> Result<Vec<u8>> {
    let (width, height) = size;
    let view = target.create_view(&Default::default());
    let bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nav readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let draw = scene.draw.clone();
    scene.prepare_draw(r);
    r.draw(&view, size, &scene.nav.cam, scene.nav.altitude, &draw, |encoder| {
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    });
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the GPU")?;
    let data = slice.get_mapped_range().map_err(|e| anyhow!("reading the image back: {e:?}"))?;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in data.chunks(bytes_per_row as usize) {
        pixels.extend_from_slice(&row[..(width * 4) as usize]);
    }
    Ok(pixels)
}

/// Shares of the picture that are pure black (the clear colour) and magenta (the base sphere,
/// with `--mark-underlay`), as fractions of all pixels. The second is the share of holes.
fn colour_shares(pixels: &[u8]) -> (f64, f64) {
    let (mut black, mut hole) = (0usize, 0usize);
    for p in pixels.chunks_exact(4) {
        let (r, g, b) = (p[0], p[1], p[2]);
        if r < 3 && g < 3 && b < 3 {
            black += 1;
        } else if r > 200 && g < 60 && b > 200 {
            hole += 1;
        }
    }
    let n = (pixels.len() / 4).max(1) as f64;
    (black as f64 / n, hole as f64 / n)
}

fn tile_under(draw: &[TileKey], lat: f64, lon: f64) -> Option<TileKey> {
    draw.iter().copied().find(|k| TileKey::containing(lat, lon, k.z) == *k)
}

/// The nearest mesh vertex of the drawn tiles inside the view (its depth along the view
/// direction), and how many such vertices are nearer than the near plane, so clipped away.
fn nearest_in_view(scene: &Scene, r: &Renderer, aspect: f64, near: f64) -> (f64, usize) {
    let b = scene.nav.cam.basis();
    let t = (crate::camera::FOV_Y / 2.0).tan();
    let (mut nearest, mut clipped) = (f64::MAX, 0);
    let n = crate::loader::GRID;
    for k in &scene.draw {
        let Some(heights) = r.tile_heights(*k) else { continue };
        for j in 0..=n {
            for i in 0..=n {
                let (lat, lon) = k.lat_lon(i as f64 / n as f64, j as f64 / n as f64);
                let rel = crate::geo::geodetic_to_ecef(lat, lon, heights[j * (n + 1) + i] as f64) - b.pos;
                let depth = rel.dot(b.look);
                if depth <= 0.0 || (rel.dot(b.right) / depth).abs() > t * aspect || (rel.dot(b.up) / depth).abs() > t {
                    continue;
                }
                nearest = nearest.min(depth);
                clipped += (depth < near) as usize;
            }
        }
    }
    (nearest, clipped)
}

/// Cracks between drawn tiles that a skirt does not cover. Walks the edge of every drawn tile and
/// compares its mesh height with the neighbouring drawn tile's at the same place: where the
/// higher one stands more than its skirt above the lower one, one sees through to the base
/// sphere. Returns the worst such step beyond the skirt, how many edge points show one, and the pair.
type WorstCrack = Option<(TileKey, TileKey, f64, f64)>;

fn open_cracks(scene: &Scene, r: &Renderer) -> (f64, usize, WorstCrack) {
    let n = crate::loader::GRID;
    let (mut worst, mut count, mut pair) = (0.0f64, 0usize, None);
    for k in &scene.draw {
        let Some(h) = r.tile_heights(*k) else { continue };
        let width = std::f64::consts::TAU * crate::geo::radius() / crate::body::current().grid.columns(k.z) as f64;
        let skirt = (width * 0.02).clamp(5.0, 50_000.0);
        let eps = 1e-4;
        for e in 0..=n {
            let f = e as f64 / n as f64;
            let edges = [(0, e, -eps, f), (n, e, 1.0 + eps, f), (e, 0, f, -eps), (e, n, f, 1.0 + eps)];
            for (i, j, s, t) in edges {
                let own = h[j * (n + 1) + i] as f64;
                let (lat, lon) = k.lat_lon(s, t);
                let Some(nk) = tile_under(&scene.draw, lat, lon).filter(|nk| nk != k) else { continue };
                let Some(other) = r.tile_report(nk, lat, lon).and_then(|t| t.height_here) else { continue };
                let exposed = own - other - skirt;
                if exposed > 0.0 {
                    count += 1;
                    if exposed > worst {
                        (worst, pair) = (exposed, Some((*k, nk, own, other)));
                    }
                }
            }
        }
    }
    (worst, count, pair)
}

/// Prints one line. Returns whether the view looks wrong: the camera under the drawn surface, or
/// more than 1% of the picture showing the base sphere.
fn log(scene: &Scene, r: &Renderer, label: &str, pixels: Option<&[u8]>, list: bool, aspect: f64) -> bool {
    let cam: &Camera = &scene.nav.cam;
    let b = cam.basis();
    let (clat, clon, ch) = ecef_to_geodetic(b.pos);
    let nav_ground = r.ground(clat, clon);
    let target_ground = r.ground(cam.lat, cam.lon);
    let drawn = tile_under(&scene.draw, clat, clon).and_then(|k| r.tile_report(k, clat, clon).map(|t| (k, t)));
    let drawn_here = drawn.as_ref().and_then(|(_, t)| t.height_here);
    let drawn_target = tile_under(&scene.draw, cam.lat, cam.lon).and_then(|k| r.tile_report(k, cam.lat, cam.lon).map(|t| t.height_here));
    let reports: Vec<(TileKey, crate::render::TileReport)> =
        scene.draw.iter().filter_map(|k| r.tile_report(*k, clat, clon).map(|t| (*k, t))).collect();
    let flat = reports.iter().filter(|(_, t)| t.min == 0.0 && t.max == 0.0).count();
    let coarse_img = reports.iter().filter(|(_, t)| t.imagery_size < 0.99).count();
    let (zmin, zmax) = (scene.draw.iter().map(|k| k.z).min().unwrap_or(0), scene.draw.iter().map(|k| k.z).max().unwrap_or(0));
    let hi = reports.iter().map(|(_, t)| t.max).fold(f32::MIN, f32::max);
    let f = |v: Option<f64>| v.map_or("none".to_string(), |v| format!("{v:.1}"));
    let under = match drawn_here {
        Some(h) => format!("{}", ch < h),
        None => "n/a".into(),
    };
    let near = cam.near_plane(scene.nav.altitude);
    let (crack, crack_points, crack_pair) = open_cracks(scene, r);
    let (front, clipped) = nearest_in_view(scene, r, aspect, near);
    let shares = pixels.map(colour_shares).map_or(String::new(), |(bl, nv)| format!(" black={:.3} hole={:.3}", bl, nv));
    println!(
        "[{label}] cam=({:.5},{:.5}) h={:.1} alt={:.1} dist={:.1} tilt={:.1}deg target_h={:.1} nav_ground={} target_ground={} drawn_under_cam={} drawn_under_target={} below_drawn={} near={:.2} tiles={} z={}..{} flat={} coarse_imagery={} wait={} tallest_drawn={:.0} front={:.1} clipped={} crack={:.0} crack_pts={}{}",
        clat.to_degrees(),
        clon.to_degrees(),
        ch,
        scene.nav.altitude,
        cam.distance,
        cam.tilt.to_degrees(),
        cam.target_h,
        f(nav_ground),
        f(target_ground),
        f(drawn_here),
        f(drawn_target.flatten()),
        under,
        cam.near_plane(scene.nav.altitude),
        scene.draw.len(),
        zmin,
        zmax,
        flat,
        coarse_img,
        scene.waiting,
        hi,
        front,
        clipped,
        crack,
        crack_points,
        shares,
    );
    if let Some((a, b, ha, hb)) = crack_pair {
        println!("    worst open crack: z{}/{}/{} at {:.0} m above z{}/{}/{} at {:.0} m", a.z, a.x, a.y, ha, b.z, b.x, b.y, hb);
    }
    if let Some((k, t)) = &drawn {
        println!("    under camera: z{}/{}/{} heights {:.0}..{:.0} imagery_part={}", k.z, k.x, k.y, t.min, t.max, t.imagery_size);
    }
    if list {
        for (k, t) in &reports {
            println!("    z{}/{}/{} heights {:.0}..{:.0} imagery_part={}", k.z, k.x, k.y, t.min, t.max, t.imagery_size);
        }
    }
    under == "true" || pixels.is_some_and(|p| colour_shares(p).1 > 0.01)
}

fn save_png(dir: &Option<std::path::PathBuf>, name: &str, px: &[u8], size: (u32, u32)) -> Result<()> {
    let Some(dir) = dir else { return Ok(()) };
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{name}.png"));
    image::save_buffer(&path, px, size.0, size.1, image::ExtendedColorType::Rgba8).with_context(|| path.display().to_string())
}

/// For each zoom level, compares the mesh of that level's tile with the finest loaded ground on a
/// grid of points around the view point, where a tile of zoom 10 or finer exists. A camera
/// standing on the finest ground is under a drawn mesh of that level where the difference is positive.
fn survey(scene: &Scene, r: &Renderer, radius: f64) {
    let cam = &scene.nav.cam;
    let n = 40;
    let per_deg = std::f64::consts::PI / 180.0;
    let dlat = radius / 111_320.0;
    let dlon = dlat / cam.lat.cos().max(0.01);
    let height = |k: TileKey, lat: f64, lon: f64| r.tile_report(k, lat, lon).and_then(|t| t.height_here);
    // The points with their finest loaded tile's zoom and the ground height there.
    let mut points = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let lat = cam.lat + dlat * (2.0 * j as f64 / n as f64 - 1.0);
            let lon = cam.lon + dlon * (2.0 * i as f64 / n as f64 - 1.0);
            let finest = (0..=crate::loader::TILE_MAX_Z).rev().find(|z| r.tile_report(TileKey::containing(lat, lon, *z), lat, lon).is_some());
            if let Some(z) = finest.filter(|z| *z >= 10) {
                if let Some(h) = height(TileKey::containing(lat, lon, z), lat, lon) {
                    points.push((lat, lon, z, h));
                }
            }
        }
    }
    let (lo, hi) = points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.3), b.max(p.3)));
    println!("  survey +-{radius:.0} m around ({:.4},{:.4}): {} points on finest tiles of zoom 10 or more", cam.lat / per_deg, cam.lon / per_deg, points.len());
    if !points.is_empty() {
        println!("    ground {lo:.0} to {hi:.0} m (relief {:.0} m)", hi - lo);
    }
    println!("    level  points  mesh-above-ground max / p95 / mean (m)   mesh-below-ground max (m)");
    for z in 4..=13u8 {
        let mut diffs: Vec<f64> = points
            .iter()
            .filter(|p| p.2 > z)
            .filter_map(|&(lat, lon, _, fine)| height(TileKey::containing(lat, lon, z), lat, lon).map(|c| c - fine))
            .collect();
        if diffs.is_empty() {
            continue;
        }
        diffs.sort_by(f64::total_cmp);
        let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
        let p95 = diffs[(diffs.len() as f64 * 0.95) as usize % diffs.len()];
        println!("    z{z:<4}  {:6}  {:8.1} / {:7.1} / {:7.1}   {:8.1}", diffs.len(), diffs[diffs.len() - 1], p95, mean, -diffs[0]);
    }
}

/// Runs the script. `target` is the picture texture of `--screenshot`.
pub fn run(args: &Args, script: &[Step], scene: &mut Scene, r: &mut Renderer, target: &wgpu::Texture, size: (u32, u32)) -> Result<()> {
    let mut pointer = DVec2::new(size.0 as f64 / 2.0, size.1 as f64 / 2.0);
    let mut list = false;
    let aspect = size.0 as f64 / size.1 as f64;
    let mut n = 0;
    let started = std::time::Instant::now();
    let mut jumps = Jumps::default();
    for step in script {
        match step {
            Step::Pointer(x, y) => pointer = DVec2::new(*x, *y),
            Step::Wheel(l) | Step::AltWheel(l) => {
                scene.mouse_moved(pointer);
                let mods = Mods { alt: matches!(step, Step::AltWheel(_)), ..Mods::default() };
                scene.mouse_wheel(*l, mods);
            }
            Step::List => list = true,
            Step::Survey(radius) => survey(scene, r, *radius),
            Step::Frames(c) => {
                for _ in 0..*c {
                    frame(scene, r, size, &mut jumps);
                }
                n += 1;
                let px = grab(r, scene, target, size)?;
                if log(scene, r, &format!("f{n}"), Some(&px), list, aspect) {
                    save_png(&args.nav_shots, &format!("auto-f{n}"), &px, size)?;
                }
            }
            Step::Settle => {
                loop {
                    frame(scene, r, size, &mut jumps);
                    if scene.loader.is_idle() && scene.waiting == 0 {
                        break;
                    }
                    if started.elapsed().as_secs_f64() > args.timeout {
                        eprintln!("settle timed out");
                        break;
                    }
                }
                for _ in 0..60 {
                    frame(scene, r, size, &mut jumps);
                }
                n += 1;
                let px = grab(r, scene, target, size)?;
                if log(scene, r, &format!("s{n}"), Some(&px), list, aspect) {
                    save_png(&args.nav_shots, &format!("auto-s{n}"), &px, size)?;
                }
            }
            Step::Log => {
                n += 1;
                let px = grab(r, scene, target, size)?;
                if log(scene, r, &format!("l{n}"), Some(&px), list, aspect) {
                    save_png(&args.nav_shots, &format!("auto-l{n}"), &px, size)?;
                }
            }
            Step::Grab(name) => {
                let dir = args.nav_shots.clone().ok_or_else(|| anyhow!("g: needs --nav-shots DIR"))?;
                std::fs::create_dir_all(&dir)?;
                let px = grab(r, scene, target, size)?;
                let path = dir.join(format!("{name}.png"));
                image::save_buffer(&path, &px, size.0, size.1, image::ExtendedColorType::Rgba8).with_context(|| path.display().to_string())?;
            }
        }
    }
    println!(
        "camera height between frames ({} frames): largest rise {:.1} m at frame {}, largest fall {:.1} m at frame {}; largest rise while the ground held the camera {:.1} m at frame {}",
        jumps.frames, jumps.up.0, jumps.up.1, jumps.down.0, jumps.down.1, jumps.lift.0, jumps.lift.1
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_script() {
        let s = parse("p10:20, z+3, a-2, f5, s, l, d, g:one").unwrap();
        assert_eq!(
            s,
            vec![Step::Pointer(10.0, 20.0), Step::Wheel(3.0), Step::AltWheel(-2.0), Step::Frames(5), Step::Settle, Step::Log, Step::List, Step::Grab("one".into())]
        );
        assert!(parse("q").is_err());
        assert!(parse("pab").is_err());
    }

    #[test]
    fn holes_and_black_are_told_apart() {
        let px = [0, 0, 0, 255, 255, 0, 255, 255, 200, 200, 200, 255, 254, 3, 250, 255];
        assert_eq!(colour_shares(&px), (0.25, 0.5));
    }
}
