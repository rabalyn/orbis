mod buildings;
mod cache;
mod camera;
mod fetcher;
mod geo;
mod labels;
mod lan;
mod loader;
mod nav;
mod overlay;
mod pacing;
mod render;
mod select;
mod servers;
mod sources;
mod text;
mod vector;

use anyhow::{anyhow, bail, Context, Result};
use cache::DiskCache;
use camera::Camera;
use fetcher::{Fetcher, Peers};
use geo::TileKey;
use glam::DVec2;
use labels::{Screen, TextKey, TextWorker, ATLAS_SIZE};
use loader::{Loader, Outcome, VECTOR_MAX_Z};
use nav::{Arrow, Button, Mods, Nav, WHOLE_EARTH};
use pacing::{FrameLimit, Pacer, RateMeter};
use render::{hemisphere_label, Corner, LabelInstance, Renderer};
use select::View;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// The data credits. These must remain visible wherever the data is shown.
const CREDITS: &str = "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH \
(Contains modified Copernicus Sentinel data 2024)\n\
Terrain: © Mapterhorn https://mapterhorn.com/attribution\n\
Map: OpenFreeMap © OpenMapTiles Data from OpenStreetMap";
const GIB: u64 = 1024 * 1024 * 1024;
const CACHE_LIMIT_GIB: u64 = 10;
/// A long-running program checks its cache's size this often.
const TRIM_EVERY: Duration = Duration::from_secs(3600);
const UPLOADS_PER_FRAME: usize = 12;
/// Label sets for this many vector tiles stay in memory.
const LABEL_SETS: usize = 2000;
/// The most labels considered per frame, most important first.
const LABEL_CANDIDATES: usize = 1500;
/// Fading a label in takes the inverse of this, in seconds.
const LABEL_FADE_RATE: f64 = 6.0;
/// A label already on screen gets this much priority, so that labels do not trade places.
const LABEL_STAY_BONUS: i32 = 60;
const USAGE: &str = "\
orbis: a globe viewer

Usage: orbis [options]

  --lat DEG  --lon DEG    view point (default 30, 10)
  --distance METRES       distance from the view point (default whole Earth)
  --heading DEG           heading, 0 is north up
  --tilt DEG              tilt, 0 is straight down
  --exaggeration N        terrain height factor (default 1)
  --lang CODE             language for names, such as de or en (default from the locale)
  --no-roads              start with roads and boundaries hidden (key R)
  --no-labels             start with labels hidden (key L)
  --no-buildings          start with 3D buildings hidden (key B)
  --fps 120|90            frame rate limit (default 120, key F switches)
  --benchmark SECONDS     turn the view in the window, print frame timings and exit
  --cache-dir PATH        tile cache location
  --cache-limit GB        largest size of the tile cache (default 10)
  --cache-status          print the cache size and server timeouts, then exit
  --offline               send no requests, and use only what is cached
  --serve-cache           run as a LAN cache server, with no window
  --share-cache           also offer this cache on the LAN, and use others found there
  --cache-server HOST:PORT  use this LAN cache server (repeatable)
  --cache-port PORT       port to serve on (default 47813)
  --screenshot FILE.png   render once all tiles arrive, write the image and exit
  --size WxH              window size (default 1280x800), or screenshot size (default 1600x1000)
  --timeout SECONDS       longest wait for tiles in a screenshot (default 90)
  --help                  show this text

See docs/SPEC.md for the mouse and keyboard controls.";

struct Args {
    cam: Camera,
    exaggeration: f32,
    cache_dir: PathBuf,
    screenshot: Option<PathBuf>,
    size: (u32, u32),
    size_given: bool,
    timeout: f64,
    lang: String,
    roads: bool,
    labels: bool,
    buildings: bool,
    limit: FrameLimit,
    benchmark: Option<f64>,
    cache_limit_gib: u64,
    cache_status: bool,
    offline: bool,
    serve_cache: bool,
    share_cache: bool,
    cache_servers: Vec<String>,
    cache_port: u16,
}

