//! The flat map: projections, the map view, tile selection and mesh building for it.
//! Pure maths, independent of the GPU and the window system.
//!
//! Projected coordinates are in units of the sphere's radius, with x east and y north,
//! measured from the central meridian and the equator. Angles are radians.
//!
//! Sources: Equal Earth is from Šavrič, Patterson and Jenny, "The Equal Earth map projection",
//! Int. J. Geographical Information Science 33(3), 2019 (equations 1-3, parameters A1-A4).
//! The others are the standard formulas of Snyder, "Map Projections: A Working Manual" (1987):
//! Mollweide (pp. 249-252), Hammer (pp. 130-133), Mercator and the equirectangular projection.

use crate::geo::TileKey;
use crate::select::{Selection, TILE_TEXELS};
use bytemuck::{Pod, Zeroable};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const A1: f64 = 1.340264;
const A2: f64 = -0.081106;
const A3: f64 = 0.000893;
const A4: f64 = 0.003796;
const SQRT3: f64 = 1.732_050_807_568_877_2;
const SQRT2: f64 = std::f64::consts::SQRT_2;

/// Web Mercator ends here, and so does the imagery.
pub const MERCATOR_LIMIT: f64 = 1.484_422_229_745_332_4;
/// Pixels per texel above which a tile is split.
const REFINE_ABOVE: f64 = 1.25;
/// Most tiles a view may select, drawn and wanted together.
const MAX_TILES: usize = 700;
/// Room left around the whole map when it is fitted to the window.
const FIT_MARGIN: f64 = 1.04;
/// How far past the finest tile one texel may be magnified.
const OVERZOOM: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    EqualEarth,
    Mollweide,
    HammerAitoff,
    Mercator,
    PlateCarree,
}

impl Projection {
    pub const ALL: [Projection; 5] = [
        Projection::EqualEarth,
        Projection::Mollweide,
        Projection::HammerAitoff,
        Projection::Mercator,
        Projection::PlateCarree,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Projection::EqualEarth => "Equal Earth",
            Projection::Mollweide => "Mollweide",
            Projection::HammerAitoff => "Hammer-Aitoff",
            Projection::Mercator => "Mercator",
            Projection::PlateCarree => "Plate Carrée",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s: String = s
            .chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| if c == 'é' || c == 'É' { 'e' } else { c })
            .collect::<String>()
            .to_lowercase();
        Some(match s.as_str() {
            "equalearth" => Projection::EqualEarth,
            "mollweide" => Projection::Mollweide,
            "hammer" | "hammeraitoff" | "aitoff" => Projection::HammerAitoff,
            "mercator" => Projection::Mercator,
            "platecarree" | "platecarre" | "equirectangular" => Projection::PlateCarree,
            _ => return None,
        })
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    /// Projected (x, y) for a latitude and a longitude measured from the central meridian.
    pub fn forward(self, lat: f64, dlon: f64) -> (f64, f64) {
        match self {
            Projection::EqualEarth => {
                let t = (SQRT3 / 2.0 * lat.sin()).clamp(-1.0, 1.0).asin();
                (dlon * 2.0 * SQRT3 * t.cos() / (3.0 * ee_slope(t)), ee_y(t))
            }
            Projection::Mollweide => {
                let t = mollweide_theta(lat);
                (2.0 * SQRT2 / PI * dlon * t.cos(), SQRT2 * t.sin())
            }
            Projection::HammerAitoff => {
                let z = (1.0 + lat.cos() * (dlon / 2.0).cos()).sqrt();
                (2.0 * SQRT2 * lat.cos() * (dlon / 2.0).sin() / z, SQRT2 * lat.sin() / z)
            }
            Projection::Mercator => {
                let lat = lat.clamp(-MERCATOR_LIMIT, MERCATOR_LIMIT);
                (dlon, lat.tan().asinh())
            }
            Projection::PlateCarree => (dlon, lat),
        }
    }

    /// Latitude and longitude from the central meridian, or `None` outside the map's outline.
    pub fn inverse(self, x: f64, y: f64) -> Option<(f64, f64)> {
        const EPS: f64 = 1e-9;
        match self {
            Projection::EqualEarth => {
                if y.abs() > self.half_height() + EPS {
                    return None;
                }
                let t = ee_theta(y);
                let dlon = 3.0 * x * ee_slope(t) / (2.0 * SQRT3 * t.cos());
                let lat = (2.0 * t.sin() / SQRT3).clamp(-1.0, 1.0).asin();
                (dlon.abs() <= PI + EPS).then_some((lat, dlon.clamp(-PI, PI)))
            }
            Projection::Mollweide => {
                if y.abs() > SQRT2 + EPS {
                    return None;
                }
                let t = (y / SQRT2).clamp(-1.0, 1.0).asin();
                let lat = ((2.0 * t + (2.0 * t).sin()) / PI).clamp(-1.0, 1.0).asin();
                if t.cos().abs() < 1e-12 {
                    return (x.abs() < EPS).then_some((lat, 0.0));
                }
                let dlon = PI * x / (2.0 * SQRT2 * t.cos());
                (dlon.abs() <= PI + EPS).then_some((lat, dlon.clamp(-PI, PI)))
            }
            Projection::HammerAitoff => {
                let z2 = 1.0 - (x / 4.0).powi(2) - (y / 2.0).powi(2);
                // The outline is the ellipse where z² is one half.
                if z2 < 0.5 - EPS {
                    return None;
                }
                let z = z2.max(0.5).sqrt();
                let dlon = 2.0 * (z * x).atan2(2.0 * (2.0 * z * z - 1.0));
                Some(((z * y).clamp(-1.0, 1.0).asin(), dlon))
            }
            Projection::Mercator => {
                (x.abs() <= PI + EPS && y.abs() <= PI + EPS).then(|| (y.sinh().atan(), x.clamp(-PI, PI)))
            }
            Projection::PlateCarree => {
                (x.abs() <= PI + EPS && y.abs() <= FRAC_PI_2 + EPS).then(|| (y.clamp(-FRAC_PI_2, FRAC_PI_2), x.clamp(-PI, PI)))
            }
        }
    }

