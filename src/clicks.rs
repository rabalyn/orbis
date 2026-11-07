//! Tells a drag, a single click and a double click apart on the map, without a window.
//! Times are seconds from any fixed origin, so tests can supply their own.

use crate::nav::Mods;
use glam::DVec2;

/// Further than this between press and release (pixels) is a drag, not a click.
pub const DRAG_PX: f64 = 5.0;
/// Held longer than this (seconds) is not a click either.
pub const HOLD_S: f64 = 0.3;
/// A second click this soon (seconds) after the first, and this near (pixels), is a double click.
pub const DOUBLE_S: f64 = 0.4;
pub const DOUBLE_PX: f64 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickKind {
    /// Reported only once the double-click window has passed without a second click.
    Single,
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub kind: ClickKind,
    pub pos: DVec2,
    pub mods: Mods,
}

#[derive(Default)]
pub struct ClickClassifier {
    down: Option<(f64, DVec2)>,
    /// A click that may still turn out to be the first of two.
    pending: Option<(f64, Click)>,
}

impl ClickClassifier {
    /// A left press that the map owns.
    pub fn press(&mut self, now: f64, pos: DVec2) {
        self.down = Some((now, pos));
    }

    /// The matching release. Returns at most one finished click: a double click, or an earlier
    /// single click that this release shows to be on its own.
    pub fn release(&mut self, now: f64, pos: DVec2, mods: Mods) -> Option<Click> {
        let (t0, p0) = self.down.take()?;
        let is_click = now - t0 <= HOLD_S && p0.distance(pos) <= DRAG_PX;
        if !is_click {
            return self.pending.take().map(|p| p.1);
        }
        if let Some((t, first)) = self.pending {
            if now - t <= DOUBLE_S && first.pos.distance(pos) <= DOUBLE_PX {
                self.pending = None;
                return Some(Click { kind: ClickKind::Double, pos, mods });
            }
        }
        let earlier = self.pending.take().map(|p| p.1);
        self.pending = Some((now, Click { kind: ClickKind::Single, pos, mods }));
        earlier
    }

    /// A single click whose double-click window has run out.
    pub fn poll(&mut self, now: f64) -> Option<Click> {
        match self.pending {
            Some((t, c)) if now - t > DOUBLE_S => {
                self.pending = None;
                Some(c)
            }
            _ => None,
        }
    }

    /// When `poll` will next have something to report, if anything is waiting.
    pub fn deadline(&self) -> Option<f64> {
        self.pending.map(|(t, _)| t + DOUBLE_S)
    }

    /// Forgets a press that was cut short, such as by losing focus.
    pub fn cancel(&mut self) {
        self.down = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }

    fn click(c: &mut ClickClassifier, t: f64, pos: DVec2) -> Option<Click> {
        c.press(t, pos);
        c.release(t + 0.05, pos, Mods::default())
    }

    #[test]
    fn single_click_waits_out_the_window() {
        let mut c = ClickClassifier::default();
        assert_eq!(click(&mut c, 1.0, p(10.0, 10.0)), None);
        assert_eq!(c.poll(1.2), None);
        assert_eq!(c.deadline(), Some(1.05 + DOUBLE_S));
        let got = c.poll(1.6).unwrap();
        assert_eq!((got.kind, got.pos), (ClickKind::Single, p(10.0, 10.0)));
        assert_eq!(c.poll(2.0), None);
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn two_clicks_close_together_make_a_double_and_no_single() {
        let mut c = ClickClassifier::default();
        click(&mut c, 1.0, p(10.0, 10.0));
        let got = click(&mut c, 1.2, p(12.0, 11.0)).unwrap();
        assert_eq!(got.kind, ClickKind::Double);
        assert_eq!(c.poll(5.0), None);
        // A third click starts afresh.
        assert_eq!(click(&mut c, 1.4, p(12.0, 11.0)), None);
        assert!(c.deadline().is_some());
    }

    #[test]
    fn slow_or_distant_second_click_is_a_new_single() {
        let mut c = ClickClassifier::default();
        click(&mut c, 1.0, p(10.0, 10.0));
        let first = click(&mut c, 1.7, p(10.0, 10.0)).unwrap();
        assert_eq!(first.kind, ClickKind::Single);
        let first = click(&mut c, 1.8, p(60.0, 10.0)).unwrap();
        assert_eq!((first.kind, first.pos), (ClickKind::Single, p(10.0, 10.0)));
        assert_eq!(c.poll(3.0).unwrap().pos, p(60.0, 10.0));
    }

    #[test]
    fn dragging_or_holding_is_no_click() {
        let mut c = ClickClassifier::default();
        c.press(1.0, p(0.0, 0.0));
        assert_eq!(c.release(1.1, p(30.0, 0.0), Mods::default()), None);
        assert_eq!(c.deadline(), None);
        c.press(2.0, p(0.0, 0.0));
        assert_eq!(c.release(2.5, p(0.0, 0.0), Mods::default()), None);
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn a_drag_between_clicks_releases_the_waiting_single() {
        let mut c = ClickClassifier::default();
        click(&mut c, 1.0, p(10.0, 10.0));
        c.press(1.1, p(10.0, 10.0));
        let got = c.release(1.2, p(80.0, 10.0), Mods::default()).unwrap();
        assert_eq!(got.kind, ClickKind::Single);
        assert_eq!(c.deadline(), None);
    }

    #[test]
    fn release_without_press_is_ignored() {
        let mut c = ClickClassifier::default();
        assert_eq!(c.release(1.0, p(0.0, 0.0), Mods::default()), None);
    }
}
