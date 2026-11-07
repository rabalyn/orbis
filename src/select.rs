//! Chooses which tiles to draw and which to load for a view.

use crate::camera::{Basis, FOV_Y};
use crate::geo::{geodetic_to_ecef, TileKey, A};
use glam::DVec3;
use std::f64::consts::TAU;

/// Highest mountains plus a margin, so tilted views do not lose peaks at the edges.
const TERRAIN_MARGIN: f64 = 9_000.0;
/// Pixels per texel above which a tile is split.
const REFINE_ABOVE: f64 = 1.25;
pub const TILE_TEXELS: f64 = 256.0;

pub struct View {
    pub basis: Basis,
    pub aspect: f64,
    pub height_px: f64,
    pub max_z: u8,
}

#[derive(Default, Debug)]
pub struct Selection {
    pub draw: Vec<TileKey>,
    /// Tiles that are not loaded yet, most urgent first.
    pub wanted: Vec<TileKey>,
}

struct Samples {
    points: [DVec3; 25],
    lats: [f64; 25],
}

fn samples(k: TileKey) -> Samples {
    let mut points = [DVec3::ZERO; 25];
    let mut lats = [0.0; 25];
    for j in 0..5 {
        for i in 0..5 {
            let (lat, lon) = k.lat_lon(i as f64 / 4.0, j as f64 / 4.0);
            points[j * 5 + i] = geodetic_to_ecef(lat, lon, 0.0);
            lats[j * 5 + i] = lat;
        }
    }
    Samples { points, lats }
}

impl View {
    fn visible(&self, k: TileKey, s: &Samples) -> bool {
        if k.z < 2 {
            return true;
        }
        let b = &self.basis;
        // The padding covers the surface bulging between samples and terrain rising above
        // the ellipsoid.
        let width = TAU * A / (1u64 << k.z) as f64;
        let pad = TERRAIN_MARGIN + width * width / (8.0 * A) + width * 0.05;
        // Above the horizon: some sample faces the camera.
        let above = s.points.iter().any(|p| (b.pos - *p).dot(p.normalize()) > -pad);
        if !above {
            return false;
        }
        // Not entirely beyond one side of the view.
        let t = (FOV_Y / 2.0).tan();
        let sides = [
            (b.right - b.look * (t * self.aspect)).normalize(),
            (-b.right - b.look * (t * self.aspect)).normalize(),
            (b.up - b.look * t).normalize(),
            (-b.up - b.look * t).normalize(),
            -b.look,
        ];
        sides.iter().all(|n| s.points.iter().any(|p| (*p - b.pos).dot(*n) < pad))
    }

    /// Distance to the nearest sample, and how many screen pixels one texel covers there.
    fn measure(&self, k: TileKey, s: &Samples) -> (f64, f64) {
        let mut nearest = f64::MAX;
        let mut lat = 0.0;
        let mut point = DVec3::X;
        for (p, l) in s.points.iter().zip(s.lats) {
            let d = (*p - self.basis.pos).length();
            if d < nearest {
                nearest = d;
                lat = l;
                point = *p;
            }
        }
        let texel = TAU * A * lat.cos() / ((1u64 << k.z) as f64 * TILE_TEXELS);
        let per_metre = self.height_px / (2.0 * (FOV_Y / 2.0).tan() * nearest.max(1.0));
        // Seen at a slant, a texel covers fewer pixels along the view direction. Averaging
        // both axes keeps grazing views from demanding full detail up to the horizon.
        let facing = (self.basis.pos - point).normalize().dot(point.normalize());
        (nearest, texel * per_metre * facing.max(0.04).sqrt())
    }

