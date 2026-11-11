//! Chooses which tiles to draw and which to load for a view.

use crate::body;
use crate::camera::{Basis, FOV_Y};
use crate::geo::{geodetic_to_ecef, radius, Grid, TileKey};
use glam::DVec3;
use std::collections::HashSet;
use std::f64::consts::TAU;

/// Highest mountains plus a margin, so tilted views do not lose peaks at the edges.
const TERRAIN_MARGIN: f64 = 9_000.0;
/// Pixels per texel above which a tile is split.
const REFINE_ABOVE: f64 = 1.25;
pub const TILE_TEXELS: f64 = 256.0;
/// The most zoom levels a loaded tile may give up when it stands in for children that cannot
/// all be drawn yet. Beyond this the finer tiles stay and the missing part is left open until
/// it arrives: one tile not yet loaded at the horizon must not swap the detail under the camera
/// for a tile of the whole continent.
const MAX_STAND_IN: u8 = 2;

pub struct View {
    pub basis: Basis,
    pub aspect: f64,
    pub height_px: f64,
    /// How far a tile may be split, which depends on the imagery available there.
    pub max_z: fn(TileKey) -> u8,
}

#[derive(Default, Debug)]
pub struct Selection {
    pub draw: Vec<TileKey>,
    /// Tiles that are not loaded yet, most urgent first.
    pub wanted: Vec<TileKey>,
}

/// How much of a tile's area the tiles added for it cover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cover {
    Full,
    /// Some is drawn, and the rest waits for tiles still loading.
    Part,
    Nothing,
}

