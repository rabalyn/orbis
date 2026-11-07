//! The geometry of cast shadows: sun-direction shadow maps in camera-relative coordinates.
//!
//! The visible ground is split by view depth into a few slices (cascades, after Engel's "Cascaded
//! Shadow Maps" and the practical split scheme of Zhang et al., "Parallel-Split Shadow Maps",
//! 2006). Each slice gets an orthographic box looking along the Sun's rays, fitted round the
//! slice's bounding sphere and stretched towards the Sun so that casters outside the view still
//! reach the ground inside it. A caster of height `H` over a ground point lies `H / sin(elevation)`
//! along the ray from it, which fixes how far the box must reach (the lengths of shadows follow
//! the same simple geometry as a gnomon's). Boxes are snapped to whole texels in the world, so
//! edges stay put while the camera moves. Nothing here touches the GPU or a window.

use crate::camera::{Basis, FOV_Y};
use crate::geo::ray_ellipsoid;
use glam::{DMat4, DVec3, DVec4, Mat4};

pub const CASCADES: usize = 3;
/// Texels along one side of each cascade's map.
pub const MAP_SIZE: u32 = 2048;
/// Where the last cascade ends at most, on the Earth; no shadows are drawn from farther away.
const RANGE: f64 = 600_000.0;
/// The tallest thing that can cast a shadow: the highest mountains and a skyscraper on top.
const TALLEST: f64 = 9_500.0;
/// The farthest towards the Sun a box reaches, so a Sun at the horizon stays affordable.
const MAX_REACH: f64 = 300_000.0;
/// Reaches are worked out for a Sun at least this high (about 1 degree).
const MIN_SIN_ELEVATION: f64 = 0.0175;
/// Terrain may lie this far below the ellipsoid.
const BELOW: f64 = 500.0;
/// Each cascade is blended into the next over the last tenth of its depth; the next one therefore
/// starts this much earlier than the previous one ends.
pub const BLEND: f64 = 0.1;
/// The most one cascade's far depth may exceed the previous one's.
const MAX_RATIO: f64 = 6.0;

/// What the camera sees.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub basis: Basis,
    pub aspect: f64,
    /// Height of the ground at the view point; the rest of the ground is judged by it.
    pub ground_h: f64,
    /// The camera's near plane, the least depth a cascade starts at.
    pub near: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascade {
    /// From camera-relative metres to the map: x and y in -1..1, depth in 0..1 growing away from the Sun.
    pub matrix: Mat4,
    /// Half the width of the box, and the size of one texel, in metres.
    pub half: f64,
    pub texel: f64,
    /// The centre of the box along the first two light axes, in absolute coordinates; a whole
    /// number of texels each.
    pub snapped: [f64; 2],
    /// Metres from one side of the box to the other along the rays.
    pub depth_range: f64,
    /// The box relative to the camera along the light axes: the centre of x and y, and z from and to.
    centre: [f64; 2],
    z: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Light space: x, y, and the direction to the Sun.
    pub axes: [DVec3; 3],
    pub cascades: [Cascade; CASCADES],
    /// The view depth where each cascade stops being used.
    pub ends: [f64; CASCADES],
    /// 1 for full shadows, less while the whole view is far from the ground.
    pub strength: f64,
}

impl Cascade {
    /// Whether a sphere (centre relative to the camera) touches this cascade's box.
    fn touches(&self, axes: &[DVec3; 3], c: DVec3, r: f64) -> bool {
        (c.dot(axes[0]) - self.centre[0]).abs() <= self.half + r
            && (c.dot(axes[1]) - self.centre[1]).abs() <= self.half + r
            && c.dot(axes[2]) >= self.z[0] - r
            && c.dot(axes[2]) <= self.z[1] + r
    }
}

impl Plan {
    /// Whether anything inside the sphere could cast a shadow into some cascade.
    pub fn touches(&self, centre_rel: DVec3, radius: f64) -> bool {
        self.cascades.iter().any(|c| c.touches(&self.axes, centre_rel, radius))
    }

    pub fn cascade_touches(&self, i: usize, centre_rel: DVec3, radius: f64) -> bool {
        self.cascades[i].touches(&self.axes, centre_rel, radius)
    }
}

