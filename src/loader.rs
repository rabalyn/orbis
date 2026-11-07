//! Background tile loading: download or read from cache, decode, and build meshes.

use crate::buildings::{self, BuildingMesh};
use crate::cache::DiskCache;
use crate::geo::{geodetic_to_ecef, TileKey, A};
use crate::overlay::{self, OVERLAY_SIZE};
use crate::vector::{self, Label, VectorTile};
use bytemuck::{Pod, Zeroable};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const GRID: usize = 32;
pub const IMAGERY_MAX_Z: u8 = 14;
pub const TERRAIN_MAX_Z: u8 = 14;
pub const TILE_MAX_Z: u8 = 16;
/// OpenMapTiles data ends here; closer tiles use a part of a tile at this zoom.
pub const VECTOR_MAX_Z: u8 = 14;
const WORKERS: usize = 8;
/// Finished results waiting to be collected. Workers pause beyond this, which limits how
/// much memory decoded tiles can take when they are produced faster than they are uploaded.
const RESULT_QUEUE: usize = 32;
const DEM_CACHE: usize = 64;
const VECTOR_CACHE: usize = 64;
/// The most requests sent to the map data service at once.
const VECTOR_CONCURRENCY: usize = 4;
const USER_AGENT: &str = "orbis-globe/0.1";
const TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";
const WEEK_SECONDS: u64 = 7 * 24 * 3600;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// Position in the tile's imagery, which may be a part of a coarser image.
    pub uv: [f32; 2],
    /// Position within this tile, for the overlay drawn just for it.
    pub st: [f32; 2],
}

pub struct TileData {
    pub key: TileKey,
    pub center: DVec3,
    pub vertices: Vec<Vertex>,
    /// Terrain heights on the (GRID + 1)² mesh grid.
    pub heights: Vec<f32>,
    /// RGBA8 mip chain, largest first.
    pub mips: Vec<(u32, u32, Vec<u8>)>,
}

pub struct OverlayData {
    pub key: TileKey,
    /// The vector tile the data came from.
    pub source: TileKey,
    pub labels: Arc<Vec<Label>>,
    /// Premultiplied RGBA8 mip chain, largest first. `None` when there are no lines to draw.
    pub mips: Option<Vec<(u32, u32, Vec<u8>)>>,
}

pub enum Outcome {
    Ready(Box<TileData>),
    /// The server has no such tile, so asking again is pointless.
    Missing(TileKey),
    /// Something went wrong that may pass.
    Retry(TileKey),
    Overlay(Box<OverlayData>),
    /// Map data could not be fetched at the moment; the tile stays usable without it.
    OverlayRetry(TileKey),
    /// A tile's building mesh, or `None` when it has no buildings.
    Buildings(TileKey, Option<Box<BuildingMesh>>),
    BuildingsRetry(TileKey),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Job {
    Tile(TileKey),
    Overlay(TileKey),
    Buildings(TileKey),
}

/// Limits how many threads do something at once.
struct Gate {
    free: Mutex<usize>,
    released: Condvar,
}

impl Gate {
    fn new(n: usize) -> Self {
        Self { free: Mutex::new(n), released: Condvar::new() }
    }

    fn enter(&self) -> GatePass<'_> {
        let mut free = self.free.lock().unwrap();
        while *free == 0 {
            free = self.released.wait(free).unwrap();
        }
        *free -= 1;
        GatePass(self)
    }
}

struct GatePass<'a>(&'a Gate);

impl Drop for GatePass<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap() += 1;
        self.0.released.notify_one();
    }
}

/// The tile URL template from a TileJSON document.
fn tile_template(json: &str) -> Option<String> {
    let after = &json[json.find("\"tiles\"")?..];
    let start = after.find("\"http")? + 1;
    let rest = &after[start..];
    let url = rest[..rest.find('"')?].replace("\\/", "/");
    let usable = url.starts_with("https://") && ["{z}", "{x}", "{y}"].iter().all(|p| url.contains(p));
    usable.then_some(url)
}