    pub fn select(&self, loaded: &impl Fn(TileKey) -> bool) -> Selection {
        let mut out = Selection::default();
        let mut wanted = Vec::new();
        self.visit(TileKey::new(0, 0, 0), loaded, &mut out.draw, &mut wanted);
        wanted.sort_by(|a: &(u8, f64, TileKey), b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        out.wanted = wanted.into_iter().map(|w| w.2).collect();
        out
    }

    /// Returns whether the area of `k` is fully covered by tiles added to `draw`. A tile
    /// is replaced only once all its visible children can be drawn, so nothing leaves a hole.
    fn visit(
        &self,
        k: TileKey,
        loaded: &impl Fn(TileKey) -> bool,
        draw: &mut Vec<TileKey>,
        wanted: &mut Vec<(u8, f64, TileKey)>,
    ) -> bool {
        let s = samples(k);
        if !self.visible(k, &s) {
            return true;
        }
        let (dist, pixels) = self.measure(k, &s);
        if k.z < self.max_z && (k.z < 2 || pixels > REFINE_ABOVE) {
            let mark = draw.len();
            let mut covered = true;
            for c in k.children() {
                covered &= self.visit(c, loaded, draw, wanted);
            }
            if covered {
                return true;
            }
            draw.truncate(mark);
        }
        if loaded(k) {
            draw.push(k);
            true
        } else {
            wanted.push((k.z, dist, k));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use std::collections::HashSet;

    fn view(distance: f64, tilt: f64) -> View {
        let cam = Camera { lat: 0.82, lon: 0.2, target_h: 0.0, distance, heading: 0.0, tilt };
        View { basis: cam.basis(), aspect: 1.5, height_px: 1000.0, max_z: 16 }
    }

    /// Fetches everything wanted until the selection settles.
    fn settle(v: &View) -> Selection {
        let mut have = HashSet::new();
        for _ in 0..40 {
            let s = v.select(&|k| have.contains(&k));
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
    fn drawn_tiles_do_not_overlap() {
        for (d, t) in [(2.2e7, 0.0), (500_000.0, 0.5), (3_000.0, 1.3), (300.0, 0.0)] {
            let s = settle(&view(d, t));
            assert!(!s.draw.is_empty());
            for (i, a) in s.draw.iter().enumerate() {
                for b in &s.draw[i + 1..] {
                    assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?} at d={d}");
                }
            }
        }
    }

    #[test]
    fn closer_views_use_finer_tiles() {
        let zmax = |d| settle(&view(d, 0.0)).draw.iter().map(|k| k.z).max().unwrap();
        assert!(zmax(2.2e7) <= 4);
        assert!(zmax(100_000.0) > zmax(2.2e7));
        assert_eq!(zmax(100.0), 16);
    }

    #[test]
    fn tile_count_stays_practical() {
        for (d, t) in [(2.2e7, 0.0), (1e6, 0.8), (20_000.0, 1.45), (2_000.0, 1.48)] {
            let n = settle(&view(d, t)).draw.len();
            assert!(n < 400, "{n} tiles at d={d} tilt={t}");
        }
    }

    #[test]
    fn nothing_loaded_requests_coarsest_first() {
        let s = view(100_000.0, 0.3).select(&|_| false);
        assert!(s.draw.is_empty());
        assert_eq!(s.wanted[0], TileKey::new(0, 0, 0));
        assert!(s.wanted.windows(2).all(|w| w[0].z <= w[1].z));
    }

    #[test]
    fn parent_stays_until_all_children_arrive() {
        let parent = TileKey::new(0, 0, 0);
        let v = view(100_000.0, 0.0);
        let s = v.select(&|k| k == parent);
        assert_eq!(s.draw, vec![parent]);
        assert!(!s.wanted.is_empty());
    }

    #[test]
    fn far_side_is_culled() {
        let v = view(500_000.0, 0.0);
        let s = settle(&v);
        // A tile on the opposite side of the Earth is not drawn.
        let far = TileKey::containing(-0.82, 0.2 + std::f64::consts::PI, 3);
        assert!(!s.draw.iter().any(|k| k.z >= 3 && k.ancestor(3) == far));
    }

    #[test]
    fn view_point_is_always_covered() {
        for (d, t) in [(2.2e7, 0.0), (500_000.0, 1.0), (3_000.0, 1.3), (50.0, 0.0)] {
            let s = settle(&view(d, t));
            let at = TileKey::containing(0.82, 0.2, 16);
            assert!(s.draw.iter().any(|k| at.ancestor(k.z) == *k), "uncovered at d={d}");
        }
    }
}
