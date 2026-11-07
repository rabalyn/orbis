//! Turns input into camera movement. Independent of the window system.

use crate::camera::{max_distance, wrap, Camera, MAX_TILT};
use crate::geo::{ecef_to_geodetic, enu, radius};
use glam::{DQuat, DVec2, DVec3};
use std::f64::consts::PI;

pub const WHOLE_EARTH: f64 = 2.2e7;

/// The distance at which the whole body fits the view.
pub fn whole_body() -> f64 {
    crate::body::scaled(WHOLE_EARTH)
}
const GROUND_CLEARANCE: f64 = 2.0;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArrowMode {
    Fly,
    Orbit,
    Height,
}

#[derive(Clone, Copy)]
struct HeldArrow {
    arrow: Arrow,
    mode: ArrowMode,
    held: f64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum DragMode {
    Grab,
    Orbit,
    Height,
}

#[derive(Clone, Copy)]
struct Drag {
    mode: DragMode,
    anchor: Option<DVec3>,
    last: DVec2,
}

#[derive(Clone, Copy)]
struct Flight {
    from_dir: DVec3,
    arc: DQuat,
    from: Camera,
    to: Camera,
    cruise: f64,
    t: f64,
    duration: f64,
}

pub struct Nav {
    pub cam: Camera,
    pub size: DVec2,
    drag: Option<Drag>,
    /// Glide after a drag, as axis times radians per second.
    glide: DVec3,
    zoom_pending: f64,
    zoom_at: DVec2,
    arrows: Vec<HeldArrow>,
    flight: Option<Flight>,
    since_move: f64,
    /// Shaped SpaceMouse axes, held until the device reports new ones, and their scale.
    device: crate::spacemouse::Axes,
    device_sensitivity: f64,
    pub altitude: f64,
    /// While true, the view stands the chosen distance from the ground, however the ground's
    /// height turns out as finer terrain arrives. Later, the camera holds its height instead.
    starting: bool,
}

impl Nav {
    pub fn new(cam: Camera, size: DVec2) -> Self {
        Self {
            cam,
            size,
            drag: None,
            glide: DVec3::ZERO,
            zoom_pending: 0.0,
            zoom_at: DVec2::ZERO,
            arrows: Vec::new(),
            flight: None,
            since_move: 0.0,
            device: [0.0; 6],
            device_sensitivity: 1.0,
            altitude: cam.distance,
            starting: false,
        }
    }

    /// A view whose distance counts from the terrain, once that is known.
    pub fn starting_above_ground(cam: Camera, size: DVec2) -> Self {
        Self { starting: true, ..Self::new(cam, size) }
    }

    /// Ends the start phase; from here on the camera holds its height over changing ground.
    pub fn finish_start(&mut self) {
        self.starting = false;
    }

    fn aspect(&self) -> f64 {
        self.size.x / self.size.y.max(1.0)
    }

    fn ndc(&self, p: DVec2) -> DVec2 {
        DVec2::new(p.x / self.size.x * 2.0 - 1.0, 1.0 - p.y / self.size.y * 2.0)
    }

    /// Moves the view so that `anchor` lies under the screen position. Returns the rotation.
    fn hold_anchor(&mut self, anchor: DVec3, ndc: DVec2) -> Option<DQuat> {
        let mut total = DQuat::IDENTITY;
        let aspect = self.aspect();
        for _ in 0..3 {
            let p = self.cam.pick(ndc, aspect)?;
            let q = DQuat::from_rotation_arc(p.normalize(), anchor.normalize());
            self.cam.apply_rotation(q);
            total = q * total;
        }
        Some(total)
    }

    fn end_automatic_motion(&mut self) {
        self.starting = false;
        self.flight = None;
        self.glide = DVec3::ZERO;
    }

    /// True while something still changes without further input.
    pub fn is_animating(&self) -> bool {
        self.flight.is_some()
            || self.glide != DVec3::ZERO
            || self.zoom_pending != 0.0
            || !self.arrows.is_empty()
            || !crate::spacemouse_nav::is_at_rest(&self.device)
    }