    /// The largest |y| of the map.
    pub fn half_height(self) -> f64 {
        match self {
            Projection::EqualEarth => ee_y(PI / 3.0),
            Projection::Mollweide | Projection::HammerAitoff => SQRT2,
            Projection::Mercator => PI,
            Projection::PlateCarree => FRAC_PI_2,
        }
    }

    /// The largest |x| of the map, which is at the equator.
    pub fn half_width(self) -> f64 {
        self.half_width_at(0.0)
    }

    /// Half the width of the map at height `y`.
    pub fn half_width_at(self, y: f64) -> f64 {
        match self {
            Projection::EqualEarth => {
                let t = ee_theta(y);
                2.0 * SQRT3 * PI * t.cos() / (3.0 * ee_slope(t))
            }
            Projection::Mollweide => 2.0 * SQRT2 * (1.0 - (y / SQRT2).powi(2)).max(0.0).sqrt(),
            Projection::HammerAitoff => 2.0 * SQRT2 * (1.0 - y * y / 2.0).max(0.0).sqrt(),
            Projection::Mercator | Projection::PlateCarree => PI,
        }
    }
}

fn ee_y(t: f64) -> f64 {
    let t2 = t * t;
    t * (A1 + A2 * t2 + t2.powi(3) * (A3 + A4 * t2))
}

/// The derivative of `ee_y` with respect to the auxiliary angle.
fn ee_slope(t: f64) -> f64 {
    let t2 = t * t;
    A1 + 3.0 * A2 * t2 + t2.powi(3) * (7.0 * A3 + 9.0 * A4 * t2)
}

/// Inverts `ee_y` by Newton's method, as the paper suggests.
fn ee_theta(y: f64) -> f64 {
    let limit = PI / 3.0;
    let mut t = (y / 1.2).clamp(-limit, limit);
    for _ in 0..20 {
        let step = (ee_y(t) - y) / ee_slope(t);
        t = (t - step).clamp(-limit, limit);
        if step.abs() < 1e-15 {
            break;
        }
    }
    t
}

/// Solves 2θ + sin 2θ = π sin φ, which Newton's method does slowly near the poles, so the
/// root is also kept inside a shrinking bracket.
fn mollweide_theta(lat: f64) -> f64 {
    if lat.abs() >= FRAC_PI_2 - 1e-12 {
        return FRAC_PI_2.copysign(lat);
    }
    let target = PI * lat.sin();
    let (mut lo, mut hi) = (-FRAC_PI_2, FRAC_PI_2);
    let mut t = lat;
    for _ in 0..100 {
        let f = 2.0 * t + (2.0 * t).sin() - target;
        if f > 0.0 {
            hi = t;
        } else {
            lo = t;
        }
        let slope = 2.0 + 2.0 * (2.0 * t).cos();
        let mut next = t - f / slope.max(1e-12);
        if !(next > lo && next < hi) {
            next = (lo + hi) / 2.0;
        }
        if (next - t).abs() < 1e-15 {
            return next;
        }
        t = next;
    }
    t
}

/// Wraps an angle to -π..π.
pub fn wrap_lon(a: f64) -> f64 {
    a - TAU * ((a + PI) / TAU).floor()
}

/// The map as seen in a window: which projection, which part and how large.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasView {
    pub projection: Projection,
    /// Longitude at the middle of the map.
    pub central_meridian: f64,
    /// The projected point at the middle of the window.
    pub centre: (f64, f64),
    /// Pixels per projected unit.
    pub scale: f64,
    /// Window size in pixels.
    pub size: (f64, f64),
}

impl AtlasView {
    /// A view of the whole map.
    pub fn whole(projection: Projection, central_meridian: f64, size: (f64, f64)) -> Self {
        let mut v = Self {
            projection,
            central_meridian: wrap_lon(central_meridian),
            centre: (0.0, 0.0),
            scale: 1.0,
            size,
        };
        v.scale = v.fit_scale();
        v
    }

    /// The scale at which the whole map fits the window.
    pub fn fit_scale(&self) -> f64 {
        let p = self.projection;
        let (w, h) = (self.size.0.max(1.0), self.size.1.max(1.0));
        (w / (2.0 * p.half_width() * FIT_MARGIN)).min(h / (2.0 * p.half_height() * FIT_MARGIN))
    }

    /// The scale at which a texel of the finest imagery covers `OVERZOOM` pixels.
    pub fn max_scale(&self) -> f64 {
        let texels_round = TILE_TEXELS * crate::body::current().grid.columns(crate::body::max_zoom()) as f64;
        OVERZOOM * texels_round / TAU
    }

