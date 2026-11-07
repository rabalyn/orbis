//! The camera: a view point on the ground plus distance, heading and tilt.

use crate::geo::{ecef_to_geodetic, enu, geodetic_to_ecef, ray_ellipsoid};
use glam::{DQuat, DVec2, DVec3, Mat4};
use std::f64::consts::{PI, TAU};

pub const FOV_Y: f64 = 45.0 * PI / 180.0;
pub const MIN_DISTANCE: f64 = 30.0;
pub const MAX_DISTANCE: f64 = 4.0e7;
pub const MAX_TILT: f64 = 85.0 * PI / 180.0;
const MAX_LAT: f64 = 89.9 * PI / 180.0;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub lat: f64,
    pub lon: f64,
    /// Height of the view point above the ellipsoid.
    pub target_h: f64,
    pub distance: f64,
    /// 0 is north up, growing clockwise.
    pub heading: f64,
    /// 0 is straight down, growing towards the horizon.
    pub tilt: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Basis {
    pub pos: DVec3,
    pub right: DVec3,
    pub up: DVec3,
    pub look: DVec3,
}

impl Camera {
    pub fn target(&self) -> DVec3 {
        geodetic_to_ecef(self.lat, self.lon, self.target_h)
    }

    /// The ground direction the view faces.
    pub fn ground_forward(&self) -> DVec3 {
        let (e, n, _) = enu(self.lat, self.lon);
        n * self.heading.cos() + e * self.heading.sin()
    }

    pub fn basis(&self) -> Basis {
        let (_, _, up) = enu(self.lat, self.lon);
        let f = self.ground_forward();
        let (st, ct) = self.tilt.sin_cos();
        let offset = up * ct - f * st;
        let look = -offset;
        let cam_up = f * ct + up * st;
        Basis {
            pos: self.target() + offset * self.distance,
            right: look.cross(cam_up),
            up: cam_up,
            look,
        }
    }

    pub fn ray(&self, ndc: DVec2, aspect: f64) -> DVec3 {
        let b = self.basis();
        let t = (FOV_Y / 2.0).tan();
        (b.look + b.right * (ndc.x * t * aspect) + b.up * (ndc.y * t)).normalize()
    }

    /// The ground point under a screen position, at the view point's height.
    pub fn pick(&self, ndc: DVec2, aspect: f64) -> Option<DVec3> {
        let b = self.basis();
        let dir = self.ray(ndc, aspect);
        ray_ellipsoid(b.pos, dir, self.target_h).map(|t| b.pos + dir * t)
    }

    /// Rotates the view rigidly about the Earth's centre.
    pub fn apply_rotation(&mut self, q: DQuat) {
        let target = q * self.target();
        let forward = q * self.ground_forward();
        let (lat, lon, _) = ecef_to_geodetic(target);
        self.lat = lat.clamp(-MAX_LAT, MAX_LAT);
        self.lon = lon;
        let (e, n, _) = enu(self.lat, self.lon);
        self.heading = forward.dot(e).atan2(forward.dot(n));
    }

    /// Re-expresses the view as a camera position and look direction. Returns false and
    /// leaves the camera unchanged when that direction misses the ground or is too flat.
    pub fn set_pose(&mut self, pos: DVec3, look: DVec3) -> bool {
        let Some(t) = ray_ellipsoid(pos, look, self.target_h) else {
            return false;
        };
        let (lat, lon, _) = ecef_to_geodetic(pos + look * t);
        let (e, n, up) = enu(lat, lon);
        let tilt = (-look.dot(up)).clamp(-1.0, 1.0).acos();
        if tilt > MAX_TILT || lat.abs() > MAX_LAT {
            return false;
        }
        self.lat = lat;
        self.lon = lon;
        self.distance = t;
        self.tilt = tilt;
        let flat = look - up * look.dot(up);
        if flat.length() > 1e-6 {
            self.heading = flat.dot(e).atan2(flat.dot(n));
        }
        true
    }

    /// Looks around from a fixed position: `yaw` to the right, `pitch` upwards.
    pub fn look_around(&mut self, yaw: f64, pitch: f64) -> bool {
        let b = self.basis();
        let (lat, lon, _) = ecef_to_geodetic(b.pos);
        let (_, _, up) = enu(lat, lon);
        let look = DQuat::from_axis_angle(b.right, pitch) * b.look;
        let look = DQuat::from_axis_angle(up, -yaw) * look;
        self.set_pose(b.pos, look.normalize())
    }