    pub fn press(&mut self, button: Button, pos: DVec2, mods: Mods) {
        self.end_automatic_motion();
        self.zoom_pending = 0.0;
        let mode = match (button, mods.alt, mods.shift) {
            (Button::Right, ..) | (Button::Left, true, _) => DragMode::Orbit,
            (Button::Left, false, true) => DragMode::Height,
            (Button::Left, false, false) => DragMode::Grab,
        };
        let anchor = (mode == DragMode::Grab)
            .then(|| self.cam.pick(self.ndc(pos), self.aspect()))
            .flatten();
        self.drag = Some(Drag { mode, anchor, last: pos });
    }

    pub fn moved(&mut self, pos: DVec2) {
        let Some(mut drag) = self.drag else { return };
        let delta = pos - drag.last;
        drag.last = pos;
        match drag.mode {
            DragMode::Grab => {
                let ndc = self.ndc(pos);
                match drag.anchor {
                    Some(anchor) => {
                        if let Some(q) = self.hold_anchor(anchor, ndc) {
                            // Smoothed estimate of how fast the hand moves, for the glide.
                            let (axis, angle) = q.to_axis_angle();
                            let dt = self.since_move.max(1.0 / 240.0);
                            let velocity = axis * (angle / dt);
                            self.glide = self.glide.lerp(velocity, 0.5);
                            self.since_move = 0.0;
                        }
                    }
                    // The press missed the globe; start holding once the pointer reaches it.
                    None => drag.anchor = self.cam.pick(ndc, self.aspect()),
                }
            }
            DragMode::Orbit => {
                self.cam.heading -= delta.x * 0.005;
                self.cam.add_tilt(delta.y * 0.005);
            }
            DragMode::Height => {
                self.cam.distance *= (-delta.y * 0.005).exp();
                self.cam.look_around(delta.x * 0.003, 0.0);
            }
        }
        self.drag = Some(drag);
        self.cam.clamp();
    }

    pub fn release(&mut self) {
        let was_grab = self.drag.is_some_and(|d| d.mode == DragMode::Grab);
        self.drag = None;
        // A hand that came to rest before letting go throws nothing.
        if !was_grab || self.since_move > 0.1 {
            self.glide = DVec3::ZERO;
        }
    }

    /// `lines` is positive for scrolling away from the user, which zooms in.
    pub fn scroll(&mut self, lines: f64, pos: DVec2, mods: Mods) {
        self.end_automatic_motion();
        if mods.alt {
            self.cam.add_tilt(-lines * 0.05);
        } else if mods.shift {
            self.cam.look_around(lines * 0.05, 0.0);
        } else {
            self.zoom_pending += lines * 0.25;
            self.zoom_at = pos;
        }
        self.cam.clamp();
    }

    pub fn double_click(&mut self, pos: DVec2, mods: Mods) {
        let Some(p) = self.cam.pick(self.ndc(pos), self.aspect()) else {
            return;
        };
        let (lat, lon, _) = ecef_to_geodetic(p);
        let factor = if mods.shift { 2.0 } else { 0.5 };
        let to = Camera { lat, lon, distance: self.cam.distance * factor, ..self.cam };
        self.fly_to(to);
    }

    pub fn arrow_down(&mut self, arrow: Arrow, mods: Mods) {
        if mods.ctrl {
            match arrow {
                Arrow::Up => self.level(0.5),
                Arrow::Down => self.level(2.0),
                _ => {}
            }
            return;
        }
        if self.arrows.iter().any(|a| a.arrow == arrow) {
            return;
        }
        self.end_automatic_motion();
        let mode = if mods.alt {
            ArrowMode::Orbit
        } else if mods.shift {
            ArrowMode::Height
        } else {
            ArrowMode::Fly
        };
        self.arrows.push(HeldArrow { arrow, mode, held: 0.0 });
    }

    /// Takes the latest SpaceMouse axes. A moving device ends a flight, as a hand does.
    pub fn set_device(&mut self, axes: crate::spacemouse::Axes, sensitivity: f64) {
        self.device = axes;
        self.device_sensitivity = sensitivity;
        if !crate::spacemouse_nav::is_at_rest(&axes) {
            self.end_automatic_motion();
            self.zoom_pending = 0.0;
        }
    }

