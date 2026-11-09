//! Geodesy of the viewed body (WGS84 for Earth) and tile addressing. Angles are radians,
//! lengths metres.

use crate::body;
use glam::DVec3;
use std::f64::consts::PI;

/// WGS84 equatorial and polar semi-axes. Use `radius()` for the body being viewed.
#[cfg(test)]
pub const A: f64 = 6_378_137.0;
#[cfg(test)]
pub const B: f64 = 6_356_752.314_245;

/// The equatorial radius of the body being viewed.
pub fn radius() -> f64 {
    body::current().a
}

/// Equirectangular imagery of the other bodies is laid out in planetocentric latitude (the
/// angle at the centre, which is what a sphere-based map gives). The rest of the program
/// uses geodetic latitude on the body's spheroid, so rows are converted at the grid.
fn planetographic(lat: f64, e2: f64) -> f64 {
    if e2 == 0.0 {
        lat
    } else {
        (lat.tan() / (1.0 - e2)).atan()
    }
}

/// The geodetic latitude this program uses for a planetocentric one given by a body's maps,
/// names and tracks (radians). `body` is the body's id; Earth's are left alone.
pub fn from_planetocentric(lat: f64, body: &str) -> f64 {
    let e2 = body::all().iter().find(|b| b.id == body).map_or(0.0, |b| b.e2);
    if body == body::EARTH.id {
        lat
    } else {
        planetographic(lat, e2)
    }
}

/// The other way: the planetocentric latitude of a geodetic one, as other sources give positions
/// on the body (radians).
pub fn to_planetocentric(lat: f64, body: &str) -> f64 {
    let e2 = body::all().iter().find(|b| b.id == body).map_or(0.0, |b| b.e2);
    if body == body::EARTH.id {
        lat
    } else {
        planetocentric(lat, e2)
    }
}

fn planetocentric(lat: f64, e2: f64) -> f64 {
    (lat.tan() * (1.0 - e2)).atan()
}

/// How a body's tile pyramid covers it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grid {
    /// Web Mercator: one square tile at zoom 0, rows linear in the Mercator ordinate.
    Mercator,
    /// Plate carrée as in OGC WMTS "EPSG:4326" sets: two square tiles at zoom 0, rows
    /// linear in latitude, top row at the north pole.
    Equirect,
}

impl Grid {
    /// Tiles around the body at a zoom level.
    pub fn columns(self, z: u8) -> u64 {
        match self {
            Grid::Mercator => 1 << z,
            Grid::Equirect => 2 << z,
        }
    }

    pub fn rows(self, z: u8) -> u64 {
        1 << z
    }
}

/// The tiles at zoom 0.
pub fn root_tiles() -> Vec<TileKey> {
    (0..body::current().grid.columns(0) as u32).map(|x| TileKey::new(0, x, 0)).collect()
}

pub fn geodetic_to_ecef(lat: f64, lon: f64, h: f64) -> DVec3 {
    let (a, e2) = (body::current().a, body::current().e2);
    let (sl, cl) = lat.sin_cos();
    let n = a / (1.0 - e2 * sl * sl).sqrt();
    DVec3::new(
        (n + h) * cl * lon.cos(),
        (n + h) * cl * lon.sin(),
        (n * (1.0 - e2) + h) * sl,
    )
}

/// Returns (lat, lon, height).
pub fn ecef_to_geodetic(p: DVec3) -> (f64, f64, f64) {
    let (a, e2) = (body::current().a, body::current().e2);
    let lon = p.y.atan2(p.x);
    let r = (p.x * p.x + p.y * p.y).sqrt();
    let mut lat = p.z.atan2(r * (1.0 - e2));
    for _ in 0..5 {
        let sl = lat.sin();
        let n = a / (1.0 - e2 * sl * sl).sqrt();
        lat = (p.z + e2 * n * sl).atan2(r);
    }
    let (sl, cl) = lat.sin_cos();
    let n = a / (1.0 - e2 * sl * sl).sqrt();
    let h = if cl.abs() > 1e-6 {
        r / cl - n
    } else {
        p.z.abs() - n * (1.0 - e2)
    };
    (lat, lon, h)
}

/// Local (east, north, up) unit vectors.
pub fn enu(lat: f64, lon: f64) -> (DVec3, DVec3, DVec3) {
    let (sl, cl) = lat.sin_cos();
    let (so, co) = lon.sin_cos();
    (
        DVec3::new(-so, co, 0.0),
        DVec3::new(-sl * co, -sl * so, cl),
        DVec3::new(cl * co, cl * so, sl),
    )
}