fn locale_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| vector::language_from_locale(&value))
        .find(|lang| !lang.is_empty())
        .unwrap_or_default()
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut a = Args {
        cam: Camera {
            lat: 30f64.to_radians(),
            lon: 10f64.to_radians(),
            target_h: 0.0,
            distance: WHOLE_EARTH,
            heading: 0.0,
            tilt: 0.0,
        },
        exaggeration: 1.0,
        cache_dir: DiskCache::default_root(),
        screenshot: None,
        size: (1600, 1000),
        size_given: false,
        timeout: 90.0,
        lang: locale_language(),
        roads: true,
        labels: true,
        buildings: true,
        limit: FrameLimit::Fps120,
        benchmark: None,
        cache_limit_gib: CACHE_LIMIT_GIB,
        cache_status: false,
        offline: false,
        serve_cache: false,
        share_cache: false,
        cache_servers: Vec::new(),
        cache_port: lan::TCP_PORT,
    };
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--help" | "-h" => return Ok(None),
            "--no-roads" => {
                a.roads = false;
                continue;
            }
            "--no-labels" => {
                a.labels = false;
                continue;
            }
            "--no-buildings" => {
                a.buildings = false;
                continue;
            }
            "--cache-status" => {
                a.cache_status = true;
                continue;
            }
            "--offline" => {
                a.offline = true;
                continue;
            }
            "--serve-cache" => {
                a.serve_cache = true;
                continue;
            }
            "--share-cache" => {
                a.share_cache = true;
                continue;
            }
            _ => {}
        }
        let value = it.next().ok_or_else(|| anyhow!("{flag} needs a value"))?;
        let num = || -> Result<f64> {
            value.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| anyhow!("{flag}: not a number: {value}"))
        };
        match flag.as_str() {
            "--lat" => a.cam.lat = num()?.clamp(-89.9, 89.9).to_radians(),
            "--lon" => a.cam.lon = num()?.to_radians(),
            "--distance" => a.cam.distance = num()?,
            "--heading" => a.cam.heading = num()?.to_radians(),
            "--tilt" => a.cam.tilt = num()?.to_radians(),
            "--exaggeration" => a.exaggeration = num()? as f32,
            "--timeout" => a.timeout = num()?,
            "--cache-limit" => {
                a.cache_limit_gib = value.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                    anyhow!("--cache-limit expects a whole number of gigabytes")
                })?;
            }
            "--cache-server" => {
                let valid = value.rsplit_once(':').is_some_and(|(host, port)| {
                    !host.is_empty()
                        && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-[]:".contains(c))
                        && port.parse::<u16>().is_ok_and(|p| p != 0)
                });
                if !valid {
                    bail!("--cache-server expects HOST:PORT");
                }
                a.cache_servers.push(value.clone());
            }
            "--cache-port" => {
                a.cache_port = value.parse().ok().filter(|p| *p != 0).ok_or_else(|| {
                    anyhow!("--cache-port expects a port number")
                })?;
            }
            "--fps" => {
                a.limit = FrameLimit::parse(&value).ok_or_else(|| anyhow!("--fps expects 120 or 90"))?;
            }
            "--benchmark" => {
                let seconds = num()?;
                if seconds <= 0.0 {
                    bail!("--benchmark expects a positive number of seconds");
                }
                a.benchmark = Some(seconds);
            }
            "--lang" => {
                if !value.chars().all(|c| c.is_ascii_alphabetic()) {
                    bail!("--lang expects a language code such as de");
                }
                a.lang = value.to_ascii_lowercase();
            }
            "--cache-dir" => a.cache_dir = value.into(),
            "--screenshot" => a.screenshot = Some(value.into()),
            "--size" => {
                let (w, h) = value.split_once('x').ok_or_else(|| anyhow!("--size expects WxH"))?;
                a.size = (w.parse().context("--size width")?, h.parse().context("--size height")?);
                if a.size.0 == 0 || a.size.1 == 0 {
                    bail!("--size must not be zero");
                }
                a.size_given = true;
            }
            _ => bail!("unknown option {flag}\n\n{USAGE}"),
        }
    }
    a.cam.clamp();
    Ok(Some(a))
}

/// Everything between input and drawing that does not depend on a window.
struct Scene {
    nav: Nav,
    loader: Loader,
    /// When a failed tile may be tried again, and how many times it has failed.
    retries: HashMap<TileKey, (Instant, u32)>,
    missing: HashSet<TileKey>,
    draw: Vec<TileKey>,
    /// Tiles and overlays the view needs that are not loaded yet.
    waiting: usize,
    /// How many of those have failed before.
    retrying: usize,
    roads_on: bool,
    labels_on: bool,
    /// Tiles whose map data contains no lines, so they need no overlay.
    overlay_empty: HashSet<TileKey>,
    overlay_retries: HashMap<TileKey, (Instant, u32)>,
    /// Labels by the vector tile they came from.
    label_sets: HashMap<TileKey, Arc<Vec<vector::Label>>>,
    text: TextWorker,
    /// How visible each label on screen is, as it fades in.
    label_alpha: HashMap<u64, f64>,
    labels_shown: usize,
    buildings_on: bool,
    /// Tiles known to have no buildings.
    building_empty: HashSet<TileKey>,
    building_retries: HashMap<TileKey, (Instant, u32)>,
    /// A line about the frame rate for the on-screen display, when drawing to a window.
    frame_status: Option<String>,
    fetcher: Arc<Fetcher>,
    /// Keeps a slow tick going for tiles waiting out a retry delay.
    last_retry_tick: Instant,
}

/// Sets up tile fetching, including sharing on the LAN if asked for.
fn make_fetcher(args: &Args) -> Arc<Fetcher> {
    let cache = DiskCache::new(args.cache_dir.clone());
    let trimmer = cache.clone();
    let limit = args.cache_limit_gib * GIB;
    std::thread::spawn(move || loop {
        trimmer.trim(limit);
        std::thread::sleep(TRIM_EVERY);
    });
    let peers = Peers::new(args.cache_servers.clone());
    Arc::new(Fetcher::new(cache, peers, args.offline))
}

/// Opens the LAN port, and announces this machine there.
fn start_sharing(fetcher: &Arc<Fetcher>, args: &Args, mode: lan::Mode) -> Result<()> {
    let listener = std::net::TcpListener::bind(("0.0.0.0", args.cache_port))
        .with_context(|| format!("could not open port {}", args.cache_port))?;
    lan::serve(listener, fetcher.clone(), mode);
    let id = lan::instance_id();
    lan::announce(id, args.cache_port, mode);
    // A dedicated server fetches for others and asks nobody itself.
    let looking = (mode == lan::Mode::SharedApp).then(|| fetcher.clone());
    lan::discover(id, Some((args.cache_port, mode)), looking);
    Ok(())
}

/// Runs as a LAN cache server until stopped.
fn serve_cache(args: &Args) -> Result<()> {
    let fetcher = make_fetcher(args);
    start_sharing(&fetcher, args, lan::Mode::Dedicated)?;
    println!("Serving the tile cache on port {}.", args.cache_port);
    println!("  Tiles:  http://THIS-MACHINE:{}/tiles/SOURCE/Z/X/Y", args.cache_port);
    println!("  Status: http://THIS-MACHINE:{}/status", args.cache_port);
    println!("  Sources: {}", sources::all().map(|s| s.id).collect::<Vec<_>>().join(", "));
    println!("Credit the data when displaying it; see docs/PROVENANCE.md. Press Ctrl-C to stop.");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB as f64 {
        format!("{:.1} GB", b / GIB as f64)
    } else {
        format!("{:.1} MB", b / MIB)
    }
}

/// How long until a time, in whole minutes, rounding up.
fn minutes_until(until: SystemTime, now: SystemTime) -> u64 {
    until.duration_since(now).map_or(0, |d| d.as_secs().div_ceil(60))
}