    pub fn device(&self) -> (crate::spacemouse::Axes, f64) {
        (self.device, self.device_sensitivity)
    }

    fn step_device(&mut self, dt: f64) {
        if crate::spacemouse_nav::is_at_rest(&self.device) {
            return;
        }
        let s = crate::spacemouse_nav::globe_step(&self.device, self.device_sensitivity, dt);
        let f = self.cam.ground_forward();
        let (_, _, up) = enu(self.cam.lat, self.cam.lon);
        let dir = f * s.forward + f.cross(up) * s.right;
        let len = dir.length();
        if len > 0.0 {
            let angle = self.cam.distance * len / radius();
            self.cam.apply_rotation(DQuat::from_axis_angle(up.cross(dir / len).normalize(), angle));
        }
        self.cam.distance *= s.climb.exp();
        self.cam.heading += s.turn;
        self.cam.add_tilt(s.tilt);
    }

    pub fn arrow_up(&mut self, arrow: Arrow) {
        self.arrows.retain(|a| a.arrow != arrow);
    }

    pub fn release_all(&mut self) {
        self.arrows.clear();
        self.release();
    }

    /// Stops a flight or a gliding movement.
    pub fn stop(&mut self) {
        self.end_automatic_motion();
        self.zoom_pending = 0.0;
    }

    fn level(&mut self, factor: f64) {
        let to = Camera { distance: self.cam.distance * factor, ..self.cam };
        self.fly_to(to);
    }

    pub fn whole_body(&mut self) {
        let to = Camera { distance: whole_body(), tilt: 0.0, sky: 0.0, ..self.cam };
        self.fly_to(to);
    }

    /// One step back towards a familiar view; what is already right is skipped.
    pub fn straighten(&mut self) {
        const EPS: f64 = 1e-3;
        let c = self.cam;
        let to = if wrap(c.heading).abs() > EPS {
            Camera { heading: 0.0, ..c }
        } else if c.total_tilt() > EPS {
            Camera { tilt: 0.0, sky: 0.0, ..c }
        } else if c.distance < whole_body() * 0.99 {
            Camera { distance: whole_body(), ..c }
        } else {
            Camera { lat: 0.0, ..c }
        };
        self.fly_to(to);
    }

    pub fn fly_to(&mut self, mut to: Camera) {
        to.clamp();
        self.starting = false;
        self.drag = None;
        self.glide = DVec3::ZERO;
        self.zoom_pending = 0.0;
        let from = self.cam;
        let (a, b) = (from.target().normalize(), to.target().normalize());
        let arc = DQuat::from_rotation_arc(a, b);
        let angle = a.dot(b).clamp(-1.0, 1.0).acos();
        // Longer trips climb first, travel high and then sink.
        let cruise = (angle * radius() * 0.8).min(crate::body::scaled(1.5e7));
        let duration = (0.6 + angle * 1.5).min(4.0);
        self.flight = Some(Flight { from_dir: a, arc, from, to, cruise, t: 0.0, duration });
    }

    fn step_flight(&mut self, dt: f64) {
        let Some(mut f) = self.flight else { return };
        f.t = (f.t + dt / f.duration).min(1.0);
        let s = f.t * f.t * (3.0 - 2.0 * f.t);
        let dir = DQuat::IDENTITY.slerp(f.arc, s) * f.from_dir;
        let (lat, lon, _) = ecef_to_geodetic(dir * radius());
        let log_d = f.from.distance.ln() * (1.0 - s) + f.to.distance.ln() * s;
        let bump = (f.cruise - f.from.distance.max(f.to.distance)).max(0.0) * (PI * s).sin();
        self.cam.lat = lat;
        self.cam.lon = lon;
        self.cam.distance = (log_d.exp() + bump).min(max_distance());
        self.cam.heading = f.from.heading + wrap(f.to.heading - f.from.heading) * s;
        self.cam.set_total_tilt(f.from.total_tilt() + (f.to.total_tilt() - f.from.total_tilt()) * s);
        if f.t >= 1.0 {
            self.cam.lat = f.to.lat;
            self.cam.lon = f.to.lon;
            self.cam.distance = f.to.distance;
            self.cam.heading = f.to.heading;
            self.cam.tilt = f.to.tilt;
            self.cam.sky = f.to.sky;
            self.flight = None;
        } else {
            self.flight = Some(f);
        }
    }