#[derive(Debug, PartialEq)]
pub enum FetchError {
    Missing,
    Transient,
}

/// Downloads through the disk cache, and pauses a source that reports trouble.
pub struct Fetcher {
    cache: DiskCache,
    agent: ureq::Agent,
    holds: Mutex<HashMap<&'static str, (Instant, u64)>>,
    map_template: Mutex<Option<String>>,
    map_gate: Gate,
}

impl Fetcher {
    pub fn new(cache: DiskCache) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(USER_AGENT)
            .build();
        Self {
            cache,
            agent: config.into(),
            holds: Mutex::new(HashMap::new()),
            map_template: Mutex::new(None),
            map_gate: Gate::new(VECTOR_CONCURRENCY),
        }
    }

    /// The map service publishes data under a versioned URL. It is looked up at most weekly;
    /// if that fails, an earlier answer still gets tiles while they remain online.
    fn map_url_template(&self) -> Result<String, FetchError> {
        let mut known = self.map_template.lock().unwrap();
        if let Some(t) = &*known {
            return Ok(t.clone());
        }
        let name = |week: u64| format!("openfreemap/tilejson-{week}.json");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let week = now / WEEK_SECONDS;
        let fresh = self.get("OpenFreeMap", &name(week), TILEJSON_URL);
        let template = fresh
            .ok()
            .and_then(|bytes| tile_template(&String::from_utf8_lossy(&bytes)))
            .or_else(|| {
                (1..=8).find_map(|back| {
                    let bytes = self.cache.get(&name(week.saturating_sub(back)))?;
                    tile_template(&String::from_utf8_lossy(&bytes))
                })
            })
            .ok_or(FetchError::Transient)?;
        *known = Some(template.clone());
        Ok(template)
    }

    fn vector(&self, k: TileKey) -> Result<Vec<u8>, FetchError> {
        let name = format!("openfreemap/{}/{}/{}.pbf", k.z, k.x, k.y);
        if let Some(data) = self.cache.get(&name) {
            return Ok(data);
        }
        let url = self
            .map_url_template()?
            .replace("{z}", &k.z.to_string())
            .replace("{x}", &k.x.to_string())
            .replace("{y}", &k.y.to_string());
        let _pass = self.map_gate.enter();
        self.get("OpenFreeMap", &name, &url)
    }

    fn wait_for_hold(&self, source: &'static str) {
        let until = self.holds.lock().unwrap().get(source).map(|h| h.0);
        if let Some(remaining) = until.and_then(|u| u.checked_duration_since(Instant::now())) {
            std::thread::sleep(remaining);
        }
    }

    fn hold(&self, source: &'static str) {
        let mut holds = self.holds.lock().unwrap();
        let seconds = holds.get(source).map_or(2, |h| (h.1 * 2).min(60));
        holds.insert(source, (Instant::now() + Duration::from_secs(seconds), seconds));
    }

    fn get(&self, source: &'static str, name: &str, url: &str) -> Result<Vec<u8>, FetchError> {
        if let Some(data) = self.cache.get(name) {
            return Ok(data);
        }
        self.wait_for_hold(source);
        match self.agent.get(url).call() {
            Ok(mut response) => {
                let data = response
                    .body_mut()
                    .read_to_vec()
                    .map_err(|_| FetchError::Transient)?;
                self.holds.lock().unwrap().remove(source);
                if let Err(e) = self.cache.put(name, &data) {
                    eprintln!("cache: could not store {name}: {e}");
                }
                Ok(data)
            }
            Err(ureq::Error::StatusCode(404 | 410)) => Err(FetchError::Missing),
            Err(e) => {
                eprintln!("{source}: {e}");
                self.hold(source);
                Err(FetchError::Transient)
            }
        }
    }

    fn imagery(&self, k: TileKey) -> Result<Vec<u8>, FetchError> {
        let name = format!("eox-s2cloudless-2024/{}/{}/{}.jpg", k.z, k.y, k.x);
        let url = format!(
            "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2024_3857/default/g/{}/{}/{}.jpg",
            k.z, k.y, k.x
        );
        self.get("EOX", &name, &url)
    }

    fn terrain(&self, k: TileKey) -> Result<Vec<u8>, FetchError> {
        let name = format!("mapterhorn/{}/{}/{}.webp", k.z, k.x, k.y);
        let url = format!("https://tiles.mapterhorn.com/{}/{}/{}.webp", k.z, k.x, k.y);
        self.get("Mapterhorn", &name, &url)
    }
}