/// Distance along `dir` to an ellipsoid inflated by `h`. The nearest forward hit, if any.
pub fn ray_ellipsoid(origin: DVec3, dir: DVec3, h: f64) -> Option<f64> {
    let (a, b) = (body::current().a, body::current().b);
    let s = DVec3::new(a + h, a + h, b + h);
    let (o, d) = (origin / s, dir / s);
    let a = d.length_squared();
    let b = o.dot(d);
    let c = o.length_squared() - 1.0;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let near = (-b - root) / a;
    if near > 0.0 {
        return Some(near);
    }
    let far = (-b + root) / a;
    (far > 0.0).then_some(far)
}

/// Latitude for a Web Mercator row coordinate, where 0 is the top and 1 the bottom.
pub fn merc_v_to_lat(v: f64) -> f64 {
    (PI * (1.0 - 2.0 * v)).sinh().atan()
}

pub fn lat_to_merc_v(lat: f64) -> f64 {
    (1.0 - lat.tan().asinh() / PI) / 2.0
}

pub fn lon_to_merc_u(lon: f64) -> f64 {
    (lon + PI) / (2.0 * PI)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct TileKey {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub const fn new(z: u8, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    pub fn children(self) -> [TileKey; 4] {
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        [
            Self::new(z, x, y),
            Self::new(z, x + 1, y),
            Self::new(z, x, y + 1),
            Self::new(z, x + 1, y + 1),
        ]
    }

    /// The tile at zoom `z` (at or above this one) containing this tile.
    pub fn ancestor(self, z: u8) -> TileKey {
        debug_assert!(z <= self.z);
        let d = self.z - z;
        Self::new(z, self.x >> d, self.y >> d)
    }

    /// This tile's square within `ancestor`, as (u0, v0, size) in 0..1.
    pub fn rect_in(self, ancestor: TileKey) -> (f64, f64, f64) {
        let d = self.z - ancestor.z;
        let n = (1u32 << d) as f64;
        let ox = self.x - (ancestor.x << d);
        let oy = self.y - (ancestor.y << d);
        (ox as f64 / n, oy as f64 / n, 1.0 / n)
    }

    /// Position within the tile's grid as fractions of the whole, with `s` and `t` in 0..1.
    fn unit_on(self, grid: Grid, s: f64, t: f64) -> (f64, f64) {
        let (cols, rows) = (grid.columns(self.z) as f64, grid.rows(self.z) as f64);
        ((self.x as f64 + s) / cols, (self.y as f64 + t) / rows)
    }

    /// (lat, lon) of a point within the tile.
    pub fn lat_lon(self, s: f64, t: f64) -> (f64, f64) {
        self.lat_lon_on(body::current().grid, s, t)
    }

    pub fn lat_lon_on(self, grid: Grid, s: f64, t: f64) -> (f64, f64) {
        let (u, v) = self.unit_on(grid, s, t);
        let lat = match grid {
            Grid::Mercator => merc_v_to_lat(v),
            Grid::Equirect => planetographic(PI / 2.0 - v * PI, body::current().e2),
        };
        (lat, u * 2.0 * PI - PI)
    }

    /// The tile at zoom `z` containing a position.
    pub fn containing(lat: f64, lon: f64, z: u8) -> TileKey {
        Self::containing_on(body::current().grid, lat, lon, z)
    }

    pub fn containing_on(grid: Grid, lat: f64, lon: f64, z: u8) -> TileKey {
        let (cols, rows) = (grid.columns(z) as f64, grid.rows(z) as f64);
        let v = match grid {
            Grid::Mercator => lat_to_merc_v(lat),
            Grid::Equirect => (PI / 2.0 - planetocentric(lat, body::current().e2)) / PI,
        };
        let x = ((lon_to_merc_u(lon) * cols).floor() as i64).clamp(0, cols as i64 - 1);
        let y = ((v * rows).floor() as i64).clamp(0, rows as i64 - 1);
        Self::new(z, x as u32, y as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_tracks_land_where_the_imagery_puts_them() {
        let l = 18.65f64.to_radians();
        // Mars is flattened: about 0.2 degrees at Olympus Mons' latitude.
        let g = from_planetocentric(l, "mars").to_degrees();
        assert!((g - 18.854).abs() < 0.01, "{g}");
        assert!((to_planetocentric(from_planetocentric(l, "mars"), "mars") - l).abs() < 1e-12);
        assert_eq!(from_planetocentric(l, "moon"), l);
        assert_eq!(from_planetocentric(l, "earth"), l);
    }

    #[test]
    fn planetocentric_rows_convert_to_geodetic_latitude() {
        // Vesta's flattening: a 45 degree row is a geodetic latitude of about 57 degrees, the
        // conversion is its own inverse, and the equator and poles stay put.
        let e2 = 1.0 - (229.0f64 / 284.5).powi(2);
        let g = planetographic(45f64.to_radians(), e2).to_degrees();
        assert!((g - 57.1).abs() < 0.2, "{g}");
        for lat in [-90.0, -60.0, -10.0, 0.0, 33.0, 89.9] {
            let l = f64::to_radians(lat);
            assert!((planetocentric(planetographic(l, e2), e2) - l).abs() < 1e-9);
        }
        assert_eq!(planetographic(0.0, e2), 0.0);
        assert!((planetographic(PI / 2.0, e2) - PI / 2.0).abs() < 1e-9);
        // A sphere is unchanged.
        assert_eq!(planetographic(0.7, 0.0), 0.7);
    }

    #[test]
    fn ecef_round_trip() {
        for &(lat, lon, h) in &[(0.0, 0.0, 0.0), (0.8, 0.2, 3000.0), (-1.2, -3.0, 1e6), (1.55, 2.0, -50.0)] {
            let (a, b, c) = ecef_to_geodetic(geodetic_to_ecef(lat, lon, h));
            assert!((a - lat).abs() < 1e-9 && (b - lon).abs() < 1e-9 && (c - h).abs() < 1e-3, "{lat} {lon} {h}: {a} {b} {c}");
        }
    }

    #[test]
    fn known_ecef_values() {
        assert!((geodetic_to_ecef(0.0, 0.0, 0.0) - DVec3::new(A, 0.0, 0.0)).length() < 1e-6);
        assert!((geodetic_to_ecef(PI / 2.0, 0.0, 0.0) - DVec3::new(0.0, 0.0, B)).length() < 1e-3);
    }

    #[test]
    fn ray_hits_and_misses() {
        let o = DVec3::new(2.0 * A, 0.0, 0.0);
        assert!((ray_ellipsoid(o, -DVec3::X, 0.0).unwrap() - A).abs() < 1e-6);
        assert!((ray_ellipsoid(o, -DVec3::X, 1000.0).unwrap() - (A - 1000.0)).abs() < 1e-6);
        assert!(ray_ellipsoid(o, DVec3::X, 0.0).is_none());
        assert!(ray_ellipsoid(o, DVec3::Z, 0.0).is_none());
    }

    #[test]
    fn mercator_round_trip_and_extent() {
        for v in [0.05, 0.3, 0.5, 0.9] {
            assert!((lat_to_merc_v(merc_v_to_lat(v)) - v).abs() < 1e-12);
        }
        assert!((merc_v_to_lat(0.0).to_degrees() - 85.0511).abs() < 1e-3);
        assert!(merc_v_to_lat(0.5).abs() < 1e-12);
    }

    #[test]
    fn tile_hierarchy() {
        let t = TileKey::new(5, 17, 9);
        assert_eq!(t.ancestor(3), TileKey::new(3, 4, 2));
        assert_eq!(t.rect_in(TileKey::new(3, 4, 2)), (0.25, 0.25, 0.25));
        for c in t.children() {
            assert_eq!(c.ancestor(5), t);
        }
        // Innsbruck.
        let k = TileKey::containing(47.27f64.to_radians(), 11.39f64.to_radians(), 10);
        assert_eq!((k.x, k.y), (544, 359));
        let (lat, lon) = k.lat_lon(0.5, 0.5);
        assert_eq!(TileKey::containing(lat, lon, 10), k);
    }

    #[test]
    fn equirect_tiles_follow_the_wmts_layout() {
        let g = Grid::Equirect;
        // Zoom 0 is two tiles side by side, each 180 degrees square.
        assert_eq!((g.columns(0), g.rows(0), g.columns(8), g.rows(8)), (2, 1, 512, 256));
        let west = TileKey::new(0, 0, 0);
        assert_eq!(west.lat_lon_on(g, 0.0, 0.0), (PI / 2.0, -PI));
        assert_eq!(west.lat_lon_on(g, 1.0, 1.0), (-PI / 2.0, 0.0));
        assert_eq!(TileKey::new(0, 1, 0).lat_lon_on(g, 0.5, 0.5), (0.0, PI / 2.0));
        // Tycho, 43.3 S 11.2 W, at zoom 5: 64 columns, 32 rows, top left at 90 N 180 W.
        let k = TileKey::containing_on(g, -43.3f64.to_radians(), -11.2f64.to_radians(), 5);
        assert_eq!((k.x, k.y), (((180.0 - 11.2) / 360.0 * 64.0) as u32, ((90.0 + 43.3) / 180.0 * 32.0) as u32));
        let (lat, lon) = k.lat_lon_on(g, 0.5, 0.5);
        assert_eq!(TileKey::containing_on(g, lat, lon, 5), k);
        // The poles and the date line stay inside the grid.
        let k = TileKey::containing_on(g, -PI / 2.0, PI, 3);
        assert_eq!((k.x, k.y), (15, 7));
    }

    #[test]
    fn containing_clamps_at_the_edges() {
        let k = TileKey::containing(89.9f64.to_radians(), PI, 3);
        assert_eq!((k.x, k.y), (7, 0));
    }
}