    pub fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        self.projection.forward(lat, wrap_lon(lon - self.central_meridian))
    }

    /// Where a point of the map is in the window, in pixels from the top left.
    pub fn screen_of(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.size.0 / 2.0 + (x - self.centre.0) * self.scale,
            self.size.1 / 2.0 - (y - self.centre.1) * self.scale,
        )
    }

    pub fn map_of(&self, (px, py): (f64, f64)) -> (f64, f64) {
        (
            self.centre.0 + (px - self.size.0 / 2.0) / self.scale,
            self.centre.1 - (py - self.size.1 / 2.0) / self.scale,
        )
    }

    /// Clip-space position of a projected point.
    pub fn ndc_of(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            (x - self.centre.0) * self.scale * 2.0 / self.size.0,
            (y - self.centre.1) * self.scale * 2.0 / self.size.1,
        )
    }

    /// Latitude and longitude at a window position, or `None` outside the map.
    #[cfg(test)]
    pub fn lat_lon_at(&self, px: f64, py: f64) -> Option<(f64, f64)> {
        let (x, y) = self.map_of((px, py));
        let (lat, dlon) = self.projection.inverse(x, y)?;
        Some((lat, wrap_lon(dlon + self.central_meridian)))
    }

    /// Latitude and longitude of the middle of the window.
    pub fn centre_lat_lon(&self) -> (f64, f64) {
        let (x, y) = self.centre;
        match self.projection.inverse(x, y) {
            Some((lat, dlon)) => (lat, wrap_lon(dlon + self.central_meridian)),
            None => (0.0, self.central_meridian),
        }
    }

    pub fn set_centre_lat_lon(&mut self, lat: f64, lon: f64) {
        self.centre = self.project(lat, lon);
        self.clamp();
    }

    /// Keeps the scale in range and the middle of the window on the map.
    pub fn clamp(&mut self) {
        let fit = self.fit_scale();
        self.scale = self.scale.clamp(fit.min(self.max_scale()), self.max_scale());
        if self.scale <= fit * (1.0 + 1e-9) {
            // The whole map is in view, so it stays centred.
            self.centre = (0.0, 0.0);
            return;
        }
        let p = self.projection;
        self.centre.1 = self.centre.1.clamp(-p.half_height(), p.half_height());
        let w = p.half_width_at(self.centre.1);
        self.centre.0 = self.centre.0.clamp(-w, w);
    }

    /// Moves the map by a drag of this many pixels.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.centre.0 -= dx / self.scale;
        self.centre.1 += dy / self.scale;
        self.clamp();
    }

    /// Scales by `factor`, keeping the map point under the window position where it is.
    pub fn zoom_at(&mut self, (px, py): (f64, f64), factor: f64) {
        let held = self.map_of((px, py));
        let before = self.scale;
        self.scale *= factor;
        self.clamp();
        if self.scale != before {
            self.centre = (
                held.0 - (px - self.size.0 / 2.0) / self.scale,
                held.1 + (py - self.size.1 / 2.0) / self.scale,
            );
            self.clamp();
        }
    }

    /// Changes the projection, keeping the point at the middle of the window.
    pub fn set_projection(&mut self, projection: Projection) {
        let (lat, lon) = self.centre_lat_lon();
        // The same relative size: the part of the window the map's height takes.
        let relative = self.scale / self.fit_scale();
        self.projection = projection;
        self.scale = relative * self.fit_scale();
        self.set_centre_lat_lon(lat, lon);
    }

    /// Changes the central meridian, keeping the point at the middle of the window.
    pub fn set_central_meridian(&mut self, lon: f64) {
        let (lat, centre_lon) = self.centre_lat_lon();
        self.central_meridian = wrap_lon(lon);
        self.set_centre_lat_lon(lat, centre_lon);
    }

    pub fn resize(&mut self, size: (f64, f64)) {
        let (lat, lon) = self.centre_lat_lon();
        let relative = self.scale / self.fit_scale();
        self.size = size;
        self.scale = relative * self.fit_scale();
        self.set_centre_lat_lon(lat, lon);
    }

    /// Shows the whole map, centred.
    pub fn show_whole(&mut self) {
        self.centre = (0.0, 0.0);
        self.scale = self.fit_scale();
    }

    /// Metres on the ground that one pixel covers at the equator.
    pub fn metres_per_pixel(&self) -> f64 {
        crate::geo::radius() / self.scale
    }

    /// The scale that shows about what a globe view at this distance shows.
    pub fn scale_for_distance(&self, distance: f64) -> f64 {
        self.size.1 * crate::geo::radius() / (2.0 * (crate::camera::FOV_Y / 2.0).tan() * distance.max(1.0))
    }

    /// Shows what a globe view of this place and distance shows. The whole Earth gives the whole map.
    pub fn match_globe(&mut self, lat: f64, lon: f64, distance: f64) {
        if distance >= crate::nav::whole_body() {
            self.show_whole();
            return;
        }
        self.scale = self.scale_for_distance(distance);
        self.clamp();
        self.set_centre_lat_lon(lat, lon);
    }

    /// The globe distance whose view has about the extent of this map view.
    pub fn distance_for_scale(&self) -> f64 {
        self.size.1 * crate::geo::radius() / (2.0 * (crate::camera::FOV_Y / 2.0).tan() * self.scale)
    }
}

/// One part of a tile in the map. A tile that crosses the seam at the far side of the
/// central meridian is cut in two so that each part stays whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// The part of the tile's width, in 0..1.
    pub s: (f64, f64),
    /// What to add to longitudes to bring this part next to the central meridian.
    shift: f64,
}

impl Piece {
    fn rel_lon(&self, k: TileKey, s: f64, cm: f64) -> f64 {
        let n = crate::body::current().grid.columns(k.z) as f64;
        TAU * (k.x as f64 + s) / n - PI - cm - self.shift
    }
}