    fn step_arrows(&mut self, dt: f64) {
        for i in 0..self.arrows.len() {
            self.arrows[i].held += dt;
            let HeldArrow { arrow, mode, held } = self.arrows[i];
            // A short press is a small step; held longer it gets faster.
            let rate = (0.25 + held * 1.5).min(3.0);
            match mode {
                ArrowMode::Fly => {
                    let f = self.cam.ground_forward();
                    let (_, _, up) = enu(self.cam.lat, self.cam.lon);
                    let dir = match arrow {
                        Arrow::Up => f,
                        Arrow::Down => -f,
                        Arrow::Right => f.cross(up),
                        Arrow::Left => up.cross(f),
                    };
                    let angle = self.cam.distance * rate * 0.6 * dt / radius();
                    self.cam.apply_rotation(DQuat::from_axis_angle(up.cross(dir).normalize(), angle));
                }
                ArrowMode::Orbit => match arrow {
                    Arrow::Up => self.cam.add_tilt(rate * dt * 0.5),
                    Arrow::Down => self.cam.add_tilt(-rate * dt * 0.5),
                    Arrow::Left => self.cam.heading -= rate * dt * 0.5,
                    Arrow::Right => self.cam.heading += rate * dt * 0.5,
                },
                ArrowMode::Height => match arrow {
                    Arrow::Up => self.cam.distance *= (rate * dt).exp(),
                    Arrow::Down => self.cam.distance *= (-rate * dt).exp(),
                    Arrow::Left => {
                        self.cam.look_around(-rate * dt * 0.5, 0.0);
                    }
                    Arrow::Right => {
                        self.cam.look_around(rate * dt * 0.5, 0.0);
                    }
                },
            }
        }
    }

    fn step_zoom(&mut self, dt: f64) {
        if self.zoom_pending == 0.0 {
            return;
        }
        let step = self.zoom_pending * (1.0 - (-dt * 14.0).exp());
        self.zoom_pending -= step;
        if self.zoom_pending.abs() < 1e-3 {
            self.zoom_pending = 0.0;
        }
        let ndc = self.ndc(self.zoom_at);
        let anchor = self.cam.pick(ndc, self.aspect());
        self.cam.distance *= (-step).exp();
        self.cam.clamp();
        if let Some(anchor) = anchor {
            self.hold_anchor(anchor, ndc);
        }
    }

    fn step_glide(&mut self, dt: f64) {
        if self.drag.is_some() || self.glide == DVec3::ZERO {
            return;
        }
        let speed = self.glide.length();
        if speed * radius() < 1.0 || speed < 1e-5 {
            self.glide = DVec3::ZERO;
            return;
        }
        self.cam.apply_rotation(DQuat::from_axis_angle(self.glide / speed, speed * dt));
        self.glide *= (-dt * 3.0).exp();
    }

    /// Advances all motion. `ground` gives the terrain height at a position, when known.
    pub fn tick(&mut self, dt: f64, ground: &dyn Fn(f64, f64) -> Option<f64>) {
        self.since_move += dt;
        self.step_flight(dt);
        self.step_arrows(dt);
        self.step_device(dt);
        self.step_zoom(dt);
        self.step_glide(dt);
        self.cam.clamp();
        self.follow_ground(dt, ground);
    }

