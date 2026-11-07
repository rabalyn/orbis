//! Background tile loading: download or read from cache, decode, and build meshes.

use crate::buildings::{self, BuildingMesh};
use crate::fetcher::{FetchError, Fetcher, Wait};
use crate::geo::{geodetic_to_ecef, radius, TileKey};
use crate::import::Imported;
use crate::layers::LineKinds;
use crate::overlay::{self, OVERLAY_SIZE};
use crate::sources;
use crate::{blend, service_lib};
use crate::vector::{self, Label, VectorTile};
use bytemuck::{Pod, Zeroable};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

pub const GRID: usize = 32;
pub const TERRAIN_MAX_Z: u8 = 14;
pub const TILE_MAX_Z: u8 = sources::ABSOLUTE_MAX_Z;
/// Channels may differ by this much for an image to count as blank.
const BLANK_RANGE: u8 = 3;
/// OpenMapTiles data ends here; closer tiles use a part of a tile at this zoom.
pub const VECTOR_MAX_Z: u8 = 14;
const WORKERS: usize = 8;
/// How often idle workers look whether a blocked job can go on.
const BLOCKED_CHECK: Duration = Duration::from_millis(20);
/// Finished results waiting to be collected. Workers pause beyond this, which limits how
/// much memory decoded tiles can take when they are produced faster than they are uploaded.
const RESULT_QUEUE: usize = 32;
const DEM_CACHE: usize = 64;
const VECTOR_CACHE: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    /// Position in the tile's imagery, which may be a part of a coarser image.
    pub uv: [f32; 2],
    /// Position within this tile, for the overlay drawn just for it.
    pub st: [f32; 2],
    /// Unit surface normal in the Earth-fixed frame, for sunlight.
    pub normal: [f32; 3],
}

pub struct TileData {
    pub key: TileKey,
    pub center: DVec3,
    pub vertices: Vec<Vertex>,
    /// Terrain heights on the (GRID + 1)² mesh grid.
    pub heights: Vec<f32>,
    /// RGBA8 mip chain, largest first.
    pub mips: Vec<(u32, u32, Vec<u8>)>,
    /// What to credit for this tile's imagery, beyond the credits always shown.
    pub credit: &'static str,
    /// Which choice of map services this was made under, as `service_lib::Active` counts.
    pub service_generation: u64,
}

pub struct OverlayData {
    pub key: TileKey,
    /// The vector tile the data came from.
    pub source: TileKey,
    pub labels: Arc<Vec<Label>>,
    /// Premultiplied RGBA8 mip chain, largest first. `None` when there are no lines to draw.
    pub mips: Option<Vec<(u32, u32, Vec<u8>)>>,
    /// The state of the imports it was drawn with.
    pub generation: u64,
}

pub enum Outcome {
    Ready(Box<TileData>),
    /// The server has no such tile, so asking again is pointless.
    Missing(TileKey),
    /// Something went wrong that may pass; a server may have said when to ask again.
    Retry(TileKey, Option<SystemTime>),
    Overlay(Box<OverlayData>),
    /// Map data could not be fetched at the moment; the tile stays usable without it.
    OverlayRetry(TileKey, Option<SystemTime>),
    /// A tile's building mesh, or `None` when it has no buildings.
    Buildings(TileKey, Option<Box<BuildingMesh>>),
    BuildingsRetry(TileKey, Option<SystemTime>),
    /// Getting on with this job now would mean waiting, so it goes back in the queue.
    Deferred(Wait),
}

/// What a failed fetch means for a job: another try now or later.
fn after_failure(e: FetchError, k: TileKey, retry: fn(TileKey, Option<SystemTime>) -> Outcome) -> Outcome {
    match e {
        FetchError::Busy(wait) => Outcome::Deferred(wait),
        e => retry(k, asks_to_wait(&e)),
    }
}

fn asks_to_wait(error: &FetchError) -> Option<SystemTime> {
    match error {
        FetchError::Paused(until) => Some(*until),
        _ => None,
    }
}

/// Whether an image is one flat colour, as services send for places they have no data for.
pub fn is_blank(rgba: &[u8]) -> bool {
    let Some(first) = rgba.get(..3) else { return true };
    let close = |a: u8, b: u8| a.abs_diff(b) <= BLANK_RANGE;
    rgba.chunks_exact(4)
        .all(|p| close(p[0], first[0]) && close(p[1], first[1]) && close(p[2], first[2]))
}