/// Tiles already drawn, and their ancestors.
struct Taken {
    drawn: HashSet<TileKey>,
    above: HashSet<TileKey>,
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
        let a = radius();
        let width = TAU * a / body::current().grid.columns(k.z) as f64;
        let pad = TERRAIN_MARGIN + width * width / (8.0 * a) + width * 0.05;
        // Above the horizon: some sample faces the camera.
        let above = s
            .points
            .iter()
            .any(|p| (b.pos - *p).dot(p.normalize()) > -pad);
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
        sides
            .iter()
            .all(|n| s.points.iter().any(|p| (*p - b.pos).dot(*n) < pad))
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
        // Mercator rows stretch east-west by 1/cos(lat); equirectangular ones do not, and
        // their north-south texel stays the larger one.
        let squeeze = match body::current().grid {
            Grid::Mercator => lat.cos(),
            Grid::Equirect => 1.0,
        };
        let texel =
            TAU * radius() * squeeze / (body::current().grid.columns(k.z) as f64 * TILE_TEXELS);
        let per_metre = self.height_px / (2.0 * (FOV_Y / 2.0).tan() * nearest.max(1.0));
        // Seen at a slant, a texel covers fewer pixels along the view direction. Averaging
        // both axes keeps grazing views from demanding full detail up to the horizon.
        let facing = (self.basis.pos - point).normalize().dot(point.normalize());
        (nearest, texel * per_metre * facing.max(0.04).sqrt())
    }

    pub fn select(&self, loaded: &impl Fn(TileKey) -> bool) -> Selection {
        let mut out = Selection::default();
        let mut wanted = Vec::new();
        for root in crate::geo::root_tiles() {
            self.visit(root, loaded, &mut out.draw, &mut wanted);
        }
        wanted.sort_by(|a: &(u8, f64, TileKey), b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        out.wanted = wanted.into_iter().map(|w| w.2).collect();
        out
    }

    /// The loaded tiles in a region that are not among `drawn`, chosen like `select` chooses them but
    /// without asking for any: where finer tiles are missing a coarser loaded one stands in, and where
    /// nothing is loaded there is a hole. Tiles overlapping a drawn one are never returned. `region` takes a
    /// tile's bounding sphere (centre, radius). Used for ground outside the view that casts shadows into it.
    pub fn select_in(
        &self,
        region: &dyn Fn(DVec3, f64) -> bool,
        loaded: &impl Fn(TileKey) -> bool,
        drawn: &[TileKey],
    ) -> Vec<TileKey> {
        let mut above = HashSet::new();
        for k in drawn {
            above.extend((0..k.z).map(|z| k.ancestor(z)));
        }
        let taken = Taken {
            drawn: drawn.iter().copied().collect(),
            above,
        };
        let mut out = Vec::new();
        for root in crate::geo::root_tiles() {
            self.visit_in(root, region, loaded, &taken, &mut out);
        }
        out
    }

    fn visit_in(
        &self,
        k: TileKey,
        region: &dyn Fn(DVec3, f64) -> bool,
        loaded: &impl Fn(TileKey) -> bool,
        taken: &Taken,
        out: &mut Vec<TileKey>,
    ) -> bool {
        if taken.drawn.contains(&k) {
            return true;
        }
        let s = samples(k);
        let centre = s.points[12];
        let a = radius();
        let width = TAU * a / body::current().grid.columns(k.z) as f64;
        let reach = s
            .points
            .iter()
            .map(|p| (*p - centre).length())
            .fold(0.0, f64::max);
        let pad = TERRAIN_MARGIN + width * width / (8.0 * a) + width * 0.05;
        if !region(centre, reach + pad) {
            return true;
        }
        // Above a drawn tile: only its other children are left to look at.
        if taken.above.contains(&k) {
            for c in k.children() {
                self.visit_in(c, region, loaded, taken, out);
            }
            return true;
        }
        let (_, pixels) = self.measure(k, &s);
        let mut partial = Vec::new();
        if k.z < (self.max_z)(k) && (k.z < 2 || pixels > REFINE_ABOVE) {
            let mut covered = true;
            for c in k.children() {
                covered &= self.visit_in(c, region, loaded, taken, &mut partial);
            }
            if covered {
                out.extend(partial);
                return true;
            }
        }
        if loaded(k) {
            out.push(k);
            true
        } else {
            out.extend(partial);
            false
        }
    }

    /// Adds the tiles to draw for the area of `k` and says how much of it they cover. A tile
    /// is replaced only once all its visible children can be drawn, so that zooming in leaves no
    /// holes, unless standing in for them would give up more than `MAX_STAND_IN` levels.
    fn visit(
        &self,
        k: TileKey,
        loaded: &impl Fn(TileKey) -> bool,
        draw: &mut Vec<TileKey>,
        wanted: &mut Vec<(u8, f64, TileKey)>,
    ) -> Cover {
        let s = samples(k);
        if !self.visible(k, &s) {
            return Cover::Full;
        }
        let (dist, pixels) = self.measure(k, &s);
        let mark = draw.len();
        if k.z < (self.max_z)(k) && (k.z < 2 || pixels > REFINE_ABOVE) {
            let mut covered = true;
            for c in k.children() {
                covered &= self.visit(c, loaded, draw, wanted) == Cover::Full;
            }
            if covered {
                return Cover::Full;
            }
            let finest = draw[mark..].iter().map(|t| t.z).max().unwrap_or(k.z);
            if loaded(k) && finest <= k.z + MAX_STAND_IN {
                draw.truncate(mark);
            } else if !loaded(k) || draw.len() > mark {
                if !loaded(k) {
                    wanted.push((k.z, dist, k));
                }
                return if draw.len() > mark {
                    Cover::Part
                } else {
                    Cover::Nothing
                };
            }
        }
        if loaded(k) {
            draw.push(k);
            Cover::Full
        } else {
            wanted.push((k.z, dist, k));
            Cover::Nothing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use std::collections::HashSet;

    fn view(distance: f64, tilt: f64) -> View {
        let cam = Camera {
            lat: 0.82,
            lon: 0.2,
            target_h: 0.0,
            distance,
            heading: 0.0,
            tilt,
            sky: 0.0,
        };
        View {
            basis: cam.basis(),
            aspect: 1.5,
            height_px: 1000.0,
            max_z: |_| 16,
        }
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
        let zmax = |d| {
            settle(&view(d, 0.0))
                .draw
                .iter()
                .map(|k| k.z)
                .max()
                .unwrap()
        };
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
    fn region_selection_adds_loaded_tiles_beside_the_drawn_ones() {
        let v = view(20_000.0, 0.8);
        let drawn = settle(&v).draw;
        let everywhere = |_: DVec3, _: f64| true;
        // Nothing but the drawn tiles loaded: nothing to add.
        let have: HashSet<TileKey> = drawn.iter().copied().collect();
        assert!(v
            .select_in(&everywhere, &|k| have.contains(&k), &drawn)
            .is_empty());
        // With the world at zoom 3 loaded as well, only tiles clear of the drawn ones come back,
        // not overlapping each other.
        let mut more = have.clone();
        more.extend((0..8u32).flat_map(|x| (0..8u32).map(move |y| TileKey::new(3, x, y))));
        let extra = v.select_in(&everywhere, &|k| more.contains(&k), &drawn);
        assert!(!extra.is_empty() && extra.len() < 64);
        for (i, a) in extra.iter().enumerate() {
            assert!(more.contains(a));
            assert!(drawn.iter().all(|d| !overlaps(*a, *d)), "{a:?}");
            assert!(extra[i + 1..].iter().all(|b| !overlaps(*a, *b)));
        }
        assert!(v
            .select_in(&|_, _| false, &|k| more.contains(&k), &drawn)
            .is_empty());
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
    fn a_missing_tile_at_the_horizon_keeps_the_detail_under_the_camera() {
        let v = view(3_000.0, 1.3);
        let settled = settle(&v);
        let at = TileKey::containing(0.82, 0.2, 16);
        let finest = |draw: &[TileKey]| draw.iter().find(|k| at.ancestor(k.z) == **k).map(|k| k.z);
        let before = finest(&settled.draw).unwrap();
        // The drawn tile farthest from the view point is gone, with all its ancestors below
        // zoom 2, as when tilting up shows country never loaded.
        let far = *settled
            .draw
            .iter()
            .max_by_key(|k| k.x.abs_diff(at.ancestor(k.z).x) + k.y.abs_diff(at.ancestor(k.z).y))
            .unwrap();
        let gone: HashSet<TileKey> = (2..=far.z).map(|z| far.ancestor(z)).collect();
        let mut have: HashSet<TileKey> = settled.draw.iter().copied().collect();
        for k in &settled.draw {
            have.extend((0..k.z).map(|z| k.ancestor(z)));
        }
        have.retain(|k| !gone.contains(k));
        let s = v.select(&|k| have.contains(&k));
        assert_eq!(
            finest(&s.draw),
            Some(before),
            "the view point lost its detail"
        );
        assert!(
            s.wanted.contains(&far.ancestor(2)),
            "the missing part is asked for, coarsest first"
        );
        for (i, a) in s.draw.iter().enumerate() {
            assert!(
                s.draw[i + 1..].iter().all(|b| !overlaps(*a, *b)),
                "{a:?} overlaps"
            );
        }
    }

    #[test]
    fn a_parent_one_level_up_still_stands_in_for_a_missing_child() {
        let v = view(3_000.0, 0.0);
        let settled = settle(&v);
        let missing = *settled.draw.iter().max_by_key(|k| k.z).unwrap();
        let mut have: HashSet<TileKey> = settled.draw.iter().copied().collect();
        for k in &settled.draw {
            have.extend((0..k.z).map(|z| k.ancestor(z)));
        }
        have.remove(&missing);
        let s = v.select(&|k| have.contains(&k));
        let parent = missing.ancestor(missing.z - 1);
        assert!(s.draw.contains(&parent));
        assert!(
            !s.draw
                .iter()
                .any(|k| k.z == missing.z && k.ancestor(parent.z) == parent),
            "no sibling overlaps it"
        );
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
            assert!(
                s.draw.iter().any(|k| at.ancestor(k.z) == *k),
                "uncovered at d={d}"
            );
        }
    }
}