pub fn pieces(k: TileKey, central_meridian: f64) -> Vec<Piece> {
    let n = crate::body::current().grid.columns(k.z) as f64;
    let lon0 = TAU * k.x as f64 / n - PI;
    let width = TAU / n;
    // The seam lies at the central meridian plus half a turn, repeating every n tile widths.
    let seam = ((central_meridian + PI - lon0) / width).rem_euclid(n);
    let cuts: Vec<(f64, f64)> = if seam > 1e-9 && seam < 1.0 - 1e-9 {
        vec![(0.0, seam), (seam, 1.0)]
    } else {
        vec![(0.0, 1.0)]
    };
    cuts.into_iter()
        .map(|s| {
            let mid = lon0 + (s.0 + s.1) / 2.0 * width;
            Piece { s, shift: ((mid - central_meridian) / TAU).round() * TAU }
        })
        .collect()
}

/// Size of the tile in pixels, as the larger of its width and height at its middle.
fn tile_pixels(v: &AtlasView, k: TileKey) -> f64 {
    let (lat_mid, lon_mid) = k.lat_lon(0.5, 0.5);
    let (lat_top, _) = k.lat_lon(0.5, 0.0);
    let (lat_bottom, _) = k.lat_lon(0.5, 1.0);
    let (_, lon_left) = k.lat_lon(0.0, 0.5);
    let (_, lon_right) = k.lat_lon(1.0, 0.5);
    let rel = wrap_lon(lon_mid - v.central_meridian);
    let p = v.projection;
    let dist = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let w = dist(
        p.forward(lat_mid, rel - (lon_mid - lon_left)),
        p.forward(lat_mid, rel + (lon_right - lon_mid)),
    );
    let h = dist(p.forward(lat_top, rel), p.forward(lat_bottom, rel));
    w.max(h) * v.scale
}

/// Whether any part of the tile can be in the window.
fn on_screen(v: &AtlasView, k: TileKey) -> bool {
    const N: usize = 8;
    for piece in pieces(k, v.central_meridian) {
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for j in 0..=N {
            for i in 0..=N {
                let s = piece.s.0 + (piece.s.1 - piece.s.0) * i as f64 / N as f64;
                let (lat, _) = k.lat_lon(s, j as f64 / N as f64);
                let (x, y) = v.projection.forward(lat, piece.rel_lon(k, s, v.central_meridian));
                let (px, py) = v.screen_of((x, y));
                lo = (lo.0.min(px), lo.1.min(py));
                hi = (hi.0.max(px), hi.1.max(py));
            }
        }
        // Edges bow out between the samples.
        let pad = 0.05 * (hi.0 - lo.0).max(hi.1 - lo.1) + 2.0;
        if hi.0 + pad >= 0.0 && lo.0 - pad <= v.size.0 && hi.1 + pad >= 0.0 && lo.1 - pad <= v.size.1 {
            return true;
        }
    }
    false
}