/// Light-space axes for a direction to the Sun: stable for a given Sun, and turning with it.
fn light_axes(sun: DVec3) -> [DVec3; 3] {
    let reference = if sun.z.abs() > 0.99 { DVec3::X } else { DVec3::Z };
    let x = reference.cross(sun).normalize();
    [x, sun.cross(x), sun]
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Rounds a half width up to the next step of a geometric series, so that small changes of the
/// view do not change the texel size and make edges shimmer.
fn quantise(x: f64) -> f64 {
    2f64.powf((x.log2() * 4.0).ceil() / 4.0)
}

/// The planned shadow maps for a view, or `None` when there is nothing to draw: no Sun, the
/// whole visible ground in the dark, the ground too far away, or only sky in view.
pub fn plan(input: &Input, sun: DVec3) -> Option<Plan> {
    if !sun.is_finite() || sun.length() < 0.5 {
        return None;
    }
    let sun = sun.normalize();
    let b = &input.basis;
    let t = (FOV_Y / 2.0).tan();
    let rays = [
        b.look,
        b.look + b.right * (t * input.aspect) + b.up * t,
        b.look - b.right * (t * input.aspect) + b.up * t,
        b.look + b.right * (t * input.aspect) - b.up * t,
        b.look - b.right * (t * input.aspect) - b.up * t,
    ]
    .map(|d| d.normalize());
    let range = crate::body::scaled(RANGE);
    let reach_cap = crate::body::scaled(MAX_REACH);

    // Where the rays meet the ground, and how far the ground goes along each.
    // The ground under the camera at the view point's height, and the tangent distance to its horizon.
    let (_, _, cam_h) = crate::geo::ecef_to_geodetic(b.pos);
    let ground_r = b.pos.length() - (cam_h - input.ground_h);
    let horizon = (b.pos.length_squared() - ground_r * ground_r).max(0.0).sqrt();
    let mut nearest = f64::INFINITY;
    let mut farthest: f64 = 0.0;
    let mut lit = false;
    for d in rays {
        let c = d.dot(b.look);
        match ray_ellipsoid(b.pos, d, input.ground_h) {
            Some(s) => {
                nearest = nearest.min(s * c);
                farthest = farthest.max(s * c);
                lit |= (b.pos + d * s).normalize().dot(sun) > 0.0;
            }
            None => farthest = farthest.max(horizon),
        }
    }
    if !nearest.is_finite() || !lit {
        return None;
    }
    let strength = 1.0 - smoothstep(0.5 * range, range, nearest);
    if strength <= 0.0 {
        return None;
    }
    // Terrain above the view point's height may be nearer than the ground found for it.
    let zn = (0.75 * nearest).max(input.near).max(0.5);
    let zf = (farthest * 1.1).min(range);
    if zf <= zn * 1.01 {
        return None;
    }

    let axes = light_axes(sun);
    // Equal ratios between the cascades keep the texels about as big as pixels at the far end of each.
    // Where that would stretch a cascade too far, the shadows stop sooner and fade out there.
    let ratio = (zf / zn).powf(1.0 / CASCADES as f64).clamp(1.2, MAX_RATIO);
    let ends = std::array::from_fn(|i| zn * ratio.powi(i as i32 + 1));
    let cascades = std::array::from_fn(|i| {
        let start = if i == 0 { zn } else { ends[i - 1] * (1.0 - BLEND) };
        fit(input, &rays, &axes, sun, start, ends[i], reach_cap)
    });
    Some(Plan { axes, cascades, ends, strength })
}

/// The box for the part of the view between two depths.
fn fit(input: &Input, rays: &[DVec3; 5], axes: &[DVec3; 3], sun: DVec3, d0: f64, d1: f64, reach_cap: f64) -> Cascade {
    let b = &input.basis;
    let corners: Vec<DVec3> = rays[1..]
        .iter()
        .flat_map(|d| {
            let c = d.dot(b.look);
            [d0, d1].map(|depth| b.pos + *d * (depth / c))
        })
        .collect();
    // A sphere round the slice centred on the view axis: it stays the same size however the
    // camera turns, which keeps the texel size steady.
    let t = (FOV_Y / 2.0).tan();
    let mid = (0.5 * (d0 + d1) * (1.0 + t * t * (1.0 + input.aspect * input.aspect))).min(d1);
    let centre = b.pos + b.look * mid;
    let r = corners.iter().map(|p| (*p - centre).length()).fold(0.0, f64::max);
    // Room for the snapping and the filter at the edge.
    let half = quantise(r * (1.0 + 16.0 / MAP_SIZE as f64));
    let texel = 2.0 * half / MAP_SIZE as f64;

    let snap = |v: f64| (v / texel).round() * texel;
    let snapped = [snap(centre.dot(axes[0])), snap(centre.dot(axes[1]))];
    let rel = [snapped[0] - b.pos.dot(axes[0]), snapped[1] - b.pos.dot(axes[1])];

    // Casters between the Sun and the ground: a point of height H lies H / sin(elevation) along
    // the ray from the ground point it shades.
    // The Sun is lowest, and the reach longest, where the slice curves away from it.
    let sin_el = corners
        .iter()
        .chain([&centre])
        .map(|p| p.normalize().dot(sun))
        .fold(f64::INFINITY, f64::min)
        .max(MIN_SIN_ELEVATION);
    let reach = (TALLEST / sin_el).min(reach_cap);
    let cz = (centre - b.pos).dot(axes[2]);
    let z = [cz - r - BELOW, cz + r + reach];
    let depth_range = z[1] - z[0];

    let row = |a: DVec3, shift: f64, scale: f64| DVec4::new(a.x * scale, a.y * scale, a.z * scale, shift * scale);
    let rows = [
        row(axes[0], -rel[0], 1.0 / half),
        row(axes[1], -rel[1], 1.0 / half),
        row(-axes[2], z[1], 1.0 / depth_range),
        DVec4::new(0.0, 0.0, 0.0, 1.0),
    ];
    let m = DMat4::from_cols(
        DVec4::new(rows[0].x, rows[1].x, rows[2].x, rows[3].x),
        DVec4::new(rows[0].y, rows[1].y, rows[2].y, rows[3].y),
        DVec4::new(rows[0].z, rows[1].z, rows[2].z, rows[3].z),
        DVec4::new(rows[0].w, rows[1].w, rows[2].w, rows[3].w),
    );
    Cascade {
        matrix: m.as_mat4(),
        half,
        texel,
        snapped,
        depth_range,
        centre: rel,
        z,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::geo::{enu, geodetic_to_ecef};
    use glam::{DVec2, Vec4};

    fn camera(lat_deg: f64, lon_deg: f64, distance: f64, tilt_deg: f64) -> Camera {
        Camera {
            lat: lat_deg.to_radians(),
            lon: lon_deg.to_radians(),
            target_h: 0.0,
            distance,
            heading: 0.3,
            tilt: tilt_deg.to_radians(),
            sky: 0.0,
        }
    }

    fn input(cam: &Camera) -> Input {
        Input { basis: cam.basis(), aspect: 1.6, ground_h: cam.target_h, near: cam.near_plane(cam.distance * 0.7) }
    }

    /// The Sun at an elevation and an azimuth (clockwise from north) over a place.
    fn sun_at(lat: f64, lon: f64, el: f64, az: f64) -> DVec3 {
        let (e, n, u) = enu(lat, lon);
        let h = n * az.cos() + e * az.sin();
        h * el.cos() + u * el.sin()
    }

    /// Where a point lands on the map, in metres from the box centre, and its depth 0..1.
    fn on_map(p: &Plan, i: usize, cam_pos: DVec3, world: DVec3) -> (f64, f64, f64) {
        let c = &p.cascades[i];
        let v = c.matrix * (world - cam_pos).as_vec3().extend(1.0);
        (v.x as f64 * c.half, v.y as f64 * c.half, v.z as f64)
    }

    fn a_plan(cam: &Camera, sun: DVec3) -> Plan {
        plan(&input(cam), sun).expect("a plan")
    }

    #[test]
    fn shadow_length_follows_height_over_tangent_of_elevation() {
        let cam = camera(46.0, 7.7, 3000.0, 40.0);
        let (lat, lon) = (cam.lat, cam.lon);
        for el_deg in [5.0f64, 15.0, 30.0, 60.0] {
            let el = el_deg.to_radians();
            let az = 2.4;
            let sun = sun_at(lat, lon, el, az);
            let p = a_plan(&cam, sun);
            let pos = cam.basis().pos;
            let foot = geodetic_to_ecef(lat, lon, 0.0);
            let (e, n, u) = enu(lat, lon);
            let toward_sun = n * az.cos() + e * az.sin();
            for h in [10.0, 100.0, 800.0] {
                let top = foot + u * h;
                let tip = foot - toward_sun * (h / el.tan());
                let a = on_map(&p, 1, pos, top);
                let b = on_map(&p, 1, pos, tip);
                assert!((a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02, "el {el_deg} h {h}: {a:?} {b:?}");
                // The tip lies farther from the Sun than the top does.
                assert!(b.2 > a.2 - 1e-6);
                // Further from the foot than the tip, the ground does not meet the same ray.
                let other = on_map(&p, 1, pos, foot - toward_sun * (h / el.tan() * 1.5));
                assert!((other.0 - a.0).hypot(other.1 - a.1) > 0.4 * h / el.tan() * el.sin());
            }
        }
    }

    #[test]
    fn at_noon_in_the_northern_midlatitudes_the_shadow_falls_north() {
        // Zurich, near local solar noon on the longest day: the highest the Sun gets that day.
        let (lat, lon) = (47.3769f64.to_radians(), 8.5417f64.to_radians());
        let day = crate::sky_time::parse_iso8601("2026-06-21T11:00:00Z").unwrap();
        let sun_of = |unix: f64| {
            let sm = crate::ephemeris::SunMoon::at(unix);
            (sm.to_earth_fixed() * sm.sun).normalize()
        };
        let (_, _, up) = enu(lat, lon);
        let noon = (0..120)
            .map(|m| day + m as f64 * 60.0)
            .max_by(|a, b| sun_of(*a).dot(up).total_cmp(&sun_of(*b).dot(up)))
            .unwrap();
        let sun = sun_of(noon);
        // About 66 degrees up.
        let el = sun.dot(up).asin().to_degrees();
        assert!((60.0..70.0).contains(&el), "elevation {el}");

        let cam = Camera { lat, lon, target_h: 0.0, distance: 2000.0, heading: 0.0, tilt: 0.5, sky: 0.0 };
        let p = a_plan(&cam, sun);
        let pos = cam.basis().pos;
        let foot = geodetic_to_ecef(lat, lon, 0.0);
        let (_, north, _) = enu(lat, lon);
        let height = 100.0;
        let top = on_map(&p, 1, pos, foot + up * height);
        let d = height / sun.dot(up).asin().tan();
        let to_north = on_map(&p, 1, pos, foot + north * d);
        let to_south = on_map(&p, 1, pos, foot - north * d);
        let miss = |q: (f64, f64, f64)| (q.0 - top.0).hypot(q.1 - top.1);
        assert!(miss(to_north) < 0.5 * d, "north is {} off, south {}", miss(to_north), miss(to_south));
        assert!(miss(to_south) > 1.5 * d);
    }

    #[test]
    fn texels_stay_on_the_same_grid_when_the_camera_moves() {
        let cam = camera(46.5, 8.0, 5000.0, 55.0);
        let sun = sun_at(cam.lat, cam.lon, 0.4, 2.0);
        let base = a_plan(&cam, sun);
        let (_, north, _) = enu(cam.lat, cam.lon);
        for step in [0.13, 0.31, 0.77, 2.4, 9.1] {
            let mut moved = cam;
            let shift = north * step;
            let target = cam.target() + shift;
            let (lat, lon, _) = crate::geo::ecef_to_geodetic(target);
            moved.lat = lat;
            moved.lon = lon;
            let p = a_plan(&moved, sun);
            for i in 0..CASCADES {
                let (a, b) = (&base.cascades[i], &p.cascades[i]);
                // The size is unchanged by a small move, and the box centre is a whole number of texels.
                assert_eq!(a.half, b.half, "cascade {i} step {step}");
                for k in 0..2 {
                    let cells = b.snapped[k] / b.texel;
                    assert!((cells - cells.round()).abs() < 1e-6, "cascade {i} axis {k}: {cells}");
                    // The relative offset in the matrix agrees with the snapped centre.
                    let rel = b.snapped[k] - p.axes[k].dot(moved.basis().pos);
                    let from_matrix = -(b.matrix * Vec4::new(0.0, 0.0, 0.0, 1.0))[k] as f64 * b.half;
                    assert!((rel - from_matrix).abs() < 0.01 * b.texel + 1e-3, "{rel} {from_matrix}");
                }
            }
        }
    }

    /// Ground points seen through a grid of screen positions.
    fn visible_ground(cam: &Camera, aspect: f64) -> Vec<(DVec3, f64)> {
        let b = cam.basis();
        let mut out = Vec::new();
        for i in 0..=10 {
            for j in 0..=10 {
                let ndc = DVec2::new(i as f64 / 5.0 - 1.0, j as f64 / 5.0 - 1.0);
                let dir = cam.ray(ndc, aspect);
                if let Some(s) = ray_ellipsoid(b.pos, dir, 0.0) {
                    out.push((b.pos + dir * s, s * dir.dot(b.look)));
                }
            }
        }
        out
    }

    #[test]
    fn boxes_hold_the_visible_ground_and_casters_nine_kilometres_up() {
        let mut checked = 0;
        for distance in [300.0, 2000.0, 15_000.0, 120_000.0] {
            for tilt in [0.0, 30.0, 60.0, 78.0] {
                for el_deg in [4.0f64, 12.0, 40.0] {
                    let cam = camera(46.0, 7.7, distance, tilt);
                    let sun = sun_at(cam.lat, cam.lon, el_deg.to_radians(), 4.0);
                    let inp = input(&cam);
                    let Some(p) = plan(&inp, sun) else { continue };
                    let pos = inp.basis.pos;
                    for (g, depth) in visible_ground(&cam, inp.aspect) {
                        if depth > p.ends[CASCADES - 1] {
                            continue;
                        }
                        let i = p.ends.iter().position(|e| depth <= *e).unwrap();
                        let (x, y, z) = on_map(&p, i, pos, g);
                        let half = p.cascades[i].half;
                        assert!(x.abs() < half && y.abs() < half, "ground x {x} y {y} half {half} d {distance} t {tilt}");
                        assert!((0.0..1.0).contains(&z), "ground z {z}");
                        // A caster over that point, 9 km above the ground along the Sun's ray.
                        // Found by bisection, because the ground curves away below the ray.
                        let (mut lo, mut hi) = (0.0, 2.0e6);
                        for _ in 0..60 {
                            let mid = 0.5 * (lo + hi);
                            if (g + sun * mid).length() - g.length() < 9000.0 { lo = mid } else { hi = mid }
                        }
                        let caster = g + sun * lo;
                        let (cx, cy, cz) = on_map(&p, i, pos, caster);
                        assert!((cx - x).abs() < 0.05 && (cy - y).abs() < 0.05);
                        assert!((0.0..1.0).contains(&cz), "caster z {cz} el {el_deg} d {distance}");
                        assert!(cz < z);
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 1000, "only {checked} points checked");
    }

    #[test]
    fn nearer_cascades_have_finer_texels() {
        let cam = camera(40.7, -74.0, 1500.0, 60.0);
        let p = a_plan(&cam, sun_at(cam.lat, cam.lon, 0.6, 3.0));
        assert!(p.cascades[0].texel < p.cascades[1].texel && p.cascades[1].texel < p.cascades[2].texel);
        assert!(p.cascades[0].texel < 2.0, "{}", p.cascades[0].texel);
        assert!(p.ends[0] < p.ends[1] && p.ends[1] < p.ends[2]);
    }

    #[test]
    fn no_plan_without_light_or_near_the_ground() {
        let cam = camera(46.0, 7.7, 5000.0, 50.0);
        let (lat, lon) = (cam.lat, cam.lon);
        assert!(plan(&input(&cam), sun_at(lat, lon, 0.5, 1.0)).is_some());
        // The Sun well below the horizon of everything in view.
        assert!(plan(&input(&cam), sun_at(lat, lon, -0.5, 1.0)).is_none());
        // No Sun at all.
        assert!(plan(&input(&cam), DVec3::ZERO).is_none());
        // The whole Earth in view.
        let far = camera(0.0, 0.0, 4.0e7, 0.0);
        assert!(plan(&input(&far), sun_at(far.lat, far.lon, 1.0, 0.0)).is_none());
        // A long way out the shadows fade, and farther still they are gone.
        let mid = camera(46.0, 8.0, 300_000.0, 0.0);
        let s = a_plan(&mid, sun_at(mid.lat, mid.lon, 0.5, 1.0)).strength;
        assert!(s > 0.0 && s <= 1.0);
        let out = camera(46.0, 8.0, 1_500_000.0, 0.0);
        assert!(plan(&input(&out), sun_at(out.lat, out.lon, 0.5, 1.0)).is_none());
        let low = camera(46.0, 8.0, 3000.0, 20.0);
        assert_eq!(a_plan(&low, sun_at(low.lat, low.lon, 0.5, 1.0)).strength, 1.0);
    }

    #[test]
    fn looking_at_the_sky_gives_no_plan() {
        let mut cam = camera(46.0, 7.7, 2.0, 0.0);
        cam.tilt = 85f64.to_radians();
        cam.sky = 60f64.to_radians();
        assert!(plan(&input(&cam), sun_at(cam.lat, cam.lon, 0.8, 1.0)).is_none());
    }

    #[test]
    fn spheres_are_tested_against_the_boxes() {
        let cam = camera(46.0, 7.7, 5000.0, 50.0);
        let sun = sun_at(cam.lat, cam.lon, 0.3, 2.0);
        let p = a_plan(&cam, sun);
        let pos = cam.basis().pos;
        let ground = cam.target() - pos;
        assert!(p.touches(ground, 100.0));
        // Towards the Sun, high above the view: a mountain shading the ground.
        assert!(p.touches(ground + sun * 20_000.0, 500.0));
        // Far to the side, or behind the ground away from the Sun.
        assert!(!p.touches(ground + p.axes[0] * 5.0e6, 500.0));
        assert!(!p.touches(ground - sun * 5.0e6, 500.0));
        // Beyond the reach towards the Sun.
        assert!(!p.touches(ground + sun * 2.0e6, 500.0));
    }
}
