//! WGS84 geodesy and Web Mercator tile addressing. Angles are radians, lengths metres.

use glam::DVec3;
use std::f64::consts::PI;

pub const A: f64 = 6_378_137.0;
pub const B: f64 = 6_356_752.314_245;
const E2: f64 = 6.694_379_990_14e-3;

pub fn geodetic_to_ecef(lat: f64, lon: f64, h: f64) -> DVec3 {
    let (sl, cl) = lat.sin_cos();
    let n = A / (1.0 - E2 * sl * sl).sqrt();
    DVec3::new(
        (n + h) * cl * lon.cos(),
        (n + h) * cl * lon.sin(),
        (n * (1.0 - E2) + h) * sl,
    )
}

/// Returns (lat, lon, height).
pub fn ecef_to_geodetic(p: DVec3) -> (f64, f64, f64) {
    let lon = p.y.atan2(p.x);
    let r = (p.x * p.x + p.y * p.y).sqrt();
    let mut lat = p.z.atan2(r * (1.0 - E2));
    for _ in 0..5 {
        let sl = lat.sin();
        let n = A / (1.0 - E2 * sl * sl).sqrt();
        lat = (p.z + E2 * n * sl).atan2(r);
    }
    let (sl, cl) = lat.sin_cos();
    let n = A / (1.0 - E2 * sl * sl).sqrt();
    let h = if cl.abs() > 1e-6 {
        r / cl - n
    } else {
        p.z.abs() - n * (1.0 - E2)
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
    let s = DVec3::new(A + h, A + h, B + h);
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

    /// Mercator coordinates of a point within the tile, with `s` and `t` in 0..1.
    pub fn merc(self, s: f64, t: f64) -> (f64, f64) {
        let n = (1u64 << self.z) as f64;
        ((self.x as f64 + s) / n, (self.y as f64 + t) / n)
    }

    /// (lat, lon) of a point within the tile.
    pub fn lat_lon(self, s: f64, t: f64) -> (f64, f64) {
        let (u, v) = self.merc(s, t);
        (merc_v_to_lat(v), u * 2.0 * PI - PI)
    }

    /// The tile at zoom `z` containing a position.
    pub fn containing(lat: f64, lon: f64, z: u8) -> TileKey {
        let n = (1u64 << z) as f64;
        let max = (1u64 << z) as i64 - 1;
        let x = ((lon_to_merc_u(lon) * n).floor() as i64).clamp(0, max);
        let y = ((lat_to_merc_v(lat) * n).floor() as i64).clamp(0, max);
        Self::new(z, x as u32, y as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn containing_clamps_at_the_edges() {
        let k = TileKey::containing(89.9f64.to_radians(), PI, 3);
        assert_eq!((k.x, k.y), (7, 0));
    }
}