/// Chooses the tiles to draw and to load for a map view. As on the globe, a tile is replaced
/// by its children only once all of them can be drawn.
pub fn select(v: &AtlasView, loaded: &impl Fn(TileKey) -> bool, max_z: fn(TileKey) -> u8) -> Selection {
    let mut out = Selection::default();
    let mut wanted = Vec::new();
    let mut budget = MAX_TILES;
    for root in crate::geo::root_tiles() {
        visit(v, root, loaded, max_z, &mut out.draw, &mut wanted, &mut budget);
    }
    wanted.sort_by(|a: &(u8, f64, TileKey), b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    out.wanted = wanted.into_iter().map(|w| w.2).collect();
    out
}

#[allow(clippy::too_many_arguments)]
fn visit(
    v: &AtlasView,
    k: TileKey,
    loaded: &impl Fn(TileKey) -> bool,
    max_z: fn(TileKey) -> u8,
    draw: &mut Vec<TileKey>,
    wanted: &mut Vec<(u8, f64, TileKey)>,
    budget: &mut usize,
) -> bool {
    if !on_screen(v, k) {
        return true;
    }
    let pixels = tile_pixels(v, k) / TILE_TEXELS;
    if k.z < max_z(k) && pixels > REFINE_ABOVE && *budget >= 4 {
        let mark = draw.len();
        let mut covered = true;
        // Each level of refinement must pay for the extra tiles it brings.
        *budget -= 3;
        for c in k.children() {
            covered &= visit(v, c, loaded, max_z, draw, wanted, budget);
        }
        if covered {
            return true;
        }
        draw.truncate(mark);
        *budget += 3;
    }
    *budget = budget.saturating_sub(1);
    if loaded(k) {
        draw.push(k);
        true
    } else {
        // Tiles nearest the middle of the window come first.
        let (lat, lon) = k.lat_lon(0.5, 0.5);
        let m = v.screen_of(v.project(lat, lon));
        let d = ((m.0 - v.size.0 / 2.0).powi(2) + (m.1 - v.size.1 / 2.0).powi(2)).sqrt();
        wanted.push((k.z, d, k));
        false
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct AtlasVertex {
    /// Clip-space position.
    pub pos: [f32; 2],
    /// Position in the tile's imagery.
    pub uv: [f32; 2],
    /// Position within the tile, for its overlay.
    pub st: [f32; 2],
    /// Unit surface normal in the Earth-fixed frame, for sunlight.
    pub normal: [f32; 3],
}

/// The upward normal of the ellipsoid at a latitude and longitude.
fn surface_normal(lat: f64, lon: f64) -> [f32; 3] {
    [(lat.cos() * lon.cos()) as f32, (lat.cos() * lon.sin()) as f32, lat.sin() as f32]
}

#[derive(Default)]
pub struct Mesh {
    pub vertices: Vec<AtlasVertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn cell_visible(&self, quad: [u32; 4]) -> bool {
        let p = |i: u32| self.vertices[i as usize].pos;
        let all = |f: &dyn Fn([f32; 2]) -> bool| quad.iter().all(|i| f(p(*i)));
        !(all(&|p| p[0] < -1.0) || all(&|p| p[0] > 1.0) || all(&|p| p[1] < -1.0) || all(&|p| p[1] > 1.0))
    }

    fn quad(&mut self, quad: [u32; 4]) {
        if self.cell_visible(quad) {
            let [a, b, c, d] = quad;
            self.indices.extend([a, c, b, b, c, d]);
        }
    }
}

/// Adds a tile as a grid of small cells, each corner projected exactly. Cells that fall
/// outside the window are left out.
pub fn add_tile(v: &AtlasView, k: TileKey, uv_rect: (f64, f64, f64), mesh: &mut Mesh) {
    let n = if k.z < 3 { 32 } else { 16 };
    for piece in pieces(k, v.central_meridian) {
        let cols = (((piece.s.1 - piece.s.0) * n as f64).ceil() as usize).max(1);
        let first = mesh.vertices.len() as u32;
        for j in 0..=n {
            let t = j as f64 / n as f64;
            for i in 0..=cols {
                let s = piece.s.0 + (piece.s.1 - piece.s.0) * i as f64 / cols as f64;
                let (lat, _) = k.lat_lon(s, t);
                let rel_lon = piece.rel_lon(k, s, v.central_meridian);
                let xy = v.projection.forward(lat, rel_lon);
                let (nx, ny) = v.ndc_of(xy);
                mesh.vertices.push(AtlasVertex {
                    pos: [nx as f32, ny as f32],
                    uv: [(uv_rect.0 + s * uv_rect.2) as f32, (uv_rect.1 + t * uv_rect.2) as f32],
                    st: [s as f32, t as f32],
                    normal: surface_normal(lat, rel_lon + v.central_meridian),
                });
            }
        }
        let row = cols as u32 + 1;
        for j in 0..n as u32 {
            for i in 0..cols as u32 {
                let a = first + j * row + i;
                mesh.quad([a, a + 1, a + row, a + row + 1]);
            }
        }
    }
}

/// Where the underlay's plain colours sit in the renderer's two-colour texture.
const OCEAN_UV: [f32; 2] = [0.5, 0.75];
const ICE_UV: [f32; 2] = [0.5, 0.25];

/// The plain ground below the tiles: ocean everywhere, with ice south of the imagery and
/// the Arctic Ocean to the north, where the tiles end. It also covers the map until tiles arrive.
pub fn add_underlay(v: &AtlasView, imagery_limit: f64, mesh: &mut Mesh) {
    let top = if v.projection == Projection::Mercator { MERCATOR_LIMIT } else { FRAC_PI_2 };
    let step = 2f64.to_radians();
    let mut lats = vec![-top];
    let mut lat = -top + step - (top % step);
    while lat < top - 1e-9 {
        if lat > -top + 1e-9 {
            lats.push(lat);
        }
        lat += step;
    }
    lats.push(top);
    for edge in [-imagery_limit, imagery_limit] {
        if edge.abs() < top && !lats.iter().any(|l| (l - edge).abs() < 1e-9) {
            lats.push(edge);
        }
    }
    lats.sort_by(f64::total_cmp);
    let segments = 72;
    for w in lats.windows(2) {
        let uv = if w[1] <= -imagery_limit + 1e-9 { ICE_UV } else { OCEAN_UV };
        for i in 0..segments {
            let (l0, l1) = (
                -PI + TAU * i as f64 / segments as f64,
                -PI + TAU * (i + 1) as f64 / segments as f64,
            );
            let first = mesh.vertices.len() as u32;
            for (lat, lon) in [(w[0], l0), (w[0], l1), (w[1], l0), (w[1], l1)] {
                let (nx, ny) = v.ndc_of(v.projection.forward(lat, lon));
                mesh.vertices.push(AtlasVertex { pos: [nx as f32, ny as f32], uv, st: [0.0, 0.0], normal: surface_normal(lat, lon + v.central_meridian) });
            }
            mesh.quad([first, first + 1, first + 2, first + 3]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn near(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn every_projection_round_trips() {
        for p in Projection::ALL {
            let limit = if p == Projection::Mercator { 85.0 } else { 89.9 };
            let mut lat = -limit;
            while lat <= limit {
                let mut lon = -179.9;
                while lon <= 179.9 {
                    let (la, lo) = (f64::to_radians(lat), f64::to_radians(lon));
                    let (x, y) = p.forward(la, lo);
                    let (la2, lo2) = p.inverse(x, y).unwrap_or_else(|| panic!("{p:?} lost {lat} {lon}"));
                    assert!(near(la, la2, 1e-8), "{p:?} lat {lat} {lon}: {la} vs {la2}");
                    assert!(near(lo, lo2, 1e-7), "{p:?} lon {lat} {lon}: {lo} vs {lo2}");
                    lon += 17.3;
                }
                lat += 7.1;
            }
        }
    }

    #[test]
    fn the_outline_is_the_edge_of_the_inverse() {
        for p in Projection::ALL {
            let h = p.half_height();
            for i in -9..=9 {
                let y = h * i as f64 / 10.0;
                let w = p.half_width_at(y);
                assert!(p.inverse(w * 0.999, y).is_some(), "{p:?} inside at y={y}");
                assert!(p.inverse(w * 1.01, y).is_none(), "{p:?} outside at y={y}");
                // The outline is where the meridian at half a turn is drawn.
                assert!(p.inverse(0.0, y).is_some());
            }
            assert!(p.inverse(0.0, h * 1.01).is_none());
            assert!(p.inverse(w_of(p) * 1.01, 0.0).is_none());
        }
    }

    fn w_of(p: Projection) -> f64 {
        p.half_width()
    }

    #[test]
    fn equal_earth_matches_the_published_values() {
        let p = Projection::EqualEarth;
        // The map is about 2.05 times as wide as tall.
        let (_, y) = p.forward(FRAC_PI_2, 0.0);
        assert!(near(y, 1.3173627591574, 1e-6), "{y}");
        let (x, _) = p.forward(0.0, PI);
        assert!(near(x, 2.7066, 1e-3), "{x}");
        assert!(near(p.half_width() / p.half_height(), 2.054, 1e-3));
        let (x, _) = p.forward(FRAC_PI_2, PI);
        assert!(near(x / 2.7066, 0.5925, 1e-3), "pole line {x}");
        // Equal area: the area enclosed equals that of the sphere (4π) in these units.
        let n = 400;
        let mut area = 0.0;
        for j in 0..n {
            let y = -p.half_height() + 2.0 * p.half_height() * (j as f64 + 0.5) / n as f64;
            area += 2.0 * p.half_width_at(y) * 2.0 * p.half_height() / n as f64;
        }
        assert!(near(area, 4.0 * PI, 0.01), "{area}");
    }

    #[test]
    fn the_other_projections_match_their_known_extents() {
        assert!(near(Projection::Mollweide.forward(0.0, PI).0, 2.0 * SQRT2, 1e-12));
        assert!(near(Projection::Mollweide.forward(FRAC_PI_2, 0.0).1, SQRT2, 1e-12));
        assert!(near(Projection::HammerAitoff.forward(0.0, PI).0, 2.0 * SQRT2, 1e-12));
        assert!(near(Projection::HammerAitoff.forward(FRAC_PI_2, 0.0).1, SQRT2, 1e-12));
        // Mollweide is equal-area too, so the ellipse has the area of the sphere.
        assert!(near(PI * 2.0 * SQRT2 * SQRT2, 4.0 * PI, 1e-12));
        // Mercator at 45 degrees.
        assert!(near(Projection::Mercator.forward(PI / 4.0, 0.0).1, 0.881_373_587, 1e-8));
        // Mercator is clipped at the limit of the imagery, giving a square.
        let (_, top) = Projection::Mercator.forward(FRAC_PI_2, 0.0);
        assert!(near(top, PI, 1e-12));
        assert!(near(Projection::PlateCarree.forward(0.3, 0.4).0, 0.4, 1e-15));
    }

    #[test]
    fn projection_names_parse_and_cycle() {
        for p in Projection::ALL {
            assert_eq!(Projection::parse(p.name()), Some(p));
            assert_eq!(p.next().previous(), p);
        }
        assert_eq!(Projection::parse("equal-earth"), Some(Projection::EqualEarth));
        assert_eq!(Projection::parse("plate-carree"), Some(Projection::PlateCarree));
        assert_eq!(Projection::parse("hammer"), Some(Projection::HammerAitoff));
        assert_eq!(Projection::parse("robinson"), None);
        let mut p = Projection::EqualEarth;
        let mut seen = HashSet::new();
        for _ in 0..5 {
            seen.insert(p.name());
            p = p.next();
        }
        assert_eq!(seen.len(), 5);
        assert_eq!(p, Projection::EqualEarth);
    }

    fn view(p: Projection) -> AtlasView {
        AtlasView::whole(p, 0.0, (1280.0, 800.0))
    }

    #[test]
    fn the_whole_map_fits_the_window() {
        for p in Projection::ALL {
            let v = view(p);
            let (l, t) = v.screen_of((-p.half_width(), p.half_height()));
            let (r, b) = v.screen_of((p.half_width(), -p.half_height()));
            assert!(l >= 0.0 && t >= 0.0 && r <= 1280.0 && b <= 800.0, "{p:?}: {l} {t} {r} {b}");
            assert!(near(v.centre_lat_lon().0, 0.0, 1e-12));
        }
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut v = view(Projection::EqualEarth);
        let pointer = (900.0, 300.0);
        let before = v.lat_lon_at(pointer.0, pointer.1).unwrap();
        for _ in 0..8 {
            v.zoom_at(pointer, 1.5);
            let now = v.lat_lon_at(pointer.0, pointer.1).unwrap();
            assert!(near(now.0, before.0, 1e-9) && near(now.1, before.1, 1e-9), "{now:?} {before:?}");
        }
        assert!(v.scale > view(Projection::EqualEarth).scale * 20.0);
    }

    #[test]
    fn zoom_stays_between_the_whole_map_and_the_finest_imagery() {
        let mut v = view(Projection::Mercator);
        v.zoom_at((10.0, 10.0), 1e-6);
        assert!(near(v.scale, v.fit_scale(), 1e-9));
        v.zoom_at((10.0, 10.0), 1e12);
        assert!(v.scale <= v.max_scale() * (1.0 + 1e-12));
    }

    #[test]
    fn panning_stays_on_the_map() {
        for p in Projection::ALL {
            let mut v = view(p);
            v.zoom_at((640.0, 400.0), 6.0);
            for (dx, dy) in [(5000.0, 0.0), (-9e4, 3e4), (0.0, 1e6), (0.0, -1e6)] {
                v.pan(dx, dy);
                let (lat, lon) = v.centre_lat_lon();
                assert!(lat.abs() <= FRAC_PI_2 + 1e-9 && lon.abs() <= PI + 1e-9);
                assert!(p.inverse(v.centre.0, v.centre.1).is_some(), "{p:?} left the map");
            }
        }
    }

    #[test]
    fn dragging_moves_the_map_with_the_hand() {
        let mut v = view(Projection::PlateCarree);
        v.zoom_at((640.0, 400.0), 4.0);
        let grabbed = v.lat_lon_at(500.0, 350.0).unwrap();
        v.pan(120.0, 40.0);
        let now = v.lat_lon_at(620.0, 390.0).unwrap();
        assert!(near(now.0, grabbed.0, 1e-9) && near(now.1, grabbed.1, 1e-9));
    }

    #[test]
    fn changing_projection_or_meridian_keeps_the_centre_point() {
        let mut v = view(Projection::Mollweide);
        v.zoom_at((640.0, 400.0), 3.0);
        v.pan(-200.0, 100.0);
        let (lat, lon) = v.centre_lat_lon();
        for p in Projection::ALL {
            v.set_projection(p);
            let (la, lo) = v.centre_lat_lon();
            assert!(near(la, lat, 1e-6) && near(lo, lon, 1e-6), "{p:?}");
        }
        for cm in [-3.0, 0.5, 2.9, 7.0] {
            v.set_central_meridian(cm);
            let (la, lo) = v.centre_lat_lon();
            assert!(near(la, lat, 1e-6) && near(wrap_lon(lo - lon), 0.0, 1e-6), "{cm}");
            assert!(v.central_meridian.abs() <= PI + 1e-12);
        }
    }

    #[test]
    fn a_central_meridian_moves_the_map_sideways() {
        let mut v = view(Projection::EqualEarth);
        v.set_central_meridian(60f64.to_radians());
        // Longitude 60 east is now the middle; Greenwich is left of it.
        let at = |lat: f64, lon: f64| v.screen_of(v.project(lat.to_radians(), lon.to_radians()));
        assert!(at(0.0, 0.0).0 < at(0.0, 60.0).0);
        // The Pacific is cut at 120 west: just east of it is at the left edge, just west at the right.
        assert!(at(0.0, -119.0).0 < at(0.0, 0.0).0);
        assert!(at(0.0, -121.0).0 > at(0.0, 60.0).0);
    }

    #[test]
    fn tiles_cut_at_the_seam() {
        // The seam at the antimeridian coincides with the tile edge: no cut.
        assert_eq!(pieces(TileKey::new(2, 1, 1), 0.0).len(), 1);
        // With the central meridian at 0 east + 90, the seam is at 90 west.
        let cm = 90f64.to_radians();
        let cut: Vec<_> = (0..4).flat_map(|x| pieces(TileKey::new(2, x, 1), cm)).collect();
        assert_eq!(cut.len(), 4, "only the seam tile is cut: {cut:?}");
        let ps = pieces(TileKey::new(1, 0, 0), 45f64.to_radians());
        assert_eq!(ps.len(), 2);
        assert!(near(ps[0].s.1, ps[1].s.0, 1e-12) && ps[0].s.0 == 0.0 && ps[1].s.1 == 1.0);
        // Longitudes of each piece stay on one side of the seam.
        for p in &ps {
            for s in [p.s.0, p.s.1] {
                let rel = p.rel_lon(TileKey::new(1, 0, 0), s, 45f64.to_radians());
                assert!(rel.abs() <= PI + 1e-9, "{rel}");
            }
        }
    }

    #[test]
    fn meshes_stay_inside_the_outline() {
        for p in Projection::ALL {
            for cm in [0.0, 2.2, -1.0] {
                let v = AtlasView::whole(p, cm, (1280.0, 800.0));
                let mut mesh = Mesh::default();
                for z in 0..3u8 {
                    for x in 0..(1u32 << z) {
                        for y in 0..(1u32 << z) {
                            add_tile(&v, TileKey::new(z, x, y), (0.0, 0.0, 1.0), &mut mesh);
                        }
                    }
                }
                assert!(!mesh.indices.is_empty());
                for vert in &mesh.vertices {
                    let (x, y) = v.map_of((
                        (vert.pos[0] as f64 + 1.0) / 2.0 * 1280.0,
                        (1.0 - vert.pos[1] as f64) / 2.0 * 800.0,
                    ));
                    let w = p.half_width_at(y.clamp(-p.half_height(), p.half_height()));
                    assert!(y.abs() <= p.half_height() * 1.0001 + 1e-4, "{p:?} cm={cm}: y={y}");
                    assert!(x.abs() <= w * 1.0001 + 1e-4, "{p:?} cm={cm}: x={x} beyond {w}");
                }
            }
        }
    }

    fn mean_covered_fraction(v: &AtlasView, mesh: &Mesh) -> f64 {
        // Counts window sample points inside the map that lie in some triangle.
        let tri_has = |px: f64, py: f64| {
            mesh.indices.chunks(3).any(|t| {
                let p = |i: u32| {
                    let q = mesh.vertices[i as usize].pos;
                    ((q[0] as f64 + 1.0) / 2.0 * v.size.0, (1.0 - q[1] as f64) / 2.0 * v.size.1)
                };
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let d = |p1: (f64, f64), p2: (f64, f64), p3: (f64, f64)| {
                    (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
                };
                let (d1, d2, d3) = (d((px, py), a, b), d((px, py), b, c), d((px, py), c, a));
                !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
            })
        };
        let (mut inside, mut covered) = (0, 0);
        for j in 0..40 {
            for i in 0..64 {
                let (px, py) = (10.0 + i as f64 * 19.9, 10.0 + j as f64 * 19.7);
                let (x, y) = v.map_of((px, py));
                // Stay clear of the edge, where the polygon only approximates the curve.
                if v.projection.inverse(x * 1.01, y * 1.01).is_none() {
                    continue;
                }
                inside += 1;
                covered += tri_has(px, py) as usize;
            }
        }
        covered as f64 / inside as f64
    }

    #[test]
    fn the_tiles_and_underlay_cover_the_whole_map() {
        for p in Projection::ALL {
            for cm in [0.0, 1.0] {
                let v = AtlasView::whole(p, cm, (1280.0, 800.0));
                let mut tiles = Mesh::default();
                for x in 0..4 {
                    for y in 0..4 {
                        add_tile(&v, TileKey::new(2, x, y), (0.0, 0.0, 1.0), &mut tiles);
                    }
                }
                let f = mean_covered_fraction(&v, &tiles);
                // Above the imagery limit only the underlay draws.
                let mut under = Mesh::default();
                add_underlay(&v, 85.06f64.to_radians(), &mut under);
                let g = mean_covered_fraction(&v, &under);
                assert!(g > 0.999, "{p:?} underlay covers {g}");
                assert!(f > 0.9, "{p:?} tiles cover {f}");
            }
        }
    }

    fn settle(v: &AtlasView) -> Selection {
        let mut have = HashSet::new();
        for _ in 0..40 {
            let s = select(v, &|k| have.contains(&k), |_| 19);
            if s.wanted.is_empty() {
                return s;
            }
            have.extend(s.wanted);
        }
        panic!("selection never settled");
    }

    fn overlaps(a: TileKey, b: TileKey) -> bool {
        let z = a.z.min(b.z);
        a.ancestor(z) == b.ancestor(z)
    }

    #[test]
    fn selection_is_disjoint_bounded_and_scaled_to_the_view() {
        for p in Projection::ALL {
            let mut v = AtlasView::whole(p, 0.7, (1280.0, 800.0));
            let mut finest = 0;
            for zoom in [1.0, 4.0, 40.0, 2000.0, 1e5] {
                v.show_whole();
                v.zoom_at((640.0, 400.0), zoom);
                v.pan(37.0, -11.0);
                let s = settle(&v);
                assert!(!s.draw.is_empty());
                assert!(s.draw.len() < MAX_TILES, "{p:?} zoom {zoom}: {} tiles", s.draw.len());
                for (i, a) in s.draw.iter().enumerate() {
                    for b in &s.draw[i + 1..] {
                        assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
                    }
                }
                let z = s.draw.iter().map(|k| k.z).max().unwrap();
                assert!(z >= finest, "{p:?}: detail should grow with zoom");
                finest = z;
            }
            assert!(finest >= 17, "{p:?} zoomed in only to z{finest}");
        }
    }

    #[test]
    fn the_world_view_needs_few_tiles_and_a_high_view_asks_for_nothing_fine() {
        let v = view(Projection::EqualEarth);
        let sel = select(&v, &|_| false, |_| 19);
        assert!(sel.draw.is_empty());
        assert_eq!(sel.wanted[0], TileKey::new(0, 0, 0), "coarsest first");
        let z = settle(&v).draw.iter().map(|k| k.z).max().unwrap();
        assert!((2..=3).contains(&z), "z{z}");
    }

    #[test]
    fn only_tiles_in_the_window_are_requested() {
        let mut v = view(Projection::Mercator);
        v.zoom_at((640.0, 400.0), 64.0);
        v.set_centre_lat_lon(0.9, 0.2);
        let s = settle(&v);
        assert!(s.draw.len() < 60, "{}", s.draw.len());
        for k in &s.draw {
            assert!(on_screen(&v, *k));
        }
    }

    #[test]
    fn the_mercator_square_is_filled_by_tiles() {
        let v = view(Projection::Mercator);
        let mut tiles = Mesh::default();
        for x in 0..4 {
            for y in 0..4 {
                add_tile(&v, TileKey::new(2, x, y), (0.0, 0.0, 1.0), &mut tiles);
            }
        }
        assert!(mean_covered_fraction(&v, &tiles) > 0.999);
    }

    #[test]
    fn mercator_latitude_is_clipped() {
        assert!(near(MERCATOR_LIMIT, crate::geo::merc_v_to_lat(0.0), 1e-12));
        assert!(near(crate::geo::lat_to_merc_v(MERCATOR_LIMIT), 0.0, 1e-12));
        let (_, y) = Projection::Mercator.forward(89f64.to_radians(), 0.0);
        assert!(near(y, PI, 1e-12));
    }

    #[test]
    fn a_whole_earth_globe_gives_the_whole_map() {
        let mut v = view(Projection::Mollweide);
        v.match_globe(0.5, 0.5, crate::nav::WHOLE_EARTH);
        assert_eq!((v.centre, v.scale), ((0.0, 0.0), v.fit_scale()));
        v.match_globe(0.5, 0.5, 100_000.0);
        let (lat, lon) = v.centre_lat_lon();
        assert!(near(lat, 0.5, 1e-9) && near(lon, 0.5, 1e-9));
        assert!(v.scale > 100.0 * v.fit_scale());
    }

    #[test]
    fn globe_and_map_scales_correspond() {
        let v = view(Projection::EqualEarth);
        for d in [1e5, 3e6, 2.2e7] {
            let mut w = v;
            w.scale = w.scale_for_distance(d);
            assert!(near(w.distance_for_scale() / d, 1.0, 1e-12));
        }
    }
}