    pub fn clamp(&mut self) {
        self.distance = self.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.tilt = self.tilt.clamp(0.0, MAX_TILT);
        self.lat = self.lat.clamp(-MAX_LAT, MAX_LAT);
        self.heading = wrap(self.heading);
        self.lon = wrap(self.lon);
    }

    /// A camera-at-origin projection, with reversed infinite depth.
    pub fn view_proj(&self, aspect: f64, near: f64) -> Mat4 {
        let b = self.basis();
        let (r, u, f) = (b.right, b.up, -b.look);
        let view = Mat4::from_cols(
            glam::Vec4::new(r.x as f32, u.x as f32, f.x as f32, 0.0),
            glam::Vec4::new(r.y as f32, u.y as f32, f.y as f32, 0.0),
            glam::Vec4::new(r.z as f32, u.z as f32, f.z as f32, 0.0),
            glam::Vec4::W,
        );
        Mat4::perspective_infinite_reverse_rh(FOV_Y as f32, aspect as f32, near as f32) * view
    }

    /// A near plane suited to the height above the ground.
    pub fn near_plane(&self, altitude_above_ground: f64) -> f64 {
        (altitude_above_ground * 0.1).clamp(0.5, 100_000.0)
    }
}

/// Wraps an angle to -PI..PI.
pub fn wrap(a: f64) -> f64 {
    if a > -PI && a <= PI {
        return a;
    }
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI { w + TAU } else { w }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera { lat: 0.8, lon: 0.2, target_h: 0.0, distance: 50_000.0, heading: 0.7, tilt: 0.6 }
    }

    #[test]
    fn straight_down_north_up_basis() {
        let c = Camera { heading: 0.0, tilt: 0.0, ..cam() };
        let b = c.basis();
        let (e, n, up) = enu(c.lat, c.lon);
        assert!((b.right - e).length() < 1e-9);
        assert!((b.up - n).length() < 1e-9);
        assert!((b.look + up).length() < 1e-9);
    }

    #[test]
    fn centre_of_screen_picks_the_view_point() {
        let c = cam();
        let p = c.pick(DVec2::ZERO, 1.5).unwrap();
        assert!((p - c.target()).length() < 1e-3);
    }

    #[test]
    fn pose_round_trip() {
        let c = cam();
        let b = c.basis();
        let mut d = Camera { lat: 0.0, lon: 0.0, distance: 1.0, heading: 0.0, tilt: 0.0, ..c };
        assert!(d.set_pose(b.pos, b.look));
        assert!((d.lat - c.lat).abs() < 1e-9);
        assert!((d.lon - c.lon).abs() < 1e-9);
        assert!((d.distance - c.distance).abs() < 1e-3);
        assert!((d.heading - c.heading).abs() < 1e-9);
        assert!((d.tilt - c.tilt).abs() < 1e-9);
    }

    #[test]
    fn looking_around_keeps_the_position() {
        let mut c = cam();
        let before = c.basis().pos;
        assert!(c.look_around(0.1, 0.0));
        assert!((c.basis().pos - before).length() < 1e-3);
    }

    #[test]
    fn set_pose_rejects_a_look_into_space() {
        let mut c = cam();
        let copy = c;
        let b = c.basis();
        assert!(!c.set_pose(b.pos, -b.look));
        assert_eq!(c.lat, copy.lat);
    }

    #[test]
    fn rotation_moves_view_rigidly() {
        let mut c = Camera { lat: 0.3, lon: 0.0, ..cam() };
        let before = c.basis();
        let q = DQuat::from_rotation_z(0.25);
        c.apply_rotation(q);
        let after = c.basis();
        assert!((after.pos - q * before.pos).length() < 1e-3);
        assert!((after.look - q * before.look).length() < 1e-9);
    }

    #[test]
    fn projection_sends_the_view_point_to_the_centre() {
        let c = cam();
        let b = c.basis();
        let rel = (c.target() - b.pos).as_vec3();
        let clip = c.view_proj(1.5, 10.0) * rel.extend(1.0);
        let ndc = clip / clip.w;
        assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4);
        assert!(ndc.z > 0.0 && ndc.z < 1.0);
    }

    #[test]
    fn wrap_range() {
        assert!((wrap(3.0 * PI) - PI).abs() < 1e-12);
        assert!((wrap(-3.5 * PI) - 0.5 * PI).abs() < 1e-12);
    }
}