/// A square elevation grid.
pub struct Dem {
    size: usize,
    data: Vec<f32>,
}

impl Dem {
    /// Decodes Terrarium encoding: height = R·256 + G + B/256 − 32768.
    pub fn from_terrarium(bytes: &[u8]) -> Option<Self> {
        let img = image::load_from_memory(bytes).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        if w != h || w == 0 {
            return None;
        }
        let data = img
            .pixels()
            .map(|p| p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0 - 32768.0)
            .collect();
        Some(Self { size: w as usize, data })
    }

    pub fn flat() -> Self {
        Self { size: 1, data: vec![0.0] }
    }

    /// Bilinear height at a position within the tile, with `u` and `v` in 0..1.
    pub fn sample(&self, u: f64, v: f64) -> f32 {
        let n = self.size as f64;
        let x = (u * n - 0.5).clamp(0.0, n - 1.0);
        let y = (v * n - 0.5).clamp(0.0, n - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.size - 1), (y0 + 1).min(self.size - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |x: usize, y: usize| self.data[y * self.size + x];
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Which imagery tile covers `k`, and the square of it `k` occupies.
fn imagery_source(k: TileKey) -> (TileKey, (f64, f64, f64)) {
    let a = k.ancestor(k.z.min(IMAGERY_MAX_Z));
    (a, k.rect_in(a))
}

/// A terrain tile holds 512 samples, so two zoom levels up still gives a 32-quad mesh
/// plenty of detail with far fewer downloads.
fn terrain_source(k: TileKey) -> (TileKey, (f64, f64, f64)) {
    let a = k.ancestor(k.z.saturating_sub(2).min(TERRAIN_MAX_Z));
    (a, k.rect_in(a))
}

/// Builds the surface of a tile: a grid plus a skirt that hides cracks between tiles.
pub fn build_mesh(
    k: TileKey,
    dem: &Dem,
    dem_rect: (f64, f64, f64),
    uv_rect: (f64, f64, f64),
    exaggeration: f32,
) -> (DVec3, Vec<Vertex>, Vec<f32>) {
    let (clat, clon) = k.lat_lon(0.5, 0.5);
    let center = geodetic_to_ecef(clat, clon, 0.0);
    let side = GRID + 1;
    let mut vertices = Vec::with_capacity(side * side + 4 * GRID);
    let mut heights = Vec::with_capacity(side * side);
    let width = std::f64::consts::TAU * A / (1u64 << k.z) as f64;
    let skirt = (width * 0.02).clamp(5.0, 50_000.0);
    let vertex = |s: f64, t: f64, h: f64| -> Vertex {
        let (lat, lon) = k.lat_lon(s, t);
        let p = geodetic_to_ecef(lat, lon, h) - center;
        Vertex {
            pos: p.as_vec3().to_array(),
            uv: [
                (uv_rect.0 + s * uv_rect.2) as f32,
                (uv_rect.1 + t * uv_rect.2) as f32,
            ],
            st: [s as f32, t as f32],
        }
    };
    for j in 0..side {
        for i in 0..side {
            let (s, t) = (i as f64 / GRID as f64, j as f64 / GRID as f64);
            let h = dem.sample(dem_rect.0 + s * dem_rect.2, dem_rect.1 + t * dem_rect.2) * exaggeration;
            heights.push(h);
            vertices.push(vertex(s, t, h as f64));
        }
    }
    for (i, j) in border() {
        let (s, t) = (i as f64 / GRID as f64, j as f64 / GRID as f64);
        vertices.push(vertex(s, t, heights[j * side + i] as f64 - skirt));
    }
    (center, vertices, heights)
}

/// Grid coordinates around the edge of a tile, in order.
fn border() -> impl Iterator<Item = (usize, usize)> {
    let top = (0..GRID).map(|i| (i, 0));
    let right = (0..GRID).map(|j| (GRID, j));
    let bottom = (0..GRID).map(|i| (GRID - i, GRID));
    let left = (0..GRID).map(|j| (0, GRID - j));
    top.chain(right).chain(bottom).chain(left)
}

/// The index list shared by all tile meshes.
pub fn mesh_indices() -> Vec<u32> {
    let side = GRID + 1;
    let at = |i: usize, j: usize| (j * side + i) as u32;
    let mut out = Vec::with_capacity(GRID * GRID * 6 + 4 * GRID * 6);
    for j in 0..GRID {
        for i in 0..GRID {
            out.extend([at(i, j), at(i, j + 1), at(i + 1, j)]);
            out.extend([at(i + 1, j), at(i, j + 1), at(i + 1, j + 1)]);
        }
    }
    let ring: Vec<_> = border().collect();
    let first_skirt = (side * side) as u32;
    let count = ring.len();
    for n in 0..count {
        let m = (n + 1) % count;
        let (a, b) = (at(ring[n].0, ring[n].1), at(ring[m].0, ring[m].1));
        let (c, d) = (first_skirt + n as u32, first_skirt + m as u32);
        out.extend([a, c, b, b, c, d]);
    }
    out
}

/// A box-filtered mip chain for a tightly packed RGBA8 image.
pub fn build_mips(mut w: u32, mut h: u32, rgba: Vec<u8>) -> Vec<(u32, u32, Vec<u8>)> {
    let mut mips = vec![(w, h, rgba)];
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = &mips.last().unwrap().2;
        let mut dst = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let (x0, y0) = (x * w / nw, y * h / nh);
                let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                for c in 0..4u32 {
                    let at = |px: u32, py: u32| src[((py * w + px) * 4 + c) as usize] as u32;
                    let sum = at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1);
                    dst[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        mips.push((nw, nh, dst));
        (w, h) = (nw, nh);
    }
    mips
}

#[derive(Default)]
struct Queue {
    /// Most urgent last.
    waiting: Vec<Job>,
    in_flight: HashSet<Job>,
}

struct Shared {
    fetcher: Fetcher,
    exaggeration: f32,
    names: Vec<String>,
    queue: Mutex<Queue>,
    wake: Condvar,
    dems: Mutex<HashMap<TileKey, Arc<Dem>>>,
    vectors: Mutex<HashMap<TileKey, Arc<VectorTile>>>,
}

impl Shared {
    fn vector(&self, k: TileKey) -> Result<Arc<VectorTile>, FetchError> {
        if let Some(v) = self.vectors.lock().unwrap().get(&k) {
            return Ok(v.clone());
        }
        let tile = match self.fetcher.vector(k) {
            // Where no data exists, such as on the open ocean, there is simply nothing to draw.
            Err(FetchError::Missing) => VectorTile::default(),
            Err(e) => return Err(e),
            Ok(bytes) => vector::decode(&bytes, k, &self.names).unwrap_or_else(|| {
                eprintln!("map data: tile {}/{}/{} is unreadable", k.z, k.x, k.y);
                VectorTile::default()
            }),
        };
        let tile = Arc::new(tile);
        let mut vectors = self.vectors.lock().unwrap();
        if vectors.len() >= VECTOR_CACHE {
            let any = *vectors.keys().next().unwrap();
            vectors.remove(&any);
        }
        vectors.insert(k, tile.clone());
        Ok(tile)
    }

    fn load_buildings(&self, k: TileKey) -> Outcome {
        let Ok(tile) = self.vector(k) else {
            return Outcome::BuildingsRetry(k);
        };
        if tile.buildings.is_empty() {
            return Outcome::Buildings(k, None);
        }
        let (dem_key, rect) = terrain_source(k);
        let Ok(dem) = self.dem(dem_key) else {
            return Outcome::BuildingsRetry(k);
        };
        let ground = |x: f64, y: f64| dem.sample(rect.0 + x * rect.2, rect.1 + y * rect.2) * self.exaggeration;
        let mesh = buildings::build_mesh(k, &tile.buildings, &ground);
        Outcome::Buildings(k, mesh.map(Box::new))
    }

    fn load_overlay(&self, k: TileKey) -> Outcome {
        let source = k.ancestor(k.z.min(VECTOR_MAX_Z));
        let Ok(tile) = self.vector(source) else {
            return Outcome::OverlayRetry(k);
        };
        let mips = overlay::render(&tile, k, source)
            .map(|rgba| build_mips(OVERLAY_SIZE, OVERLAY_SIZE, rgba));
        Outcome::Overlay(Box::new(OverlayData { key: k, source, labels: tile.labels.clone(), mips }))
    }

    fn dem(&self, k: TileKey) -> Result<Arc<Dem>, FetchError> {
        if let Some(d) = self.dems.lock().unwrap().get(&k) {
            return Ok(d.clone());
        }
        let dem = match self.fetcher.terrain(k) {
            // Where no terrain exists, the surface is simply flat.
            Err(FetchError::Missing) => Dem::flat(),
            Err(e) => return Err(e),
            Ok(bytes) => Dem::from_terrarium(&bytes).ok_or(FetchError::Transient)?,
        };
        let dem = Arc::new(dem);
        let mut dems = self.dems.lock().unwrap();
        if dems.len() >= DEM_CACHE {
            let any = *dems.keys().next().unwrap();
            dems.remove(&any);
        }
        dems.insert(k, dem.clone());
        Ok(dem)
    }

    fn load(&self, k: TileKey) -> Outcome {
        let (img_key, uv_rect) = imagery_source(k);
        let jpeg = match self.fetcher.imagery(img_key) {
            Ok(b) => b,
            Err(FetchError::Missing) => return Outcome::Missing(k),
            Err(FetchError::Transient) => return Outcome::Retry(k),
        };
        let Ok(img) = image::load_from_memory(&jpeg) else {
            return Outcome::Missing(k);
        };
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        let (dem_key, dem_rect) = terrain_source(k);
        let Ok(dem) = self.dem(dem_key) else {
            return Outcome::Retry(k);
        };
        let (center, vertices, heights) = build_mesh(k, &dem, dem_rect, uv_rect, self.exaggeration);
        Outcome::Ready(Box::new(TileData {
            key: k,
            center,
            vertices,
            heights,
            mips: build_mips(w, h, img.into_raw()),
        }))
    }
}

pub struct Loader {
    shared: Arc<Shared>,
    results: Receiver<(Job, Outcome)>,
}

impl Loader {
    pub fn new(cache: DiskCache, exaggeration: f32, names: Vec<String>) -> Self {
        let shared = Arc::new(Shared {
            fetcher: Fetcher::new(cache),
            exaggeration,
            names,
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            dems: Mutex::new(HashMap::new()),
            vectors: Mutex::new(HashMap::new()),
        });
        let (tx, results) = sync_channel(RESULT_QUEUE);
        for _ in 0..WORKERS {
            let (shared, tx) = (shared.clone(), tx.clone());
            std::thread::spawn(move || worker(&shared, &tx));
        }
        Self { shared, results }
    }

    /// Replaces the queue, so tiles the view no longer needs are not fetched. Imagery and
    /// terrain come before map data, which can only appear on a tile that is already there,
    /// and buildings come last.
    pub fn set_wanted(&self, tiles: &[TileKey], overlays: &[TileKey], buildings: &[TileKey]) {
        let jobs = buildings
            .iter()
            .rev()
            .map(|k| Job::Buildings(*k))
            .chain(overlays.iter().rev().map(|k| Job::Overlay(*k)))
            .chain(tiles.iter().rev().map(|k| Job::Tile(*k)));
        let mut q = self.shared.queue.lock().unwrap();
        let in_flight = std::mem::take(&mut q.in_flight);
        q.waiting.clear();
        q.waiting.extend(jobs.filter(|j| !in_flight.contains(j)));
        q.in_flight = in_flight;
        drop(q);
        self.shared.wake.notify_all();
    }

    /// Collects a finished job. A job counts as in progress until this point: the view still
    /// wants it, so clearing it any earlier would have it built again while the first result
    /// sits in the queue.
    pub fn try_result(&self) -> Option<Outcome> {
        let (job, outcome) = self.results.try_recv().ok()?;
        self.shared.queue.lock().unwrap().in_flight.remove(&job);
        Some(outcome)
    }

    pub fn is_idle(&self) -> bool {
        let q = self.shared.queue.lock().unwrap();
        q.waiting.is_empty() && q.in_flight.is_empty()
    }
}

fn worker(shared: &Shared, tx: &SyncSender<(Job, Outcome)>) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                if let Some(j) = q.waiting.pop() {
                    q.in_flight.insert(j);
                    break j;
                }
                q = shared.wake.wait(q).unwrap();
            }
        };
        let outcome = match job {
            Job::Tile(k) => shared.load(k),
            Job::Overlay(k) => shared.load_overlay(k),
            Job::Buildings(k) => shared.load_buildings(k),
        };
        if tx.send((job, outcome)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn world(center: DVec3, v: &Vertex) -> DVec3 {
        center + Vec3::from_array(v.pos).as_dvec3()
    }

    #[test]
    fn neighbouring_tiles_meet_exactly() {
        let dem = Dem::flat();
        let full = (0.0, 0.0, 1.0);
        let (a, b) = (TileKey::new(5, 16, 10), TileKey::new(5, 17, 10));
        let (ca, va, _) = build_mesh(a, &dem, full, full, 1.0);
        let (cb, vb, _) = build_mesh(b, &dem, full, full, 1.0);
        for j in 0..=GRID {
            let right_edge = world(ca, &va[j * (GRID + 1) + GRID]);
            let left_edge = world(cb, &vb[j * (GRID + 1)]);
            assert!((right_edge - left_edge).length() < 2.0);
        }
    }

    #[test]
    fn skirt_vertices_hang_below_the_surface() {
        let dem = Dem::flat();
        let full = (0.0, 0.0, 1.0);
        let k = TileKey::new(6, 33, 20);
        let (c, v, h) = build_mesh(k, &dem, full, full, 1.0);
        assert_eq!(h.len(), (GRID + 1) * (GRID + 1));
        assert_eq!(v.len(), (GRID + 1) * (GRID + 1) + 4 * GRID);
        let edge = world(c, &v[0]);
        let skirt = world(c, &v[(GRID + 1) * (GRID + 1)]);
        assert!(skirt.length() < edge.length());
        let max = *mesh_indices().iter().max().unwrap() as usize;
        assert_eq!(max, v.len() - 1);
    }

    #[test]
    fn terrain_raises_the_surface() {
        let dem = Dem { size: 1, data: vec![1000.0] };
        let full = (0.0, 0.0, 1.0);
        let k = TileKey::new(6, 33, 20);
        let (c0, flat, _) = build_mesh(k, &Dem::flat(), full, full, 1.0);
        let (c1, raised, h) = build_mesh(k, &dem, full, full, 2.0);
        assert_eq!(h[0], 2000.0);
        let gained = world(c1, &raised[0]).length() - world(c0, &flat[0]).length();
        assert!((gained - 2000.0).abs() < 2.0, "{gained}");
    }

    #[test]
    fn sources_above_their_limits_use_a_part_of_an_ancestor() {
        let k = TileKey::new(16, 34816, 22976);
        let (img, rect) = imagery_source(k);
        assert_eq!(img, k.ancestor(14));
        assert_eq!(rect.2, 0.25);
        let (dem, rect) = terrain_source(k);
        assert_eq!(dem, k.ancestor(14));
        assert_eq!(rect.2, 0.25);
        assert_eq!(terrain_source(TileKey::new(1, 1, 0)).0, TileKey::new(0, 0, 0));
    }

    #[test]
    fn dem_sampling_interpolates() {
        let dem = Dem { size: 2, data: vec![0.0, 100.0, 0.0, 100.0] };
        assert_eq!(dem.sample(0.5, 0.5), 50.0);
        assert_eq!(dem.sample(0.0, 0.0), 0.0);
        assert_eq!(dem.sample(1.0, 1.0), 100.0);
    }

    #[test]
    fn terrarium_decoding() {
        // Height 1000.5: 33768.5 = 131·256 + 232 + 128/256.
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([131, 232, 128, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let dem = Dem::from_terrarium(bytes.get_ref()).unwrap();
        assert_eq!(dem.sample(0.5, 0.5), 1000.5);
    }

    #[test]
    fn mip_chain_ends_at_one_pixel_and_averages() {
        let rgba = [0u8, 0, 0, 255, 100, 100, 100, 255, 100, 100, 100, 255, 200, 200, 200, 255];
        let mips = build_mips(2, 2, rgba.to_vec());
        assert_eq!(mips.len(), 2);
        assert_eq!((mips[1].0, mips[1].1), (1, 1));
        assert_eq!(&mips[1].2[..], &[100, 100, 100, 255]);
        assert_eq!(build_mips(256, 256, vec![0; 256 * 256 * 4]).len(), 9);
    }

    #[test]
    fn cached_data_needs_no_network() {
        let root = std::env::temp_dir().join(format!("orbis-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cache = DiskCache::new(root.clone());
        cache.put("x/1.bin", b"data").unwrap();
        let f = Fetcher::new(cache);
        let got = f.get("test", "x/1.bin", "http://invalid.invalid/none");
        assert_eq!(got.unwrap(), b"data");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A loader whose imagery is cached but unreadable, so jobs finish fast with no network.
    fn offline_loader(tag: &str, keys: &[TileKey]) -> (Loader, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("orbis-loader-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cache = DiskCache::new(root.clone());
        for k in keys {
            let name = format!("eox-s2cloudless-2024/{}/{}/{}.jpg", k.z, k.y, k.x);
            cache.put(&name, b"not an image").unwrap();
        }
        (Loader::new(cache, 1.0, vec!["name".into()]), root)
    }

    fn collect_for(loader: &Loader, wanted: &[TileKey], millis: u64) -> usize {
        let mut count = 0;
        let end = Instant::now() + Duration::from_millis(millis);
        while Instant::now() < end {
            loader.set_wanted(wanted, &[], &[]);
            std::thread::sleep(Duration::from_millis(5));
            while loader.try_result().is_some() {
                count += 1;
            }
            // A view that still wants a tile keeps asking for it, as the app does.
            if count > 0 {
                break;
            }
        }
        count
    }

    #[test]
    fn a_finished_job_is_not_redone_before_its_result_is_collected() {
        let k = TileKey::new(3, 2, 1);
        let (loader, root) = offline_loader("dedupe", &[k]);
        // The view keeps asking while nothing collects the result.
        for _ in 0..40 {
            loader.set_wanted(&[k], &[], &[]);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!loader.is_idle(), "an uncollected result still counts as work");
        loader.set_wanted(&[], &[], &[]);
        std::thread::sleep(Duration::from_millis(20));
        let mut results = 0;
        while loader.try_result().is_some() {
            results += 1;
        }
        assert_eq!(results, 1, "asked 40 times, built once");
        assert!(loader.is_idle());
        // Once collected, it can be requested again.
        assert_eq!(collect_for(&loader, &[k], 1000), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workers_pause_when_results_are_not_collected() {
        let keys: Vec<TileKey> = (0..512).map(|x| TileKey::new(9, x, 3)).collect();
        let (loader, root) = offline_loader("bound", &keys);
        loader.set_wanted(&keys, &[], &[]);
        std::thread::sleep(Duration::from_millis(300));
        let q = loader.shared.queue.lock().unwrap();
        let started = q.in_flight.len();
        let left = q.waiting.len();
        drop(q);
        assert!(started <= RESULT_QUEUE + WORKERS, "{started} jobs ran with nothing collected");
        assert_eq!(started + left, keys.len());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tile_template_comes_from_tilejson() {
        let json = r#"{"tilejson":"3.0.0","tiles":["https://t.example/v1/{z}/{x}/{y}.pbf"],"x":"http://no"}"#;
        assert_eq!(tile_template(json).unwrap(), "https://t.example/v1/{z}/{x}/{y}.pbf");
        let escaped = r#"{"tiles":["https:\/\/t.example\/{z}\/{x}\/{y}.pbf"]}"#;
        assert_eq!(tile_template(escaped).unwrap(), "https://t.example/{z}/{x}/{y}.pbf");
        assert_eq!(tile_template(r#"{"tiles":["http://t.example/{z}/{x}/{y}"]}"#), None);
        assert_eq!(tile_template(r#"{"tiles":["https://t.example/fixed.pbf"]}"#), None);
        assert_eq!(tile_template("not json"), None);
    }

    #[test]
    fn overlay_coordinates_stay_within_the_tile_when_imagery_is_borrowed() {
        let k = TileKey::new(16, 34817, 22977);
        let (_, uv_rect) = imagery_source(k);
        let full = (0.0, 0.0, 1.0);
        let (_, v, _) = build_mesh(k, &Dem::flat(), full, uv_rect, 1.0);
        assert_eq!(v[0].st, [0.0, 0.0]);
        let last_grid = (GRID + 1) * (GRID + 1) - 1;
        assert_eq!(v[last_grid].st, [1.0, 1.0]);
        // The imagery coordinates cover just a quarter of the image.
        assert_eq!(v[0].uv, [0.25, 0.25]);
        assert_eq!(v[last_grid].uv, [0.5, 0.5]);
    }

    #[test]
    fn gate_limits_how_many_enter() {
        let gate = Arc::new(Gate::new(2));
        let inside = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let most = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (gate, inside, most) = (gate.clone(), inside.clone(), most.clone());
                std::thread::spawn(move || {
                    use std::sync::atomic::Ordering::SeqCst;
                    let _pass = gate.enter();
                    let now = inside.fetch_add(1, SeqCst) + 1;
                    most.fetch_max(now, SeqCst);
                    std::thread::sleep(Duration::from_millis(10));
                    inside.fetch_sub(1, SeqCst);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(most.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn repeated_trouble_lengthens_the_pause() {
        let f = Fetcher::new(DiskCache::new(std::env::temp_dir().join("orbis-unused")));
        f.hold("s");
        assert_eq!(f.holds.lock().unwrap()["s"].1, 2);
        f.hold("s");
        assert_eq!(f.holds.lock().unwrap()["s"].1, 4);
        for _ in 0..10 {
            f.hold("s");
        }
        assert_eq!(f.holds.lock().unwrap()["s"].1, 60);
    }
}