/// Prints what the cache holds and which servers are being left alone.
fn cache_status(args: &Args) {
    let cache = DiskCache::new(args.cache_dir.clone());
    let (entries, bytes) = cache.usage();
    println!("Cache: {}", args.cache_dir.display());
    println!("  {entries} tiles, {} of {} GB", format_bytes(bytes), args.cache_limit_gib);
    let servers = servers::Servers::new(Some(cache.state_path("servers.txt")));
    let now = SystemTime::now();
    let paused: Vec<_> = servers.report(now).into_iter().filter(|r| r.paused_until.is_some()).collect();
    if paused.is_empty() {
        println!("  No servers are timed out.");
    }
    for r in paused {
        let until = r.paused_until.unwrap_or(now);
        println!("  {} is timed out for {} more minutes", r.host, minutes_until(until, now));
    }
}

fn vector_source(k: TileKey) -> TileKey {
    k.ancestor(k.z.min(VECTOR_MAX_Z))
}

/// The building tiles covering the drawn tiles. Height data only exists at one zoom level,
/// so finer tiles share the mesh of the tile they lie in, and a tile one level coarser uses
/// its four children. That moves the line where buildings end further into the distance.
fn building_tiles(draw: &[TileKey]) -> Vec<TileKey> {
    let mut keys: Vec<TileKey> = Vec::new();
    for k in draw {
        if k.z >= vector::BUILDING_Z {
            keys.push(k.ancestor(vector::BUILDING_Z));
        } else if k.z + 1 == vector::BUILDING_Z {
            keys.extend(k.children());
        }
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn label_id(label: &vector::Label) -> u64 {
    let mut h = DefaultHasher::new();
    label.text.hash(&mut h);
    // Coarse enough that the same place from a neighbouring zoom level counts as the same.
    ((label.lat * 2000.0) as i64, (label.lon * 2000.0) as i64).hash(&mut h);
    h.finish()
}

/// Schedules another attempt: when a server said so, or else after a growing delay.
fn next_retry(retries: &mut HashMap<TileKey, (Instant, u32)>, k: TileKey, until: Option<SystemTime>) {
    let n = retries.get(&k).map_or(1, |e| e.1 + 1);
    let told = until.and_then(|u| u.duration_since(SystemTime::now()).ok());
    let delay = told.unwrap_or(Duration::from_secs(2u64.saturating_pow(n.min(8))));
    retries.insert(k, (Instant::now() + delay, n));
}

/// The credits for what is on screen: those always shown, then any aerial imagery in view.
fn credits_for<'a>(detail: impl Iterator<Item = &'a str>) -> String {
    let names: BTreeSet<&str> = detail.filter(|c| !c.is_empty()).collect();
    if names.is_empty() {
        return CREDITS.to_string();
    }
    let list: Vec<&str> = names.into_iter().collect();
    format!("{CREDITS}\nAerial images: {}", list.join(", "))
}

/// Says whether more is coming: a finished view and a stuck one otherwise look the same.
fn loading_status(waiting: usize, retrying: usize) -> String {
    match (waiting.saturating_sub(retrying), retrying) {
        (0, 0) => "all loaded".to_string(),
        (loading, 0) => format!("loading {loading}"),
        (0, retrying) => format!("{retrying} waiting to retry"),
        (loading, retrying) => format!("loading {loading}, {retrying} waiting to retry"),
    }
}

/// The on-screen line about traffic, and a line for each server being left alone.
fn network_status(fetcher: &Fetcher, now: SystemTime) -> Vec<String> {
    let t = fetcher.traffic();
    let total = t.disk_hits + t.lan_hits + t.downloads;
    let mut lines = Vec::new();
    if let Some(saved) = ((t.disk_hits + t.lan_hits) * 100).checked_div(total) {
        let lan = if t.lan_hits > 0 { format!("  LAN {}", t.lan_hits) } else { String::new() };
        lines.push(format!("downloaded {}  cached {saved}%{lan}", format_bytes(t.downloaded_bytes)));
    }
    for r in fetcher.server_report() {
        if let Some(until) = r.paused_until {
            lines.push(format!("timed out: {} {} min", r.host, minutes_until(until, now)));
        }
    }
    lines
}

impl Scene {
    fn new(args: &Args, size: DVec2) -> Self {
        let fetcher = make_fetcher(args);
        if args.share_cache {
            // The app is useful without sharing, for example if the port is taken.
            if let Err(e) = start_sharing(&fetcher, args, lan::Mode::SharedApp) {
                eprintln!("cache sharing is off: {e:#}");
            }
        }
        Self {
            nav: Nav::starting_above_ground(args.cam, size),
            loader: Loader::new(fetcher.clone(), args.exaggeration, vector::name_fields(&args.lang)),
            fetcher,
            last_retry_tick: Instant::now(),
            retries: HashMap::new(),
            missing: Default::default(),
            draw: Vec::new(),
            waiting: 0,
            retrying: 0,
            roads_on: args.roads,
            labels_on: args.labels,
            overlay_empty: HashSet::new(),
            overlay_retries: HashMap::new(),
            label_sets: HashMap::new(),
            text: TextWorker::new(),
            label_alpha: HashMap::new(),
            labels_shown: 0,
            buildings_on: args.buildings,
            building_empty: HashSet::new(),
            building_retries: HashMap::new(),
            frame_status: None,
        }
    }

    fn receive(&mut self, r: &mut Renderer) -> bool {
        let changed = self.collect_results(r);
        // Not every update is followed by a frame: a hidden window draws none.
        if changed {
            r.flush_uploads();
        }
        changed
    }

    fn collect_results(&mut self, r: &mut Renderer) -> bool {
        let mut changed = false;
        for _ in 0..UPLOADS_PER_FRAME {
            match self.loader.try_result() {
                Some(Outcome::Ready(data)) => {
                    r.insert_tile(&data);
                    self.retries.remove(&data.key);
                    changed = true;
                }
                Some(Outcome::Missing(k)) => {
                    self.missing.insert(k);
                }
                Some(Outcome::Retry(k, until)) => next_retry(&mut self.retries, k, until),
                Some(Outcome::Overlay(data)) => {
                    self.overlay_retries.remove(&data.key);
                    self.label_sets.insert(data.source, data.labels.clone());
                    match &data.mips {
                        Some(mips) => r.insert_overlay(data.key, mips),
                        None => {
                            self.overlay_empty.insert(data.key);
                        }
                    }
                    changed = true;
                }
                Some(Outcome::OverlayRetry(k, until)) => next_retry(&mut self.overlay_retries, k, until),
                Some(Outcome::Buildings(k, mesh)) => {
                    self.building_retries.remove(&k);
                    match mesh {
                        Some(mesh) => r.insert_buildings(k, &mesh),
                        None => {
                            self.building_empty.insert(k);
                        }
                    }
                    changed = true;
                }
                Some(Outcome::BuildingsRetry(k, until)) => next_retry(&mut self.building_retries, k, until),
                // The workers keep these themselves.
                Some(Outcome::Deferred(_)) => {}
                None => break,
            }
        }
        changed
    }

    /// Advances the view and works out what to draw and fetch. Returns whether the picture changed.
    fn update(&mut self, r: &mut Renderer, dt: f64, aspect_height: (f64, f64)) -> bool {
        let received = self.receive(r);
        {
            let ground = |lat: f64, lon: f64| r.ground(lat, lon);
            self.nav.tick(dt, &ground);
        }
        let view = View {
            basis: self.nav.cam.basis(),
            aspect: aspect_height.0,
            height_px: aspect_height.1,
            max_z: sources::max_tile_z,
        };
        let selection = view.select(&|k| r.has_tile(k));
        let now = Instant::now();
        let wanted: Vec<TileKey> = selection
            .wanted
            .into_iter()
            .filter(|k| !self.missing.contains(k))
            .collect();
        // Tiles waiting out a retry delay still count as outstanding, but are not requested yet.
        self.waiting = wanted.len();
        self.retrying = wanted.iter().filter(|k| self.retries.contains_key(k)).count();
        let wanted: Vec<TileKey> = wanted
            .into_iter()
            .filter(|k| self.retries.get(k).is_none_or(|e| e.0 <= now))
            .collect();
        let changed = received || selection.draw != self.draw;
        self.draw = selection.draw;

        let mut overlays = Vec::new();
        if self.roads_on || self.labels_on {
            for &k in &self.draw {
                let lines_needed = !r.has_overlay(k) && !self.overlay_empty.contains(&k);
                if !lines_needed && self.label_sets.contains_key(&vector_source(k)) {
                    continue;
                }
                self.waiting += 1;
                match self.overlay_retries.get(&k) {
                    Some(e) if e.0 > now => self.retrying += 1,
                    Some(_) => {
                        self.retrying += 1;
                        overlays.push(k);
                    }
                    None => overlays.push(k),
                }
            }
        }
        let mut building_jobs = Vec::new();
        let visible = if self.buildings_on { building_tiles(&self.draw) } else { Vec::new() };
        for &k in &visible {
            if r.has_buildings(k) || self.building_empty.contains(&k) {
                continue;
            }
            self.waiting += 1;
            match self.building_retries.get(&k) {
                Some(e) if e.0 > now => self.retrying += 1,
                Some(_) => {
                    self.retrying += 1;
                    building_jobs.push(k);
                }
                None => building_jobs.push(k),
            }
        }
        r.set_visible_buildings(visible);
        self.loader.set_wanted(&wanted, &overlays, &building_jobs);
        // Once everything has arrived, the terrain under the start view is known for good.
        if self.waiting == 0 && self.loader.is_idle() {
            self.nav.finish_start();
        }
        changed
    }

    /// Chooses the labels for this frame and hands them to the renderer.
    fn place_labels(&mut self, r: &mut Renderer, (width, height): (u32, u32), scale: u32, dt: f64) {
        while let Some(result) = self.text.try_result() {
            if let Ok(mask) = result {
                r.receive_mask(mask);
            }
        }
        self.labels_shown = 0;
        if !self.labels_on {
            r.set_labels(&[]);
            self.label_alpha.clear();
            return;
        }
        let sources: HashSet<TileKey> = self.draw.iter().map(|k| vector_source(*k)).collect();
        if self.label_sets.len() > LABEL_SETS {
            self.label_sets.retain(|k, _| sources.contains(k));
        }
        // In a fixed order: labels with the same name nearby share an identity, so only their
        // position in this list keeps them from swapping between frames.
        let mut ordered: Vec<TileKey> = sources.iter().copied().collect();
        ordered.sort_unstable();
        let sets: Vec<_> = ordered.iter().filter_map(|k| self.label_sets.get(k).cloned()).collect();

        let pad = |p: [f64; 4]| [p[0] - 4.0, p[1] - 4.0, p[2] + 4.0, p[3] + 4.0];
        let screen = Screen {
            basis: self.nav.cam.basis(),
            size: DVec2::new(width as f64, height as f64),
            reserved: vec![
                pad(r.panel_rect(0, Corner::BottomLeft, width, height)),
                pad(r.panel_rect(1, Corner::TopRight, width, height)),
            ],
        };
        let mut shown: Vec<(&vector::Label, i32, u64)> = sets
            .iter()
            .flat_map(|set| set.iter())
            .filter(|label| labels::may_show(&screen, label))
            .map(|label| {
                let id = label_id(label);
                let bonus = if self.label_alpha.contains_key(&id) { LABEL_STAY_BONUS } else { 0 };
                (label, label.priority - bonus, id)
            })
            .collect();
        // The identity breaks ties, so equal labels keep their order from frame to frame.
        shown.sort_by_key(|s| (s.1, s.2));
        shown.truncate(LABEL_CANDIDATES);

        let mut candidates = Vec::with_capacity(shown.len());
        let mut details = Vec::with_capacity(shown.len());
        for (label, _, id) in &shown {
            let Some(text) = labels::usable_text(&label.text) else { continue };
            let key = TextKey { text, style: label.style, scale };
            let Some(rect) = r.label_rect(&key, &mut self.text) else { continue };
            candidates.push((*label, DVec2::new(rect.w as f64, rect.h as f64)));
            details.push((rect, *id));
        }

        let ground = |lat: f64, lon: f64| r.ground(lat, lon);
        let placed = labels::place(&screen, &candidates, &ground);

        let mut alpha = HashMap::with_capacity(placed.len());
        let atlas = ATLAS_SIZE as f32;
        let instances: Vec<LabelInstance> = placed
            .iter()
            .map(|p| {
                let (rect, id) = details[p.index];
                let look = labels::appearance(candidates[p.index].0.style);
                let opacity = self.label_alpha.get(&id).copied().unwrap_or(0.0);
                let opacity = (opacity + dt * LABEL_FADE_RATE).min(1.0);
                alpha.insert(id, opacity);
                LabelInstance {
                    center: [p.center.x as f32, p.center.y as f32],
                    half: [rect.w as f32 / 2.0, rect.h as f32 / 2.0],
                    uv: [
                        rect.x as f32 / atlas,
                        rect.y as f32 / atlas,
                        (rect.x + rect.w) as f32 / atlas,
                        (rect.y + rect.h) as f32 / atlas,
                    ],
                    fill: [look.fill[0], look.fill[1], look.fill[2], opacity as f32],
                    halo: [look.halo[0], look.halo[1], look.halo[2], p.angle as f32],
                }
            })
            .collect();
        self.labels_shown = instances.len();
        self.label_alpha = alpha;
        r.set_labels(&instances);
    }

    /// True while labels are still arriving or fading in.
    fn labels_changing(&self) -> bool {
        !self.text.is_idle() || self.label_alpha.values().any(|a| *a < 1.0)
    }

    fn hud(&self, r: &Renderer) -> String {
        let cam = &self.nav.cam;
        let mut text = format!(
            "{}\naltitude {}  tiles {}/{}\n{}",
            hemisphere_label(cam.lat, cam.lon),
            format_distance(self.nav.altitude),
            self.draw.len(),
            r.tile_count(),
            loading_status(self.waiting, self.retrying),
        );
        if let Some(status) = &self.frame_status {
            text.push('\n');
            text.push_str(status);
        }
        for line in network_status(&self.fetcher, SystemTime::now()) {
            text.push('\n');
            text.push_str(&line);
        }
        text
    }

    fn set_panels(&self, r: &mut Renderer, width: u32, scale: usize) {
        let columns = ((width as usize).saturating_sub(40) / (8 * scale)).max(20);
        let credits = credits_for(self.draw.iter().map(|k| r.tile_credit(*k)));
        r.set_panel(0, &credits, columns, scale);
        r.set_panel(1, &self.hud(r), columns, scale);
    }

    /// True about once a second while tiles wait out a retry delay, which is often enough to
    /// notice one coming due without redrawing for the whole of a server's timeout.
    fn retry_tick(&mut self, now: Instant) -> bool {
        if self.retrying == 0 || now.duration_since(self.last_retry_tick) < Duration::from_secs(1) {
            return false;
        }
        self.last_retry_tick = now;
        true
    }
}

fn format_distance(m: f64) -> String {
    if m >= 10_000.0 {
        format!("{:.0} km", m / 1000.0)
    } else if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{m:.0} m")
    }
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("orbis"),
        ..Default::default()
    }))
    .context("no suitable graphics device")
}