/// Cuts a large image down to the part a tile shows, which keeps graphics memory the same as
/// with 256-pixel sources. Returns the image with the tile's position in it, as `rect_in` does.
fn keep_tile_part(img: image::RgbaImage, rect: (f64, f64, f64)) -> (image::RgbaImage, (f64, f64, f64)) {
    let (u, v, size) = rect;
    if img.width() <= 256 || size >= 1.0 {
        return (img, rect);
    }
    let (w, h) = img.dimensions();
    let part = image::imageops::crop_imm(
        &img,
        (u * w as f64).round() as u32,
        (v * h as f64).round() as u32,
        (size * w as f64).round().max(1.0) as u32,
        (size * h as f64).round().max(1.0) as u32,
    )
    .to_image();
    (part, (0.0, 0.0, 1.0))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Job {
    Tile(TileKey),
    Overlay(TileKey),
    Buildings(TileKey),
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
    let width = std::f64::consts::TAU * radius() / crate::body::current().grid.columns(k.z) as f64;
    let skirt = (width * 0.02).clamp(5.0, 50_000.0);
    let position = |s: f64, t: f64, h: f64| -> DVec3 {
        let (lat, lon) = k.lat_lon(s, t);
        geodetic_to_ecef(lat, lon, h) - center
    };
    let vertex = |s: f64, t: f64, h: f64, normal: DVec3| -> Vertex {
        Vertex {
            pos: position(s, t, h).as_vec3().to_array(),
            uv: [
                (uv_rect.0 + s * uv_rect.2) as f32,
                (uv_rect.1 + t * uv_rect.2) as f32,
            ],
            st: [s as f32, t as f32],
            normal: normal.as_vec3().to_array(),
        }
    };
    let at = |i: usize, j: usize| (i as f64 / GRID as f64, j as f64 / GRID as f64);
    for j in 0..side {
        for i in 0..side {
            let (s, t) = at(i, j);
            heights.push(dem.sample(dem_rect.0 + s * dem_rect.2, dem_rect.1 + t * dem_rect.2) * exaggeration);
        }
    }
    // Smooth normals from the surface slope across neighbouring grid points.
    let normals: Vec<DVec3> = (0..side * side)
        .map(|n| {
            let (i, j) = (n % side, n / side);
            let p = |i: usize, j: usize| {
                let (s, t) = at(i, j);
                position(s, t, heights[j * side + i] as f64)
            };
            let along_s = p((i + 1).min(GRID), j) - p(i.saturating_sub(1), j);
            let along_t = p(i, (j + 1).min(GRID)) - p(i, j.saturating_sub(1));
            // t grows southwards, so s x t points down.
            let n = along_t.cross(along_s).normalize_or_zero();
            if n == DVec3::ZERO { (center + position(0.5, 0.5, 0.0)).normalize() } else { n }
        })
        .collect();
    for j in 0..side {
        for i in 0..side {
            let (s, t) = at(i, j);
            vertices.push(vertex(s, t, heights[j * side + i] as f64, normals[j * side + i]));
        }
    }
    for (i, j) in border() {
        let (s, t) = at(i, j);
        vertices.push(vertex(s, t, heights[j * side + i] as f64 - skirt, normals[j * side + i]));
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
    /// What waiting jobs found in their way the last time they tried.
    blocked: HashMap<Job, Wait>,
}

impl Queue {
    /// Takes the most urgent job that has nothing in its way.
    fn next_job(&mut self, fetcher: &Fetcher) -> Option<Job> {
        let at = self.waiting.iter().rposition(|j| self.blocked.get(j).is_none_or(|w| fetcher.is_clear(w)))?;
        let job = self.waiting.remove(at);
        self.blocked.remove(&job);
        Some(job)
    }
}

struct Shared {
    fetcher: Arc<Fetcher>,
    exaggeration: f32,
    names: Vec<String>,
    queue: Mutex<Queue>,
    wake: Condvar,
    dems: Mutex<HashMap<TileKey, Arc<Dem>>>,
    vectors: Mutex<HashMap<TileKey, Arc<VectorTile>>>,
    /// What was imported, with the count of changes that made it.
    imports: Mutex<(Arc<Vec<Arc<Imported>>>, u64)>,
    lines: Mutex<LineKinds>,
}

impl Shared {
    fn vector(&self, k: TileKey) -> Result<Arc<VectorTile>, FetchError> {
        if let Some(v) = self.vectors.lock().unwrap().get(&k) {
            return Ok(v.clone());
        }
        let tile = match self.fetcher.try_fetch(&sources::MAP, k) {
            // Where no data exists, such as on the open ocean, there is simply nothing to draw.
            Err(FetchError::Missing) => VectorTile::default(),
            Err(e) => return Err(e),
            Ok(bytes) => match vector::decode(&bytes, k, &self.names) {
                Some(tile) => tile,
                None => {
                    eprintln!("map data: tile {}/{}/{} is damaged", k.z, k.x, k.y);
                    self.fetcher.discard(&sources::MAP, k);
                    return Err(FetchError::Transient);
                }
            },
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
        let tile = match self.vector(k) {
            Ok(tile) => tile,
            Err(e) => return after_failure(e, k, Outcome::BuildingsRetry),
        };
        if tile.buildings.is_empty() {
            return Outcome::Buildings(k, None);
        }
        let (dem_key, rect) = terrain_source(k);
        let dem = match self.dem(dem_key) {
            Ok(dem) => dem,
            Err(e) => return after_failure(e, k, Outcome::BuildingsRetry),
        };
        let ground = |x: f64, y: f64| dem.sample(rect.0 + x * rect.2, rect.1 + y * rect.2) * self.exaggeration;
        let mesh = buildings::build_mesh(k, &tile.buildings, &ground);
        Outcome::Buildings(k, mesh.map(Box::new))
    }

    fn load_overlay(&self, k: TileKey) -> Outcome {
        let source = k.ancestor(k.z.min(VECTOR_MAX_Z));
        let tile = match self.vector(source) {
            Ok(tile) => tile,
            Err(e) => return after_failure(e, k, Outcome::OverlayRetry),
        };
        let (imports, generation) = self.imports.lock().unwrap().clone();
        let mips = overlay::render(&tile, k, source, &imports, *self.lines.lock().unwrap())
            .map(|rgba| build_mips(OVERLAY_SIZE, OVERLAY_SIZE, rgba));
        Outcome::Overlay(Box::new(OverlayData { key: k, source, labels: tile.labels.clone(), mips, generation }))
    }

    fn dem(&self, k: TileKey) -> Result<Arc<Dem>, FetchError> {
        if let Some(d) = self.dems.lock().unwrap().get(&k) {
            return Ok(d.clone());
        }
        let dem = match self.fetcher.try_fetch(&sources::TERRAIN, k) {
            // Where no terrain exists, the surface is simply flat.
            Err(FetchError::Missing) => Dem::flat(),
            Err(e) => return Err(e),
            Ok(bytes) => match Dem::from_terrarium(&bytes) {
                Some(dem) => dem,
                None => {
                    eprintln!("terrain: tile {}/{}/{} is damaged", k.z, k.x, k.y);
                    self.fetcher.discard(&sources::TERRAIN, k);
                    return Err(FetchError::Transient);
                }
            },
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
        // Terrain first: a job sent back to the queue then does not decode its image again.
        let (dem_key, dem_rect) = terrain_source(k);
        let dem = if crate::body::current().surface_data {
            match self.dem(dem_key) {
                Ok(dem) => dem,
                Err(e) => return after_failure(e, k, Outcome::Retry),
            }
        } else {
            Arc::new(Dem::flat())
        };
        let active = service_lib::active();
        // Best imagery first. While a better source is unavailable for a moment the tile
        // waits, with a coarser one staying on screen, so nothing gets stuck on poor imagery.
        for (source, source_key) in sources::imagery_for(k) {
            if source.blank_means_missing() && self.fetcher.empty_above(source, source_key) {
                continue;
            }
            let user = service_lib::is_service(source);
            let bytes = match self.fetcher.try_fetch(source, source_key) {
                Ok(bytes) => bytes,
                Err(FetchError::Missing) => continue,
                Err(e) => {
                    if user {
                        note_fetch_trouble(source, &e);
                    }
                    return after_failure(e, k, Outcome::Retry);
                }
            };
            let Ok(img) = image::load_from_memory(&bytes) else {
                eprintln!("{}: tile {}/{}/{} is damaged", source.id, source_key.z, source_key.x, source_key.y);
                self.fetcher.discard(source, source_key);
                if user {
                    // A web page or error text instead of a picture: do not ask again this run.
                    service_lib::note_problem(source, "sent something that is not a picture, such as an error page".into());
                    return Outcome::Missing(k);
                }
                return Outcome::Retry(k, None);
            };
            let img = img.to_rgba8();
            if user {
                if let Err(why) = service_lib::check_size(source, img.width(), img.height()) {
                    self.fetcher.discard(source, source_key);
                    service_lib::note_problem(source, why);
                    return Outcome::Missing(k);
                }
                service_lib::note_good(source);
            }
            if source.blank_means_missing() && is_blank(img.as_raw()) {
                self.fetcher.mark_missing(source, source_key);
                continue;
            }
            let (img, uv_rect) = keep_tile_part(img, k.rect_in(source_key));
            if source.blank_means_missing() && is_blank(img.as_raw()) {
                continue; // Others may still have this part, so the image stays known.
            }
            let (img, uv_rect, credit) = match self.with_service_overlay(k, &active, img, uv_rect, source.credit) {
                Ok(x) => x,
                Err(outcome) => return outcome,
            };
            let (w, h) = img.dimensions();
            let (center, vertices, heights) = build_mesh(k, &dem, dem_rect, uv_rect, self.exaggeration);
            return Outcome::Ready(Box::new(TileData {
                key: k,
                center,
                vertices,
                heights,
                mips: build_mips(w, h, img.into_raw()),
                credit,
                service_generation: active.generation,
            }));
        }
        Outcome::Missing(k)
    }
}

/// A tile's picture with its position in the image and the credit it is owed.
type Imagery = (image::RgbaImage, (f64, f64, f64), &'static str);

impl Shared {
    /// Lays the chosen map service over a tile's imagery, if there is one with a picture for it.
    /// A service that fails leaves the imagery as it is; only a wait for its turn is passed on.
    fn with_service_overlay(
        &self,
        k: TileKey,
        active: &service_lib::Active,
        base: image::RgbaImage,
        base_rect: (f64, f64, f64),
        credit: &'static str,
    ) -> Result<Imagery, Outcome> {
        let Some(over) = active.overlay.filter(|o| k.z >= o.min_z && o.covers(k)) else { return Ok((base, base_rect, credit)) };
        let key = over.source_key(k);
        let bytes = match self.fetcher.try_fetch(over, key) {
            Ok(bytes) => bytes,
            Err(FetchError::Missing) => return Ok((base, base_rect, credit)),
            Err(FetchError::Busy(wait)) => return Err(Outcome::Deferred(wait)),
            Err(e) => {
                note_fetch_trouble(over, &e);
                return Ok((base, base_rect, credit));
            }
        };
        let picture = image::load_from_memory(&bytes).map(|i| i.to_rgba8());
        let picture = match picture {
            Ok(p) => p,
            Err(_) => {
                self.fetcher.discard(over, key);
                service_lib::note_problem(over, "sent something that is not a picture, such as an error page".into());
                return Ok((base, base_rect, credit));
            }
        };
        if let Err(why) = service_lib::check_size(over, picture.width(), picture.height()) {
            self.fetcher.discard(over, key);
            service_lib::note_problem(over, why);
            return Ok((base, base_rect, credit));
        }
        service_lib::note_good(over);
        let (picture, over_rect) = keep_tile_part(picture, k.rect_in(key));
        let mixed = blend::composite(&base, base_rect, &picture, over_rect, active.opacity);
        let credit = if credit.is_empty() {
            over.credit
        } else {
            service_lib::intern(format!("{credit}{}{}", service_lib::CREDIT_SEP, over.credit))
        };
        Ok((mixed, (0.0, 0.0, 1.0), credit))
    }
}

/// Tells the panel why a map service's tiles are not arriving.
fn note_fetch_trouble(source: &sources::TileSource, e: &FetchError) {
    let why = match e {
        FetchError::Paused(_) => "is being left alone for a while after errors",
        FetchError::Transient => "did not answer or sent an error",
        _ => return,
    };
    service_lib::note_problem(source, why.into());
}

pub struct Loader {
    shared: Arc<Shared>,
    results: Receiver<(Job, Outcome)>,
}

impl Loader {
    pub fn new(fetcher: Arc<Fetcher>, exaggeration: f32, names: Vec<String>) -> Self {
        let shared = Arc::new(Shared {
            fetcher,
            exaggeration,
            names,
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            dems: Mutex::new(HashMap::new()),
            vectors: Mutex::new(HashMap::new()),
            imports: Mutex::new((Arc::default(), 0)),
            lines: Mutex::new(LineKinds { streets: true, trails: true, trains: true, boundaries: true }),
        });
        let (tx, results) = sync_channel(RESULT_QUEUE);
        for _ in 0..WORKERS {
            let (shared, tx) = (shared.clone(), tx.clone());
            std::thread::spawn(move || worker(&shared, &tx));
        }
        Self { shared, results }
    }

    /// Sets what overlays are drawn with besides map data. Overlays already drawn carry the
    /// generation they were made with, so that older ones can be told apart.
    pub fn set_imports(&self, docs: Arc<Vec<Arc<Imported>>>, generation: u64) {
        *self.shared.imports.lock().unwrap() = (docs, generation);
    }

    /// Chooses which kinds of line later overlays are drawn with. Overlays already drawn are
    /// made again by the caller, under a new generation (see `set_imports`).
    pub fn set_lines(&self, kinds: LineKinds) {
        *self.shared.lines.lock().unwrap() = kinds;
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
        if !q.blocked.is_empty() {
            let current: HashSet<Job> = q.waiting.iter().copied().collect();
            q.blocked.retain(|j, _| current.contains(j));
        }
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
                if let Some(j) = q.next_job(&shared.fetcher) {
                    q.in_flight.insert(j);
                    break j;
                }
                // Blocked jobs may become free at any moment, without anyone saying so here.
                q = if q.waiting.is_empty() {
                    shared.wake.wait(q).unwrap()
                } else {
                    shared.wake.wait_timeout(q, BLOCKED_CHECK).unwrap().0
                };
            }
        };
        let outcome = match job {
            Job::Tile(k) => shared.load(k),
            Job::Overlay(k) => shared.load_overlay(k),
            Job::Buildings(k) => shared.load_buildings(k),
        };
        if let Outcome::Deferred(wait) = outcome {
            let mut q = shared.queue.lock().unwrap();
            q.in_flight.remove(&job);
            q.blocked.insert(job, wait);
            // The next update of the queue restores its place.
            q.waiting.insert(0, job);
            continue;
        }
        if tx.send((job, outcome)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::DiskCache;
    use crate::fetcher::Peers;
    use glam::Vec3;
    use std::time::Instant;

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

    /// A loader whose imagery is cached but unreadable, so jobs finish fast with no network.
    fn offline_loader(tag: &str, keys: &[TileKey]) -> (Loader, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("orbis-loader-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cache = DiskCache::new(root.clone());
        for k in keys {
            cache.put(&sources::EOX.cache_name(*k), b"not an image").unwrap();
        }
        let fetcher = Fetcher::new(cache, Peers::default(), true);
        (Loader::new(Arc::new(fetcher), 1.0, vec!["name".into()]), root)
    }

    /// A small JPEG with the given pixels, so that a tile can be decoded for real.
    fn tile_bytes(pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(16, 16, |x, y| image::Rgb(pixel(x, y)));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn load_with(cache: &DiskCache, k: TileKey) -> Outcome {
        let fetcher = Fetcher::new(cache.clone(), Peers::default(), true);
        let loader = Loader::new(Arc::new(fetcher), 1.0, vec!["name".into()]);
        loader.shared.load(k)
    }

    /// A close tile over Innsbruck, with its flat terrain already known.
    fn detail_setup(tag: &str) -> (DiskCache, TileKey, TileKey) {
        let cache = crate::fetcher::tests::temp_cache(tag);
        let k = TileKey::containing(47.27f64.to_radians(), 11.4f64.to_radians(), 18);
        cache.mark_missing(&sources::TERRAIN.cache_name(k.ancestor(14)));
        cache.mark_missing(&sources::by_id("versatiles").unwrap().cache_name(k.ancestor(17)));
        (cache, k, k.ancestor(14))
    }

    #[test]
    fn detail_imagery_is_preferred_where_it_exists() {
        let (cache, k, base) = detail_setup("prefer");
        let at = sources::by_id("basemap-at").unwrap();
        cache.put(&at.cache_name(k), &tile_bytes(|x, y| [x as u8 * 10, y as u8 * 10, 50])).unwrap();
        cache.put(&sources::EOX.cache_name(base), &tile_bytes(|x, _| [x as u8, 9, 9])).unwrap();
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(tile.credit, "Grundkarte: basemap.at");
        assert_eq!(tile.vertices[0].uv, [0.0, 0.0], "its own image, used whole");
    }

    #[test]
    fn a_blank_answer_falls_back_to_the_base_and_is_remembered() {
        let (cache, k, base) = detail_setup("blank");
        let at = sources::by_id("basemap-at").unwrap();
        cache.put(&at.cache_name(k), &tile_bytes(|_, _| [255, 255, 255])).unwrap();
        cache.put(&sources::EOX.cache_name(base), &tile_bytes(|x, y| [x as u8, y as u8, 9])).unwrap();
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(tile.credit, "", "the base imagery's credit is always shown");
        // The base is magnified: a part of its image, not all of it.
        let rect = k.rect_in(base);
        assert_eq!(tile.vertices[0].uv, [rect.0 as f32, rect.1 as f32]);
        assert!(cache.is_known_missing(&at.cache_name(k)));
        assert_eq!(cache.get(&at.cache_name(k)), None, "no need to keep it");
    }

    #[test]
    fn an_empty_coarse_image_spares_the_service_all_finer_requests() {
        let (cache, k, base) = detail_setup("empty-above");
        let at = sources::by_id("basemap-at").unwrap();
        // Were it consulted, this finer image would win.
        cache.put(&at.cache_name(k), &tile_bytes(|x, y| [x as u8 * 10, y as u8 * 10, 50])).unwrap();
        cache.put(&sources::EOX.cache_name(base), &tile_bytes(|x, y| [x as u8, y as u8, 9])).unwrap();
        cache.mark_missing(&at.cache_name(k.ancestor(15)));
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(tile.credit, "");
    }

    #[test]
    fn only_an_empty_image_of_the_same_area_counts() {
        let (cache, k, _) = detail_setup("empty-elsewhere");
        let at = sources::by_id("basemap-at").unwrap();
        cache.put(&at.cache_name(k), &tile_bytes(|x, y| [x as u8 * 10, y as u8 * 10, 50])).unwrap();
        let elsewhere = k.ancestor(15);
        cache.mark_missing(&at.cache_name(TileKey::new(15, elsewhere.x + 1, elsewhere.y)));
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(tile.credit, at.credit);
    }

    #[test]
    fn a_large_source_image_is_shared_between_four_tiles() {
        let cache = crate::fetcher::tests::temp_cache("large");
        let k = TileKey::containing(50.11f64.to_radians(), 8.68f64.to_radians(), 18);
        cache.mark_missing(&sources::TERRAIN.cache_name(k.ancestor(14)));
        let vt = sources::by_id("versatiles").unwrap();
        let img = image::RgbImage::from_fn(512, 512, |x, y| image::Rgb([(x / 2) as u8, (y / 2) as u8, 9]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        cache.put(&vt.cache_name(k.ancestor(17)), &bytes.into_inner()).unwrap();
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(tile.credit, vt.credit);
        assert_eq!((tile.mips[0].0, tile.mips[0].1), (256, 256));
        assert_eq!(tile.vertices[0].uv, [0.0, 0.0], "its own part, used whole");
    }

    #[test]
    fn a_plain_coloured_base_tile_stays_valid() {
        // The base has plain oceans; only detail services use blank to mean "no data".
        let cache = crate::fetcher::tests::temp_cache("ocean");
        let k = TileKey::new(5, 1, 1);
        cache.mark_missing(&sources::TERRAIN.cache_name(k.ancestor(3)));
        cache.put(&sources::EOX.cache_name(k), &tile_bytes(|_, _| [10, 20, 60])).unwrap();
        assert!(matches!(load_with(&cache, k), Outcome::Ready(_)));
    }

    #[test]
    fn a_damaged_tile_is_removed_so_it_can_be_fetched_again() {
        let (cache, k, _) = detail_setup("damaged");
        let at = sources::by_id("basemap-at").unwrap();
        cache.put(&at.cache_name(k), b"not an image").unwrap();
        assert!(matches!(load_with(&cache, k), Outcome::Retry(..)));
        assert_eq!(cache.get(&at.cache_name(k)), None);
        assert!(!cache.is_known_missing(&at.cache_name(k)));
    }

    #[test]
    fn nothing_anywhere_means_missing() {
        let (cache, k, base) = detail_setup("nothing");
        let at = sources::by_id("basemap-at").unwrap();
        cache.mark_missing(&at.cache_name(k));
        cache.mark_missing(&sources::EOX.cache_name(base));
        assert!(matches!(load_with(&cache, k), Outcome::Missing(_)));
    }

    #[test]
    fn blank_images_are_recognised() {
        let flat = [200u8, 200, 200, 255].repeat(100);
        assert!(is_blank(&flat));
        let mut noisy = flat.clone();
        noisy[40] = 201;
        assert!(is_blank(&noisy), "compression leaves small differences");
        noisy[41] = 100;
        assert!(!is_blank(&noisy));
        assert!(is_blank(&[]));
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
    fn overlay_coordinates_stay_within_the_tile_when_imagery_is_borrowed() {
        let k = TileKey::new(16, 34817, 22977);
        let uv_rect = k.rect_in(k.ancestor(14));
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
    fn large_images_are_cut_down_to_the_tiles_part() {
        let img = image::RgbaImage::from_fn(512, 512, |x, y| image::Rgba([(x / 2) as u8, (y / 2) as u8, 0, 255]));
        let parent = TileKey::new(17, 100, 200);
        let (part, rect) = keep_tile_part(img.clone(), TileKey::new(18, 201, 400).rect_in(parent));
        assert_eq!(part.dimensions(), (256, 256));
        assert_eq!(rect, (0.0, 0.0, 1.0));
        assert_eq!(part.get_pixel(0, 0), &image::Rgba([128, 0, 0, 255]));

        // A whole image, and ordinary sources, stay as they are.
        let (same, rect) = keep_tile_part(img, (0.0, 0.0, 1.0));
        assert_eq!((same.width(), rect), (512, (0.0, 0.0, 1.0)));
        let small = image::RgbaImage::new(256, 256);
        let (same, rect) = keep_tile_part(small, (0.5, 0.25, 0.25));
        assert_eq!((same.width(), rect), (256, (0.5, 0.25, 0.25)));
    }

    #[test]
    fn the_most_urgent_job_without_an_obstacle_goes_first() {
        let f = Fetcher::new(crate::fetcher::tests::temp_cache("next-job"), Peers::default(), true);
        let gate = f.gate("slow.example", 1);
        let pass = gate.enter();
        let (low, high) = (Job::Tile(TileKey::new(5, 1, 1)), Job::Tile(TileKey::new(5, 2, 2)));
        let mut q = Queue { waiting: vec![low, high], ..Queue::default() };
        q.blocked.insert(high, Wait::Server("slow.example".into()));

        assert_eq!(q.next_job(&f), Some(low));
        assert_eq!(q.next_job(&f), None, "only the blocked job is left");
        drop(pass);
        assert_eq!(q.next_job(&f), Some(high));
        assert!(q.blocked.is_empty());
    }

    #[test]
    fn a_full_server_does_not_hold_up_jobs_for_other_data() {
        let cache = crate::fetcher::tests::temp_cache("no-holdup");
        let ready = TileKey::new(5, 1, 1);
        cache.mark_missing(&sources::TERRAIN.cache_name(ready.ancestor(3)));
        cache.put(&sources::EOX.cache_name(ready), &tile_bytes(|_, _| [10, 20, 60])).unwrap();

        // Online, but no request gets out: terrain is the only thing not known, and its server
        // stays full. More blocked jobs than workers, and all more urgent than the other one.
        let fetcher = Arc::new(Fetcher::new(cache, Peers::default(), false));
        let gate = fetcher.gate(&crate::fetcher::host_of(sources::TERRAIN.url), 1);
        std::mem::forget(gate.enter());
        let blocked: Vec<TileKey> = (0..(WORKERS as u32 * 2)).map(|i| TileKey::new(5, 16 + i, 16)).collect();
        let mut wanted = vec![ready];
        wanted.extend(&blocked);
        wanted.reverse();

        let loader = Loader::new(fetcher, 1.0, vec!["name".into()]);
        loader.set_wanted(&wanted, &[], &[]);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut got = None;
        while got.is_none() && Instant::now() < deadline {
            got = loader.try_result();
            std::thread::sleep(Duration::from_millis(5));
        }
        let Some(Outcome::Ready(tile)) = got else { panic!("the free job was held up") };
        assert_eq!(tile.key, ready);
        assert!(!loader.is_idle(), "the blocked jobs are still waiting");
        loader.set_wanted(&[], &[], &[]);
    }

    fn png(n: u32, colour: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(n, n, image::Rgba(colour));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    /// An XYZ service with 64-pixel tiles, and a cache holding the terrain note and base tile.
    fn service_setup(tag: &str, name: &str) -> (DiskCache, TileKey, crate::services::Service) {
        let cache = crate::fetcher::tests::temp_cache(tag);
        let k = TileKey::new(5, 1, 1);
        cache.mark_missing(&sources::TERRAIN.cache_name(k.ancestor(3)));
        cache.put(&sources::EOX.cache_name(k), &png(16, [10, 20, 60, 255])).unwrap();
        let mut s = crate::services::xyz_service(&format!("https://{name}.example/{{z}}/{{x}}/{{y}}.png"), false).unwrap();
        s.tile_size = 64;
        s.credit = "Credit".into();
        s.name = name.into();
        s.assign_id();
        (cache, k, s)
    }

    #[test]
    fn a_chosen_service_replaces_the_imagery_and_is_credited() {
        let (cache, k, s) = service_setup("svc-base", "base-ok");
        let src = service_lib::source_for(&s);
        cache.put(&src.cache_name(k), &png(64, [200, 0, 0, 255])).unwrap();
        service_lib::select_base(Some(&s));
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(&tile.mips[0].2[..4], &[200, 0, 0, 255]);
        assert_eq!(tile.credit, "\u{2}Credit");
        assert_eq!(tile.service_generation, service_lib::active().generation);
        service_lib::select_base(None);
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(&tile.mips[0].2[..4], &[10, 20, 60, 255], "the usual imagery is back");
        assert_eq!(tile.credit, "");
    }

    #[test]
    fn a_service_that_sends_web_pages_or_wrong_sizes_gives_no_tile_and_a_reason() {
        let (cache, k, s) = service_setup("svc-bad", "bad-answers");
        let src = service_lib::source_for(&s);
        service_lib::select_base(Some(&s));
        cache.put(&src.cache_name(k), b"<html><body>Error 500</body></html>").unwrap();
        assert!(matches!(load_with(&cache, k), Outcome::Missing(_)));
        assert!(service_lib::problem_text(src).unwrap().contains("not a picture"));
        assert!(cache.get(&src.cache_name(k)).is_none(), "the bad answer is not kept");
        cache.put(&src.cache_name(k), &png(32, [1, 2, 3, 255])).unwrap();
        assert!(matches!(load_with(&cache, k), Outcome::Missing(_)));
        assert!(service_lib::problem_text(src).unwrap().contains("32 by 32"));
    }

    #[test]
    fn an_overlay_is_mixed_into_the_tile_with_its_opacity_and_credit() {
        let (cache, k, s) = service_setup("svc-over", "over-ok");
        let src = service_lib::source_for(&s);
        cache.put(&src.cache_name(k), &png(64, [250, 250, 250, 255])).unwrap();
        service_lib::select_overlay(Some(&s));
        service_lib::set_opacity(0.5);
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        let px = &tile.mips[0].2[..4];
        assert_eq!(px, &[130, 135, 155, 255], "half way between the base and the overlay");
        assert_eq!(tile.credit, "\u{2}Credit");
        service_lib::select_overlay(None);
    }

    #[test]
    fn a_failing_overlay_leaves_the_base_as_it_was() {
        let (cache, k, s) = service_setup("svc-over-bad", "over-bad");
        let src = service_lib::source_for(&s);
        cache.put(&src.cache_name(k), b"oops").unwrap();
        service_lib::select_overlay(Some(&s));
        let Outcome::Ready(tile) = load_with(&cache, k) else { panic!("no tile") };
        assert_eq!(&tile.mips[0].2[..4], &[10, 20, 60, 255]);
        assert!(service_lib::problem_text(src).is_some());
        service_lib::select_overlay(None);
    }
}