    fn follow_ground(&mut self, dt: f64, ground: &dyn Fn(f64, f64) -> Option<f64>) {
        // The view point sits on the terrain, while the camera keeps its height.
        if let Some(h) = ground(self.cam.lat, self.cam.lon).filter(|_| self.starting) {
            self.cam.target_h = h;
        } else if let Some(h) = ground(self.cam.lat, self.cam.lon) {
            let change = (h - self.cam.target_h) * (1.0 - (-dt * 6.0).exp());
            self.cam.target_h += change;
            self.cam.distance -= change * self.cam.tilt.cos();
        }
        let (lat, lon, h) = ecef_to_geodetic(self.cam.basis().pos);
        let terrain = ground(lat, lon).unwrap_or(0.0);
        let gap = h - terrain - GROUND_CLEARANCE;
        if gap < 0.0 {
            self.cam.distance -= gap / self.cam.tilt.cos().max(MAX_TILT.cos());
        }
        self.cam.clamp();
        self.altitude = (h - terrain).max(GROUND_CLEARANCE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: DVec2 = DVec2::new(1200.0, 800.0);
    const NO_GROUND: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| None;

    fn nav() -> Nav {
        let cam = Camera { lat: 0.82, lon: 0.2, target_h: 0.0, distance: 200_000.0, heading: 0.3, tilt: 0.4, sky: 0.0 };
        Nav::new(cam, SIZE)
    }

    fn run(n: &mut Nav, seconds: f64) {
        for _ in 0..(seconds * 60.0) as usize {
            n.tick(1.0 / 60.0, NO_GROUND);
        }
    }

    #[test]
    fn dragged_point_stays_under_the_pointer() {
        let mut n = nav();
        let start = DVec2::new(600.0, 400.0);
        let end = DVec2::new(800.0, 300.0);
        let grabbed = n.cam.pick(n.ndc(start), n.aspect()).unwrap();
        n.press(Button::Left, start, Mods::default());
        n.moved(DVec2::new(700.0, 350.0));
        n.moved(end);
        let now = n.cam.pick(n.ndc(end), n.aspect()).unwrap();
        assert!((now - grabbed).length() < 2.0, "off by {} m", (now - grabbed).length());
    }

    #[test]
    fn drag_follows_the_hand() {
        let mut n = Nav::new(Camera { heading: 0.0, tilt: 0.0, ..nav().cam }, SIZE);
        let lon = n.cam.lon;
        n.press(Button::Left, DVec2::new(600.0, 400.0), Mods::default());
        n.moved(DVec2::new(700.0, 400.0));
        // Dragging the ground to the right brings the view further west.
        assert!(n.cam.lon < lon);
    }

    #[test]
    fn glide_needs_a_moving_hand_and_ends() {
        let mut n = nav();
        n.press(Button::Left, DVec2::new(600.0, 400.0), Mods::default());
        n.tick(1.0 / 60.0, NO_GROUND);
        n.moved(DVec2::new(640.0, 400.0));
        n.tick(1.0 / 60.0, NO_GROUND);
        n.release();
        let lon = n.cam.lon;
        n.tick(1.0 / 60.0, NO_GROUND);
        assert!(n.cam.lon != lon, "keeps gliding after a throw");
        run(&mut n, 10.0);
        assert!(!n.is_animating(), "glide runs out");

        let mut n = nav();
        n.press(Button::Left, DVec2::new(600.0, 400.0), Mods::default());
        n.tick(1.0 / 60.0, NO_GROUND);
        n.moved(DVec2::new(640.0, 400.0));
        run(&mut n, 0.5);
        n.release();
        assert!(!n.is_animating(), "a hand at rest throws nothing");
    }

    #[test]
    fn scroll_zooms_around_the_pointer() {
        let mut n = nav();
        let pos = DVec2::new(900.0, 250.0);
        let before = n.cam.pick(n.ndc(pos), n.aspect()).unwrap();
        let d = n.cam.distance;
        n.scroll(3.0, pos, Mods::default());
        run(&mut n, 1.0);
        assert!(n.cam.distance < d);
        let after = n.cam.pick(n.ndc(pos), n.aspect()).unwrap();
        assert!((after - before).length() < 5.0, "off by {} m", (after - before).length());
        assert!(!n.is_animating());
    }

    #[test]
    fn scroll_back_returns_to_the_same_distance() {
        let mut n = nav();
        let d = n.cam.distance;
        let pos = DVec2::new(600.0, 400.0);
        n.scroll(2.0, pos, Mods::default());
        run(&mut n, 1.0);
        n.scroll(-2.0, pos, Mods::default());
        run(&mut n, 1.0);
        assert!((n.cam.distance / d - 1.0).abs() < 0.01);
    }

    #[test]
    fn right_drag_orbits_and_keeps_the_view_point() {
        let mut n = nav();
        let (lat, lon, heading, tilt) = (n.cam.lat, n.cam.lon, n.cam.heading, n.cam.tilt);
        n.press(Button::Right, DVec2::new(600.0, 400.0), Mods::default());
        n.moved(DVec2::new(650.0, 440.0));
        assert_eq!((n.cam.lat, n.cam.lon), (lat, lon));
        assert!(n.cam.heading != heading);
        assert!(n.cam.tilt > tilt, "dragging down goes towards the horizon");
    }

    #[test]
    fn tilt_goes_past_the_horizon_up_to_straight_up() {
        let mut n = nav();
        n.press(Button::Right, DVec2::ZERO, Mods::default());
        n.moved(DVec2::new(0.0, 5000.0));
        assert_eq!(n.cam.tilt, MAX_TILT);
        assert!((n.cam.total_tilt() - std::f64::consts::PI).abs() < 1e-9);
        assert!(n.cam.basis().look.dot(enu(n.cam.lat, n.cam.lon).2) > 0.99, "looks up");
        n.moved(DVec2::new(0.0, -5000.0));
        assert_eq!((n.cam.tilt, n.cam.sky), (0.0, 0.0));
    }

    #[test]
    fn raising_the_head_keeps_the_camera_in_place() {
        let mut n = nav();
        n.cam.set_total_tilt(MAX_TILT);
        let before = n.cam.basis().pos;
        n.cam.add_tilt(0.6);
        assert!((n.cam.basis().pos - before).length() < 1e-6);
        assert!(n.cam.sky > 0.59);
    }

    #[test]
    fn shift_drag_up_climbs() {
        let mut n = nav();
        let d = n.cam.distance;
        n.press(Button::Left, DVec2::new(600.0, 400.0), Mods { shift: true, ..Mods::default() });
        n.moved(DVec2::new(600.0, 300.0));
        assert!(n.cam.distance > d);
    }

    #[test]
    fn arrows_fly_in_the_view_direction() {
        let mut n = nav();
        let before = n.cam.target();
        let forward = n.cam.ground_forward();
        n.arrow_down(Arrow::Up, Mods::default());
        run(&mut n, 0.5);
        n.arrow_up(Arrow::Up);
        let moved = n.cam.target() - before;
        assert!(moved.normalize().dot(forward) > 0.99);
        assert!(!n.is_animating(), "stops right away when let go");
    }

    #[test]
    fn right_arrow_flies_right_of_the_view() {
        let mut n = nav();
        let before = n.cam.target();
        let right = n.cam.basis().right;
        n.arrow_down(Arrow::Right, Mods::default());
        run(&mut n, 0.5);
        n.arrow_up(Arrow::Right);
        assert!((n.cam.target() - before).normalize().dot(right) > 0.99);
    }

    #[test]
    fn arrow_keeps_its_meaning_when_modifiers_change() {
        let mut n = nav();
        let d = n.cam.distance;
        n.arrow_down(Arrow::Up, Mods::default());
        n.arrow_down(Arrow::Up, Mods { shift: true, ..Mods::default() });
        run(&mut n, 0.5);
        assert_eq!(n.cam.distance, d, "still flying, not climbing");
    }

    #[test]
    fn ctrl_arrows_move_one_level() {
        let mut n = nav();
        let d = n.cam.distance;
        n.arrow_down(Arrow::Up, Mods { ctrl: true, ..Mods::default() });
        run(&mut n, 2.0);
        assert!((n.cam.distance / (d * 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn double_click_centres_and_closes_in() {
        let mut n = nav();
        let pos = DVec2::new(800.0, 300.0);
        let p = n.cam.pick(n.ndc(pos), n.aspect()).unwrap();
        let d = n.cam.distance;
        n.double_click(pos, Mods::default());
        run(&mut n, 5.0);
        assert!((n.cam.target() - p).length() < 1.0);
        assert!((n.cam.distance / (d * 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn straighten_takes_the_documented_steps() {
        let mut n = nav();
        n.straighten();
        run(&mut n, 5.0);
        assert!(n.cam.heading.abs() < 1e-9 && n.cam.tilt > 0.1);
        n.straighten();
        run(&mut n, 5.0);
        assert!(n.cam.tilt.abs() < 1e-9 && n.cam.distance < WHOLE_EARTH);
        n.straighten();
        run(&mut n, 5.0);
        assert_eq!(n.cam.distance, WHOLE_EARTH);
        assert!(n.cam.lat > 0.5);
        n.straighten();
        run(&mut n, 5.0);
        assert!(n.cam.lat.abs() < 1e-9);
    }

    #[test]
    fn long_flight_climbs_and_a_new_input_ends_it() {
        let mut n = nav();
        let to = Camera { lat: -0.5, lon: 2.5, ..n.cam };
        n.fly_to(to);
        let mut highest: f64 = 0.0;
        for _ in 0..60 {
            n.tick(1.0 / 60.0, NO_GROUND);
            highest = highest.max(n.cam.distance);
        }
        assert!(highest > 1_000_000.0);
        assert!(n.is_animating());
        n.press(Button::Left, DVec2::new(600.0, 400.0), Mods::default());
        assert!(!n.is_animating());
    }

    #[test]
    fn camera_keeps_clear_of_rising_ground() {
        let mut n = Nav::new(Camera { distance: 300.0, tilt: 0.0, ..nav().cam }, SIZE);
        let ground: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| Some(3000.0);
        for _ in 0..240 {
            n.tick(1.0 / 60.0, ground);
            let (_, _, h) = ecef_to_geodetic(n.cam.basis().pos);
            // Allow for the converging step while the view point rises.
            assert!(h > 300.0 - 1.0);
        }
        let (_, _, h) = ecef_to_geodetic(n.cam.basis().pos);
        assert!(h >= 3000.0 + GROUND_CLEARANCE - 0.5, "camera at {h}");
    }

    #[test]
    fn flat_ground_does_not_lift_the_camera() {
        let mut n = Nav::new(Camera { tilt: 0.0, ..nav().cam }, SIZE);
        let (_, _, before) = ecef_to_geodetic(n.cam.basis().pos);
        let ground: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| Some(1500.0);
        run_with(&mut n, ground, 2.0);
        let (_, _, after) = ecef_to_geodetic(n.cam.basis().pos);
        assert!((after - before).abs() < 1.0, "{before} -> {after}");
    }

    #[test]
    fn the_start_distance_counts_from_the_ground() {
        let cam = Camera { distance: 500.0, tilt: 0.0, ..nav().cam };
        let mut n = Nav::starting_above_ground(cam, SIZE);
        // First a coarse guess at the terrain, then the real height.
        let coarse: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| Some(1800.0);
        run_with(&mut n, coarse, 0.2);
        let fine: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| Some(2400.0);
        run_with(&mut n, fine, 0.2);
        assert_eq!(n.cam.distance, 500.0);
        let (_, _, h) = ecef_to_geodetic(n.cam.basis().pos);
        assert!((h - 2900.0).abs() < 0.5, "camera at {h}");
        n.finish_start();
        // Now terrain differences no longer carry the camera along.
        let other: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| Some(2300.0);
        run_with(&mut n, other, 2.0);
        let (_, _, later) = ecef_to_geodetic(n.cam.basis().pos);
        assert!((later - 2900.0).abs() < 1.0, "camera moved to {later}");
    }

    #[test]
    fn any_input_ends_the_start_phase() {
        let cam = Camera { distance: 500.0, tilt: 0.0, ..nav().cam };
        let mut n = Nav::starting_above_ground(cam, SIZE);
        n.scroll(1.0, DVec2::new(600.0, 400.0), Mods::default());
        assert!(!n.starting);
        let mut n = Nav::starting_above_ground(cam, SIZE);
        n.press(Button::Left, DVec2::new(600.0, 400.0), Mods::default());
        assert!(!n.starting);
        let mut n = Nav::starting_above_ground(cam, SIZE);
        n.whole_body();
        assert!(!n.starting);
    }

    fn run_with(n: &mut Nav, ground: &dyn Fn(f64, f64) -> Option<f64>, seconds: f64) {
        for _ in 0..(seconds * 60.0) as usize {
            n.tick(1.0 / 60.0, ground);
        }
    }

    #[test]
    fn device_moves_the_camera_and_ends_a_flight() {
        let mut n = nav();
        n.whole_body();
        assert!(n.flight.is_some());
        n.set_device([0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 1.0);
        assert!(n.flight.is_some(), "a resting device leaves the flight alone");
        n.set_device([0.0, 0.5, 0.0, 0.0, 0.0, 0.0], 1.0);
        assert!(n.flight.is_none());
        assert!(n.is_animating());
        let d = n.cam.distance;
        n.tick(0.1, NO_GROUND);
        assert!(n.cam.distance > d);
        n.set_device([0.0; 6], 1.0);
        assert!(!n.is_animating());
    }

    #[test]
    fn device_speed_follows_height_not_frame_rate() {
        let axes = [0.5, 0.3, -0.4, 0.2, 0.5, 0.0];
        let go = |dist: f64, steps: usize| {
            let mut n = nav();
            n.cam.distance = dist;
            n.set_device(axes, 1.0);
            for _ in 0..steps {
                n.tick(1.0 / steps as f64, NO_GROUND);
            }
            n.cam
        };
        let (a, b) = (go(100_000.0, 30), go(100_000.0, 240));
        assert!((a.lat - b.lat).abs() < 1e-3 && (a.lon - b.lon).abs() < 1e-3);
        assert!((a.distance / b.distance - 1.0).abs() < 1e-6);
        assert!((a.heading - b.heading).abs() < 1e-3);
        // Ten times lower, ten times less ground covered.
        let (hi, lo) = (go(100_000.0, 60), go(10_000.0, 60));
        let moved = |c: Camera| ((c.lat - 0.82).powi(2) + ((c.lon - 0.2) * 0.82f64.cos()).powi(2)).sqrt();
        assert!((moved(hi) / moved(lo) - 10.0).abs() < 1.5, "{} {}", moved(hi), moved(lo));
    }

    #[test]
    fn device_tilts_past_the_horizon_and_respects_limits() {
        let mut n = nav();
        n.set_device([0.0, 0.0, 0.0, -1.0, 0.0, 0.0], 1.0);
        run(&mut n, 6.0);
        assert!(n.cam.sky > 0.0, "tilting on looks up into the sky");
        assert!(n.cam.total_tilt() <= MAX_TILT + crate::camera::MAX_SKY + 1e-9);
        n.set_device([0.0, 0.0, 0.0, 1.0, 0.0, 0.0], 1.0);
        run(&mut n, 10.0);
        assert_eq!(n.cam.total_tilt(), 0.0);
        n.set_device([0.0, -1.0, 0.0, 0.0, 0.0, 0.0], 1.0);
        run(&mut n, 30.0);
        assert!(n.cam.distance >= crate::camera::MIN_DISTANCE);
    }

    #[test]
    fn device_sinking_keeps_two_metres_above_ground() {
        let mut n = Nav::new(Camera { distance: 500.0, tilt: 0.0, ..nav().cam }, SIZE);
        let ground = |_: f64, _: f64| Some(120.0);
        n.cam.target_h = 120.0;
        n.set_device([0.0, -1.0, 0.0, 0.0, 0.0, 0.0], 1.0);
        for _ in 0..600 {
            n.tick(1.0 / 60.0, &ground);
        }
        assert!(n.altitude >= 2.0 - 1e-6, "altitude {}", n.altitude);
    }
}