fn screenshot(args: &Args, path: &PathBuf) -> Result<()> {
    let (width, height) = args.size;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .context("no graphics adapter")?;
    let (device, queue) = request_device(&adapter)?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let samples = render::sample_count(&adapter, format);
    let mut r = Renderer::new(device, queue, format, samples);
    r.roads_visible = args.roads;
    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let aspect = width as f64 / height as f64;
    let mut scene = Scene::new(args, DVec2::new(width as f64, height as f64));

    let started = Instant::now();
    let mut quiet_since = Instant::now();
    let step = 1.0 / 30.0;
    loop {
        scene.update(&mut r, step, (aspect, height as f64));
        scene.set_panels(&mut r, width, 1);
        scene.place_labels(&mut r, (width, height), 1, step);
        r.flush_uploads();
        if !scene.loader.is_idle() || scene.waiting > 0 || !scene.text.is_idle() {
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() > Duration::from_millis(300) {
            break;
        }
        if started.elapsed().as_secs_f64() > args.timeout {
            eprintln!("timed out with {} tiles outstanding", scene.waiting);
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    // A few steps let the view point settle on the terrain.
    for _ in 0..90 {
        scene.update(&mut r, step, (aspect, height as f64));
    }
    scene.set_panels(&mut r, width, 1);
    let text_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        scene.place_labels(&mut r, (width, height), 1, step);
        r.flush_uploads();
        if !scene.labels_changing() || Instant::now() > text_deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    // Labels on screen keep their place, so which ones won so far depends on the order their
    // text arrived. Choosing afresh, fully faded in, makes saved images repeatable.
    scene.label_alpha.clear();
    scene.place_labels(&mut r, (width, height), 1, 1.0);

    let frame_started = Instant::now();
    scene.update(&mut r, 0.0, (aspect, height as f64));
    scene.place_labels(&mut r, (width, height), 1, 0.0);
    let frame_cpu = frame_started.elapsed();

    let bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let draw = scene.draw.clone();
    let gpu_started = Instant::now();
    r.draw(&view, (width, height), &scene.nav.cam, scene.nav.altitude, &draw, |encoder| {
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    });
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the GPU")?;
    let frame_gpu = gpu_started.elapsed();
    let data = slice.get_mapped_range().map_err(|e| anyhow!("reading the image back: {e:?}"))?;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in data.chunks(bytes_per_row as usize) {
        pixels.extend_from_slice(&row[..(width * 4) as usize]);
    }
    image::save_buffer(path, &pixels, width, height, image::ExtendedColorType::Rgba8)
        .with_context(|| format!("writing {}", path.display()))?;
    println!(
        "{}: {} tiles drawn, finest z{}, {} labels, {} building tiles, {:.1} ms frame logic, {:.1} ms drawing, {} tiles missing on the server, adapter {}",
        path.display(),
        draw.len(),
        draw.iter().map(|k| k.z).max().unwrap_or(0),
        scene.labels_shown,
        r.visible_building_count(),
        frame_cpu.as_secs_f64() * 1000.0,
        frame_gpu.as_secs_f64() * 1000.0,
        scene.missing.len(),
        adapter.get_info().name,
    );
    let t = scene.fetcher.traffic();
    println!(
        "traffic: {} downloads ({}), {} from disk, {} from LAN, {} shared downloads",
        t.downloads,
        format_bytes(t.downloaded_bytes),
        t.disk_hits,
        t.lan_hits,
        t.shared_downloads,
    );
    for r in scene.fetcher.server_report() {
        if let Some(until) = r.paused_until {
            println!("timed out: {} for {} min", r.host, minutes_until(until, SystemTime::now()));
        }
    }
    Ok(())
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    scene: Scene,
    last_frame: Instant,
    pacer: Pacer,
    meter: RateMeter,
    /// Something changed, so a frame should be drawn at the next allowed moment.
    redraw_wanted: bool,
    present_mode: wgpu::PresentMode,
    benchmark: Option<Benchmark>,
}

/// Turns the view steadily, so that frame timing can be measured without any input.
struct Benchmark {
    end: Instant,
    last: Option<Instant>,
    intervals: Vec<f64>,
}

struct App {
    args: Args,
    gfx: Option<Gfx>,
    error: Option<anyhow::Error>,
    mods: Mods,
    pointer: DVec2,
    pressed_at: Option<(Instant, DVec2)>,
    last_click: Option<(Instant, DVec2)>,
}

impl App {
    fn start(&self, event_loop: &ActiveEventLoop) -> Result<Gfx> {
        let (width, height) = if self.args.size_given { self.args.size } else { (1280, 800) };
        let window = Arc::new(event_loop.create_window(
            Window::default_attributes()
                .with_title("Orbis")
                .with_inner_size(PhysicalSize::new(width, height)),
        )?);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("no graphics adapter for this window")?;
        let (device, queue) = request_device(&adapter)?;
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let mut config = surface
            .get_default_config(&adapter, w, h)
            .ok_or_else(|| anyhow!("this window's surface is not supported"))?;
        let caps = surface.get_capabilities(&adapter);
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);
        config.present_mode = pacing::present_mode(&caps.present_modes);
        surface.configure(&device, &config);
        let samples = render::sample_count(&adapter, config.format);
        let mut renderer = Renderer::new(device, queue, config.format, samples);
        renderer.roads_visible = self.args.roads;
        let scene = Scene::new(&self.args, DVec2::new(w as f64, h as f64));
        let now = Instant::now();
        let benchmark = self.args.benchmark.map(|seconds| Benchmark {
            end: now + Duration::from_secs_f64(seconds),
            last: None,
            intervals: Vec::new(),
        });
        Ok(Gfx {
            present_mode: config.present_mode,
            window,
            surface,
            config,
            renderer,
            scene,
            last_frame: now,
            pacer: Pacer::new(self.args.limit, now),
            meter: RateMeter::default(),
            redraw_wanted: true,
            benchmark,
        })
    }

    fn redraw(gfx: &mut Gfx) {
        let now = Instant::now();
        gfx.pacer.frame_started(now);
        gfx.meter.record(now);
        let dt = gfx.last_frame.elapsed().as_secs_f64().min(0.1);
        gfx.last_frame = now;
        if let Some(b) = &mut gfx.benchmark {
            if let Some(last) = b.last {
                b.intervals.push((now - last).as_secs_f64());
            }
            b.last = Some(now);
            gfx.scene.nav.cam.heading += dt * 0.15;
        }
        let display_hz = gfx
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| mhz as f64 / 1000.0);
        gfx.scene.frame_status =
            Some(pacing::status_line(gfx.pacer.limit(), gfx.meter.rate(), display_hz));
        let (w, h) = (gfx.config.width, gfx.config.height);
        let aspect = w as f64 / h as f64;
        gfx.scene.update(&mut gfx.renderer, dt, (aspect, h as f64));
        let scale = gfx.window.scale_factor().round().max(1.0) as usize;
        gfx.scene.set_panels(&mut gfx.renderer, w, scale);
        gfx.scene.place_labels(&mut gfx.renderer, (w, h), scale as u32, dt);

        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                gfx.redraw_wanted = true;
                return;
            }
            other => {
                if !matches!(other, wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout) {
                    eprintln!("could not get a frame from the window");
                }
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let draw = gfx.scene.draw.clone();
        let (cam, altitude) = (gfx.scene.nav.cam, gfx.scene.nav.altitude);
        gfx.renderer.draw(&view, (w, h), &cam, altitude, &draw, |_| {});
        gfx.window.pre_present_notify();
        gfx.renderer.queue.present(frame);

        // Tiles only waiting out a retry delay are picked up by the slow tick instead.
        let scene = &gfx.scene;
        let loading = !scene.loader.is_idle() || scene.waiting > scene.retrying;
        if scene.nav.is_animating() || loading || scene.labels_changing() || gfx.benchmark.is_some() {
            gfx.redraw_wanted = true;
        }
    }

    /// Prints the measurements of a finished benchmark.
    fn report_benchmark(gfx: &Gfx, b: &Benchmark) {
        let display = gfx
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map_or("unknown".to_string(), |mhz| format!("{:.1} Hz", mhz as f64 / 1000.0));
        println!(
            "limit {} fps, present mode {:?}, display {display}, {}x{}",
            gfx.pacer.limit().hz(),
            gfx.present_mode,
            gfx.config.width,
            gfx.config.height,
        );
        match pacing::summarize(&b.intervals) {
            Some(s) => println!(
                "{} frames, average {:.1} fps, median {:.2} ms, 99th percentile {:.2} ms, worst {:.2} ms",
                s.frames, s.average_fps, s.median_ms, s.p99_ms, s.worst_ms,
            ),
            None => println!("no frames were drawn; the window may have been hidden"),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match self.start(event_loop) {
            Ok(g) => {
                g.window.request_redraw();
                self.gfx = Some(g);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // A slow tick picks up tiles whose retry time has come.
        let mut wake = now + Duration::from_secs(1);
        if let Some(g) = &mut self.gfx {
            if let Some(b) = &g.benchmark {
                if now >= b.end {
                    Self::report_benchmark(g, b);
                    event_loop.exit();
                    return;
                }
            }
            // Refreshes the on-screen rate once drawing has stopped.
            if g.scene.retry_tick(now) || g.meter.is_stale(now) {
                g.redraw_wanted = true;
            }
            if g.redraw_wanted {
                if g.pacer.due(now) {
                    g.redraw_wanted = false;
                    g.window.request_redraw();
                } else {
                    wake = g.pacer.next_frame_at();
                }
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gfx) = &mut self.gfx else { return };
        let nav = &mut gfx.scene.nav;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                Self::redraw(gfx);
                return;
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    gfx.config.width = size.width;
                    gfx.config.height = size.height;
                    gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                    nav.size = DVec2::new(size.width as f64, size.height as f64);
                }
            }
            WindowEvent::Focused(false) => nav.release_all(),
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Mods { shift: s.shift_key(), alt: s.alt_key(), ctrl: s.control_key() };
            }
            WindowEvent::CursorMoved { position: PhysicalPosition { x, y }, .. } => {
                self.pointer = DVec2::new(x, y);
                nav.moved(self.pointer);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => return,
                };
                match state {
                    ElementState::Pressed => {
                        nav.press(b, self.pointer, self.mods);
                        if b == Button::Left {
                            self.pressed_at = Some((Instant::now(), self.pointer));
                        }
                    }
                    ElementState::Released => {
                        nav.release();
                        let was_click = b == Button::Left
                            && self.pressed_at.is_some_and(|p| {
                                p.0.elapsed() < Duration::from_millis(300)
                                    && p.1.distance(self.pointer) < 5.0
                            });
                        if was_click {
                            let again = self.last_click.is_some_and(|c| {
                                c.0.elapsed() < Duration::from_millis(400)
                                    && c.1.distance(self.pointer) < 6.0
                            });
                            if again {
                                nav.double_click(self.pointer, self.mods);
                                self.last_click = None;
                            } else {
                                self.last_click = Some((Instant::now(), self.pointer));
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                nav.scroll(lines, self.pointer, self.mods);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let pressed = event.state == ElementState::Pressed;
                let arrow = match code {
                    KeyCode::ArrowUp => Some(Arrow::Up),
                    KeyCode::ArrowDown => Some(Arrow::Down),
                    KeyCode::ArrowLeft => Some(Arrow::Left),
                    KeyCode::ArrowRight => Some(Arrow::Right),
                    _ => None,
                };
                if let Some(a) = arrow {
                    if !pressed {
                        nav.arrow_up(a);
                    } else if !event.repeat {
                        nav.arrow_down(a, self.mods);
                    }
                } else if pressed && !event.repeat {
                    match code {
                        KeyCode::Escape => nav.stop(),
                        KeyCode::Digit0 if self.mods.ctrl => nav.whole_body(),
                        KeyCode::KeyN if self.mods.ctrl && self.mods.alt => nav.straighten(),
                        KeyCode::KeyQ if self.mods.ctrl => event_loop.exit(),
                        KeyCode::KeyR if !self.mods.ctrl => {
                            gfx.scene.roads_on = !gfx.scene.roads_on;
                            gfx.renderer.roads_visible = gfx.scene.roads_on;
                        }
                        KeyCode::KeyF if !self.mods.ctrl => gfx.pacer.toggle(),
                        KeyCode::KeyB if !self.mods.ctrl => {
                            gfx.scene.buildings_on = !gfx.scene.buildings_on;
                        }
                        KeyCode::KeyL if !self.mods.ctrl => {
                            gfx.scene.labels_on = !gfx.scene.labels_on;
                        }
                        _ => {}
                    }
                }
            }
            _ => return,
        }
        gfx.redraw_wanted = true;
    }
}

fn main() -> Result<()> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        println!("{USAGE}");
        return Ok(());
    };
    if args.cache_status {
        cache_status(&args);
        return Ok(());
    }
    if args.serve_cache {
        return serve_cache(&args);
    }
    if let Some(path) = args.screenshot.clone() {
        return screenshot(&args, &path);
    }
    let event_loop = EventLoop::new().context("could not connect to the display")?;
    let mut app = App {
        args,
        gfx: None,
        error: None,
        mods: Mods::default(),
        pointer: DVec2::ZERO,
        pressed_at: None,
        last_click: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[&str]) -> Result<Option<Args>> {
        parse_args(s.iter().map(|x| x.to_string()))
    }

    #[test]
    fn options_parse() {
        let a = parse(&["--lat", "47", "--lon", "11", "--tilt", "60", "--size", "800x600"]).unwrap().unwrap();
        assert!((a.cam.lat.to_degrees() - 47.0).abs() < 1e-9);
        assert_eq!(a.size, (800, 600));
    }

    #[test]
    fn bad_options_are_reported() {
        assert!(parse(&["--nope", "1"]).is_err());
        assert!(parse(&["--lat"]).is_err());
        assert!(parse(&["--lat", "abc"]).is_err());
        assert!(parse(&["--lat", "NaN"]).is_err());
        assert!(parse(&["--size", "0x10"]).is_err());
        assert!(parse(&["--help"]).unwrap().is_none());
    }

    #[test]
    fn distances_read_well() {
        assert_eq!(format_distance(250.4), "250 m");
        assert_eq!(format_distance(2500.0), "2.5 km");
        assert_eq!(format_distance(120_000.0), "120 km");
    }

    #[test]
    fn both_credits_are_present() {
        assert!(CREDITS.contains("EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH"));
        assert!(CREDITS.contains("Copernicus Sentinel data 2024"));
        assert!(CREDITS.contains("Mapterhorn"));
        assert!(CREDITS.contains("OpenMapTiles") && CREDITS.contains("OpenStreetMap"));
    }

    #[test]
    fn building_tiles_follow_the_drawn_tiles() {
        let fine = TileKey::new(14, 300, 400);
        let finer = [TileKey::new(16, 1200, 1600), TileKey::new(15, 601, 801)];
        // Finer tiles share the mesh of the tile they lie in.
        assert_eq!(building_tiles(&[fine, finer[0], finer[1]]), [fine]);
        // One level coarser uses its four children; further out there are none.
        let coarse = TileKey::new(13, 100, 100);
        let mut children = coarse.children();
        children.sort_unstable();
        assert_eq!(building_tiles(&[coarse]), children);
        assert!(building_tiles(&[TileKey::new(12, 50, 50)]).is_empty());
    }

    #[test]
    fn cache_options() {
        let a = parse(&[
            "--cache-limit", "50", "--offline", "--share-cache", "--cache-port", "9000",
            "--cache-server", "nas.local:47813", "--cache-server", "10.0.0.2:47813",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(a.cache_limit_gib, 50);
        assert!(a.offline && a.share_cache && !a.serve_cache);
        assert_eq!(a.cache_port, 9000);
        assert_eq!(a.cache_servers, ["nas.local:47813", "10.0.0.2:47813"]);
        for bad in [
            ["--cache-limit", "0"],
            ["--cache-limit", "x"],
            ["--cache-port", "0"],
            ["--cache-server", "nas"],
            ["--cache-server", ":1"],
            ["--cache-server", "a/b:1"],
            ["--cache-server", "a:99999"],
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
        // Sharing is opt-in: nothing opens a port or looks for others by default.
        let d = parse(&[]).unwrap().unwrap();
        assert!(!d.serve_cache && !d.share_cache && !d.offline && d.cache_servers.is_empty());
    }

    #[test]
    fn credits_follow_what_is_on_screen() {
        assert_eq!(credits_for([""].into_iter()), CREDITS);
        let c = credits_for(["© swisstopo", "", "Grundkarte: basemap.at", "© swisstopo"].into_iter());
        assert!(c.starts_with(CREDITS));
        assert!(c.ends_with("Aerial images: Grundkarte: basemap.at, © swisstopo"), "{c}");
    }

    #[test]
    fn a_server_timeout_sets_the_retry_time_directly() {
        let mut retries = HashMap::new();
        let k = TileKey::new(3, 1, 1);
        let start = Instant::now();
        next_retry(&mut retries, k, None);
        assert!(retries[&k].0 < start + Duration::from_secs(5), "a short first delay");
        next_retry(&mut retries, k, Some(SystemTime::now() + Duration::from_secs(900)));
        assert!(retries[&k].0 > start + Duration::from_secs(890), "the server's timeout");
    }

    #[test]
    fn traffic_shows_on_screen_with_timed_out_servers() {
        let dir = fetcher::tests::temp_cache("hud");
        let f = Fetcher::new(dir, Peers::default(), true);
        let now = SystemTime::now();
        assert!(network_status(&f, now).is_empty());
        f.servers.failed(
            "tiles.example.org",
            servers::Failure::TooManyRequests { retry_after: None },
            now,
        );
        let lines = network_status(&f, now);
        assert_eq!(lines, ["timed out: tiles.example.org 15 min"]);
    }

    #[test]
    fn sizes_and_times_read_well() {
        assert_eq!(format_bytes(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(format_bytes(GIB * 2), "2.0 GB");
        let now = SystemTime::now();
        assert_eq!(minutes_until(now + Duration::from_secs(61), now), 2);
        assert_eq!(minutes_until(now, now + Duration::from_secs(5)), 0);
    }

    #[test]
    fn frame_options() {
        assert_eq!(parse(&[]).unwrap().unwrap().limit, FrameLimit::Fps120);
        assert_eq!(parse(&["--fps", "90"]).unwrap().unwrap().limit, FrameLimit::Fps90);
        assert!(parse(&["--fps", "60"]).is_err());
        assert_eq!(parse(&["--benchmark", "5"]).unwrap().unwrap().benchmark, Some(5.0));
        assert!(parse(&["--benchmark", "0"]).is_err());
    }

    #[test]
    fn map_options() {
        let a = parse(&["--lang", "EN", "--no-roads", "--lat", "1"]).unwrap().unwrap();
        assert_eq!(a.lang, "en");
        assert!(!a.roads && a.labels);
        assert!(parse(&["--lang", "../x"]).is_err());
        let b = parse(&["--no-labels"]).unwrap().unwrap();
        assert!(b.roads && !b.labels);
    }

    #[test]
    fn same_place_keeps_its_identity_across_zoom_levels() {
        let label = |lat, text: &str| vector::Label {
            text: text.into(),
            style: vector::LabelStyle::City,
            priority: 0,
            lat,
            lon: 0.2,
            along: None,
        };
        assert_eq!(label_id(&label(0.8001, "A")), label_id(&label(0.80011, "A")));
        assert_ne!(label_id(&label(0.8001, "A")), label_id(&label(0.8001, "B")));
    }

    #[test]
    fn the_display_tells_finished_from_still_loading() {
        assert_eq!(loading_status(0, 0), "all loaded");
        assert_eq!(loading_status(5, 0), "loading 5");
        assert_eq!(loading_status(3, 3), "3 waiting to retry");
        assert_eq!(loading_status(5, 2), "loading 3, 2 waiting to retry");
    }
}
