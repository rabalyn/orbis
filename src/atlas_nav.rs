//! Turns input into movement of the flat map. Independent of the window system.

use crate::atlas::{AtlasView, Projection};
use glam::DVec2;

/// Zoom per scroll line, as a logarithm of the scale factor (the globe uses the same step).
const ZOOM_PER_LINE: f64 = 0.25;
const MERIDIAN_STEP: f64 = 15.0;
const MERIDIAN_FINE_STEP: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Next projection, or the previous one with `back`.
    CycleProjection { back: bool },
    /// Moves the central meridian east or west, in finer steps with `fine`.
    ShiftMeridian { east: bool, fine: bool },
    /// Shows the whole map.
    Whole,
}

pub struct AtlasNav {
    pub view: AtlasView,
    drag: Option<DVec2>,
}

impl AtlasNav {
    pub fn new(view: AtlasView) -> Self {
        Self { view, drag: None }
    }

    pub fn press(&mut self, pos: DVec2) {
        self.drag = Some(pos);
    }

    /// Returns whether the view changed.
    pub fn moved(&mut self, pos: DVec2) -> bool {
        let Some(last) = self.drag else { return false };
        self.drag = Some(pos);
        let before = self.view;
        self.view.pan(pos.x - last.x, pos.y - last.y);
        self.view != before
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    #[cfg(test)]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// `lines` is positive for scrolling away from the user, which zooms in.
    pub fn scroll(&mut self, lines: f64, pos: DVec2) {
        self.view.zoom_at((pos.x, pos.y), (lines * ZOOM_PER_LINE).exp());
    }

    /// Centres on a point and zooms in (or out, with `out`) by a factor of two.
    pub fn double_click(&mut self, pos: DVec2, out: bool) {
        let at = self.view.map_of((pos.x, pos.y));
        if self.view.projection.inverse(at.0, at.1).is_some() {
            self.view.centre = at;
        }
        self.view.zoom_at((self.view.size.0 / 2.0, self.view.size.1 / 2.0), if out { 0.5 } else { 2.0 });
    }

    pub fn key(&mut self, key: Key) {
        let v = &mut self.view;
        match key {
            Key::CycleProjection { back } => {
                let p: Projection = if back { v.projection.previous() } else { v.projection.next() };
                v.set_projection(p);
            }
            Key::ShiftMeridian { east, fine } => {
                let step = if fine { MERIDIAN_FINE_STEP } else { MERIDIAN_STEP }.to_radians();
                v.set_central_meridian(v.central_meridian + if east { step } else { -step });
            }
            Key::Whole => v.show_whole(),
        }
    }

    pub fn resize(&mut self, size: DVec2) {
        self.view.resize((size.x, size.y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav() -> AtlasNav {
        AtlasNav::new(AtlasView::whole(Projection::EqualEarth, 0.0, (1280.0, 800.0)))
    }

    #[test]
    fn moving_without_a_press_does_nothing() {
        let mut n = nav();
        assert!(!n.moved(DVec2::new(10.0, 10.0)));
    }

    #[test]
    fn dragging_pans_and_release_stops() {
        let mut n = nav();
        n.scroll(8.0, DVec2::new(640.0, 400.0));
        let held = n.view.lat_lon_at(600.0, 400.0).unwrap();
        n.press(DVec2::new(600.0, 400.0));
        assert!(n.is_dragging());
        assert!(n.moved(DVec2::new(650.0, 380.0)));
        let now = n.view.lat_lon_at(650.0, 380.0).unwrap();
        assert!((now.0 - held.0).abs() < 1e-9 && (now.1 - held.1).abs() < 1e-9);
        n.release();
        assert!(!n.moved(DVec2::new(900.0, 100.0)));
    }

    #[test]
    fn dragging_the_world_view_is_held_at_the_edge() {
        let mut n = nav();
        n.press(DVec2::new(100.0, 100.0));
        n.moved(DVec2::new(5000.0, 100.0));
        let (_, lon) = n.view.centre_lat_lon();
        assert!(lon.abs() <= std::f64::consts::PI + 1e-9);
        assert!(n.view.projection.inverse(n.view.centre.0, n.view.centre.1).is_some());
    }

    #[test]
    fn scrolling_zooms_in_and_out_around_the_pointer() {
        let mut n = nav();
        let pointer = DVec2::new(1000.0, 200.0);
        let before = n.view.lat_lon_at(pointer.x, pointer.y).unwrap();
        n.scroll(6.0, pointer);
        let now = n.view.lat_lon_at(pointer.x, pointer.y).unwrap();
        assert!((now.0 - before.0).abs() < 1e-9 && (now.1 - before.1).abs() < 1e-9);
        let zoomed = n.view.scale;
        n.scroll(-6.0, pointer);
        assert!(n.view.scale < zoomed);
        n.scroll(-100.0, pointer);
        assert!((n.view.scale - n.view.fit_scale()).abs() < 1e-9);
    }

    #[test]
    fn keys_cycle_the_projection_and_shift_the_meridian() {
        let mut n = nav();
        n.key(Key::CycleProjection { back: false });
        assert_eq!(n.view.projection, Projection::Mollweide);
        n.key(Key::CycleProjection { back: true });
        n.key(Key::CycleProjection { back: true });
        assert_eq!(n.view.projection, Projection::PlateCarree);
        n.key(Key::ShiftMeridian { east: true, fine: false });
        assert!((n.view.central_meridian.to_degrees() - 15.0).abs() < 1e-9);
        n.key(Key::ShiftMeridian { east: false, fine: true });
        assert!((n.view.central_meridian.to_degrees() - 14.0).abs() < 1e-9);
        for _ in 0..30 {
            n.key(Key::ShiftMeridian { east: true, fine: false });
        }
        // 14 + 450 degrees wraps to 104.
        assert!((n.view.central_meridian.to_degrees() - 104.0).abs() < 1e-9);
        n.key(Key::Whole);
        assert!((n.view.scale - n.view.fit_scale()).abs() < 1e-9);
    }

    #[test]
    fn the_point_in_the_middle_survives_key_changes() {
        let mut n = nav();
        n.scroll(10.0, DVec2::new(400.0, 300.0));
        let (lat, lon) = n.view.centre_lat_lon();
        n.key(Key::ShiftMeridian { east: true, fine: false });
        n.key(Key::CycleProjection { back: false });
        let (la, lo) = n.view.centre_lat_lon();
        assert!((la - lat).abs() < 1e-6 && (lo - lon).abs() < 1e-6);
    }

    #[test]
    fn double_click_centres_and_zooms() {
        let mut n = nav();
        let at = DVec2::new(900.0, 250.0);
        let target = n.view.lat_lon_at(at.x, at.y).unwrap();
        let scale = n.view.scale;
        n.double_click(at, false);
        assert!((n.view.scale / scale - 2.0).abs() < 1e-9);
        let c = n.view.centre_lat_lon();
        assert!((c.0 - target.0).abs() < 1e-3 && (c.1 - target.1).abs() < 1e-3);
        let off_map = DVec2::new(1.0, 1.0);
        let before = n.view.centre;
        n.double_click(off_map, true);
        assert!(n.view.centre.0.is_finite() && before.0.is_finite());
    }

    #[test]
    fn resizing_keeps_the_middle() {
        let mut n = nav();
        n.scroll(8.0, DVec2::new(300.0, 300.0));
        let c = n.view.centre_lat_lon();
        n.resize(DVec2::new(800.0, 600.0));
        let d = n.view.centre_lat_lon();
        assert!((c.0 - d.0).abs() < 1e-6 && (c.1 - d.1).abs() < 1e-6);
    }
}
