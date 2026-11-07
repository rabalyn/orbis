//! The time icon and dialog: three sliders (year, day of the year, time of day), the buttons for
//! the clock and for the eclipses, and what is less accurate at the date.
//!
//! The dialog shows a [`Model`] shared with the scene, which fills it from the clock; what the
//! user does comes back from [`handle`] as an [`Action`] for the scene to carry out on the clock.
//! The date arithmetic is in `time_slider`.

use crate::text;
use crate::time_slider::{self, Level, Track};
use crate::ui::{Drag, Metrics, Panel, Rect, Tone, Ui, UiEvent, Widget, WidgetId};
use std::cell::RefCell;
use std::rc::Rc;

pub const ICON: &str = "time-icon";
pub const WINDOW: &str = "time-window";
const OPEN_BUTTON: WidgetId = 1;
const CLOSE_BUTTON: WidgetId = 2;
const NOW_BUTTON: WidgetId = 3;
const PAUSE_BUTTON: WidgetId = 4;
const PREVIOUS_BUTTON: WidgetId = 5;
const NEXT_BUTTON: WidgetId = 6;
const YEAR_SLIDER: WidgetId = 10;
const DAY_SLIDER: WidgetId = 11;
const TIME_SLIDER: WidgetId = 12;
const COLS: usize = 38;

const TRACKS: [(WidgetId, Track, &str); 3] = [(YEAR_SLIDER, Track::Year, "Year"), (DAY_SLIDER, Track::Day, "Day"), (TIME_SLIDER, Track::Time, "Time of day")];

/// What the dialog shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// The clock, to the minute: where the thumbs stand.
    pub time: f64,
    /// The full reading of the clock with its seconds and its state.
    pub reading: String,
    pub running: bool,
    /// Eclipses are worked out for the Earth only.
    pub earth: bool,
    /// The eclipse going on at the clock's time, if any.
    pub eclipse: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        Self { time: 0.0, reading: String::new(), running: true, earth: true, eclipse: None }
    }
}

pub type Shared = Rc<RefCell<Model>>;

/// What the user asked of the clock.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A drag begins: the clock stands still. On the bar beside the thumb the value jumps there.
    Begin(Track, Option<f64>),
    Set(Track, f64),
    /// The drag is over: the clock goes on as it was.
    End,
    Step(Track, i32),
    Now,
    PauseRun,
    PreviousEclipse,
    NextEclipse,
}

pub fn icon_panel() -> Panel {
    let mut p = Panel::new(ICON, false, |p, m| {
        p.rect = m.icon_rect(2);
        p.fixed = vec![Widget::clock_face(OPEN_BUTTON, Rect::new(0.0, 0.0, p.rect.w, p.rect.h))];
        p.visible = true;
    });
    p.in_band = true;
    p
}

pub fn window_panel(model: Shared) -> Panel {
    Panel::new(WINDOW, true, move |p, m| layout(p, m, &model.borrow()))
}

fn track_of(widget: WidgetId) -> Option<Track> {
    TRACKS.iter().find(|t| t.0 == widget).map(|t| t.1)
}

/// Opens or closes the dialog for a click on the icon or its close button. Anything else that the
/// dialog was used for is returned as an action. The first value is true if the event was the dialog's.
pub fn handle(ui: &mut Ui, event: UiEvent) -> (bool, Option<Action>) {
    match event {
        UiEvent::Clicked { panel: ICON, widget: OPEN_BUTTON } => ui.toggle(WINDOW),
        UiEvent::Clicked { panel: WINDOW, widget: CLOSE_BUTTON } => ui.set_visible(WINDOW, false),
        UiEvent::Clicked { panel: WINDOW, widget } => {
            let action = match widget {
                NOW_BUTTON => Action::Now,
                PAUSE_BUTTON => Action::PauseRun,
                PREVIOUS_BUTTON => Action::PreviousEclipse,
                NEXT_BUTTON => Action::NextEclipse,
                _ => return (true, None),
            };
            return (true, Some(action));
        }
        UiEvent::Slid { panel: WINDOW, widget, pos, drag } => {
            let Some(track) = track_of(widget) else { return (true, None) };
            let action = match drag {
                Drag::Grab => Action::Begin(track, None),
                Drag::Jump => Action::Begin(track, Some(pos)),
                Drag::Move => Action::Set(track, pos),
                Drag::End => Action::End,
            };
            return (true, Some(action));
        }
        UiEvent::Stepped { panel: WINDOW, widget, steps } => {
            return (true, track_of(widget).map(|t| Action::Step(t, steps)));
        }
        _ => return (false, None),
    }
    (true, None)
}

fn layout(p: &mut Panel, m: &Metrics, model: &Model) {
    let s = m.scale;
    let (glyph, step, pad) = (8.0 * s, 10.0 * s, 6.0 * s);
    let (row, margin, header, button_h) = (12.0 * s, 8.0 * s, 18.0 * s, 14.0 * s);
    let w = (m.screen.x - 2.0 * margin).min(COLS as f64 * glyph + 2.0 * pad).max(20.0 * glyph);
    let inner = w - 2.0 * pad;
    let cols = ((inner / glyph) as usize).max(10);
    let text_at = |y: f64, text: &str, tone: Tone| Widget::label(Rect::new(pad, y, text.chars().count() as f64 * glyph, glyph), text, tone);

    let mut widgets = Vec::new();
    let mut y = header + pad;
    let reading: String = model.reading.chars().take(cols).collect();
    widgets.push(text_at(y, &reading, Tone::Heading));
    y += step + 6.0 * s;
    for (id, track, name) in TRACKS {
        widgets.push(text_at(y, &format!("{name}: {}", time_slider::value_label(track, model.time)), Tone::Normal));
        y += step + 2.0 * s;
        widgets.push(Widget::slider(id, Rect::new(pad, y, inner, row), time_slider::position(track, model.time), 8.0 * s));
        y += row + 1.0 * s;
        let (first, last) = time_slider::end_labels(track);
        widgets.push(text_at(y, first, Tone::Dim));
        let right = Widget::label(Rect::new(w - pad - last.len() as f64 * glyph, y, last.len() as f64 * glyph, glyph), last, Tone::Dim);
        widgets.push(right);
        y += step + 4.0 * s;
    }
    let gap = 4.0 * s;
    let half = (inner - gap) / 2.0;
    let pair = |widgets: &mut Vec<Widget>, y: f64, a: (WidgetId, &str), b: (WidgetId, &str)| {
        widgets.push(Widget::button(a.0, Rect::new(pad, y, half, button_h), a.1, 1));
        widgets.push(Widget::button(b.0, Rect::new(pad + half + gap, y, half, button_h), b.1, 1));
    };
    pair(&mut widgets, y, (NOW_BUTTON, "Now"), (PAUSE_BUTTON, if model.running { "Pause" } else { "Run" }));
    y += button_h + gap;
    if model.earth {
        pair(&mut widgets, y, (PREVIOUS_BUTTON, "Previous eclipse"), (NEXT_BUTTON, "Next eclipse"));
        y += button_h + gap + 2.0 * s;
        let (line, tone) = match &model.eclipse {
            Some(e) => (e.as_str(), Tone::Key),
            None => ("No eclipse at this moment", Tone::Dim),
        };
        for l in text::wrap(line, cols) {
            widgets.push(text_at(y, &l, tone));
            y += step;
        }
    }
    let notices = time_slider::notices(model.time);
    if !notices.is_empty() {
        y += 4.0 * s;
    }
    for (level, note) in notices {
        for l in text::wrap(note, cols) {
            widgets.push(text_at(y, &l, if level == Level::Warning { Tone::Warn } else { Tone::Dim }));
            y += step;
        }
    }
    let h = y + pad;

    // Beside the view options, above the credits.
    let bottom = m.floor() - 4.0 * s;
    let h = h.min((bottom - margin).max(header + 4.0 * step));
    let x = (margin + crate::options::window_width(m) + margin).min(m.screen.x - margin - w).max(margin);
    p.rect = Rect::new(x, bottom - h, w, h);
    widgets.push(text_at((header - glyph) / 2.0, "Time", Tone::Heading));
    widgets.push(Widget::button(CLOSE_BUTTON, Rect::new(w - pad - 12.0 * s, 3.0 * s, 12.0 * s, 12.0 * s), "x", 1));
    p.fixed = widgets;
    p.body = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::Button;
    use crate::sky_time::{format_utc, parse_iso8601, Clock};
    use crate::ui::{Kind, Response};
    use glam::DVec2;

    /// The dialog, a clock, and what the scene does between them.
    struct Sim {
        ui: Ui,
        model: Shared,
        clock: Clock,
    }

    fn sim(w: f64, h: f64, scale: f64, time: &str, earth: bool) -> Sim {
        let model: Shared = Default::default();
        let mut ui = Ui::new();
        ui.resize(DVec2::new(w, h), scale);
        ui.add(crate::options::icon_panel());
        ui.add(icon_panel());
        ui.add(window_panel(model.clone()));
        ui.add(crate::options::window_panel(Default::default()));
        ui.set_visible(WINDOW, true);
        let mut s = Sim { ui, model, clock: Clock::new(parse_iso8601(time).unwrap(), 0.0) };
        s.model.borrow_mut().earth = earth;
        s.sync();
        s
    }

    impl Sim {
        fn sync(&mut self) {
            let t = self.clock.time();
            let m = Model { time: (t / 60.0).floor() * 60.0, reading: self.clock.reading(), running: self.clock.is_running(), earth: self.model.borrow().earth, eclipse: None };
            *self.model.borrow_mut() = m;
            self.ui.refresh(WINDOW);
        }

        fn act(&mut self, a: Action) {
            let t = self.clock.time();
            match a {
                Action::Begin(track, pos) => {
                    self.clock.hold();
                    if let Some(p) = pos {
                        self.clock.set_time(time_slider::set_position(track, t, p));
                    }
                }
                Action::Set(track, pos) => self.clock.set_time(time_slider::set_position(track, t, pos)),
                Action::End => self.clock.release(),
                Action::Step(track, n) => self.clock.set_time(time_slider::step(track, t, n as i64)),
                Action::PauseRun => self.clock.apply(crate::sky_time::ClockKey::PauseResume, 0.0),
                Action::Now => self.clock.apply(crate::sky_time::ClockKey::Now, t),
                _ => {}
            }
            self.sync();
        }

        fn feed(&mut self, r: Response) -> Option<Action> {
            let (_, a) = handle(&mut self.ui, r.event?);
            if let Some(a) = a.clone() {
                self.act(a);
            }
            a
        }

        fn panel(&self) -> &Panel {
            self.ui.panels().iter().find(|p| p.id == WINDOW).unwrap()
        }

        /// The middle of a slider's thumb, on the screen.
        fn thumb(&self, id: WidgetId) -> DVec2 {
            let p = self.panel();
            let w = p.fixed.iter().find(|w| w.id == id).unwrap();
            let Kind::Slider { pos, thumb_w } = w.kind else { panic!() };
            let t = crate::ui::slider_thumb(w.rect, thumb_w, pos);
            DVec2::new(p.rect.x + t.x + t.w / 2.0, p.rect.y + t.y + t.h / 2.0)
        }

        fn press(&mut self, p: DVec2) -> Option<Action> {
            self.ui.pointer_moved(p);
            let r = self.ui.press(Button::Left, p);
            self.feed(r)
        }

        fn moved(&mut self, p: DVec2) -> Option<Action> {
            let r = self.ui.pointer_moved(p);
            self.feed(r)
        }

        fn release(&mut self, p: DVec2) -> Option<Action> {
            let r = self.ui.release(Button::Left, p);
            self.feed(r)
        }

        fn shown(&self) -> String {
            format_utc(self.clock.time(), true)
        }

        fn labels(&self) -> Vec<String> {
            self.panel().fixed.iter().filter_map(|w| match &w.kind {
                Kind::Label { text, .. } | Kind::Button { label: text, .. } => Some(text.clone()),
                _ => None,
            }).collect()
        }
    }

    #[test]
    fn the_thumbs_stand_where_the_time_is() {
        for scale in [1.0, 2.0] {
            let s = sim(1280.0, 800.0, scale, "2026-06-21T16:00:00Z", true);
            let p = s.panel();
            let bar = |id: WidgetId| {
                let w = p.fixed.iter().find(|w| w.id == id).unwrap();
                (p.rect.x + w.rect.x, w.rect.w)
            };
            for (id, want) in [(YEAR_SLIDER, 226.0 / 250.0), (TIME_SLIDER, 960.0 / 1439.0)] {
                let (x0, w) = bar(id);
                let at = (s.thumb(id).x - 4.0 * scale - x0) / (w - 8.0 * scale);
                assert!((at - want).abs() < 0.01, "{id} at scale {scale}: {at}");
            }
            assert!(s.labels().contains(&"Year: 2026".to_string()) && s.labels().contains(&"Time of day: 16:00".to_string()));
            assert!(s.labels().iter().any(|l| l.starts_with("2026-06-21 16:00 UTC")));
        }
    }

    #[test]
    fn dragging_from_the_thumb_moves_only_that_part_and_does_not_jump_on_grab() {
        for scale in [1.0, 2.0] {
            let mut s = sim(1280.0, 800.0, scale, "2026-06-21T16:00:00Z", true);
            let start = s.thumb(TIME_SLIDER) + DVec2::new(2.5 * scale, 0.0);
            assert_eq!(s.press(start), Some(Action::Begin(Track::Time, None)));
            assert_eq!(s.shown(), "2026-06-21 16:00:00 UTC", "grabbing changes nothing");
            assert!(s.clock.is_held());
            // 296 design pixels of travel for 1439 minutes: 74 px is a quarter of the day, six hours.
            let a = s.moved(start + DVec2::new(-74.0 * scale, 30.0 * scale));
            assert!(matches!(a, Some(Action::Set(Track::Time, _))));
            let minutes = (s.clock.time() / 60.0).rem_euclid(1440.0);
            assert!((minutes - (960.0 - 359.75)).abs() < 2.0, "{minutes}");
            assert!(s.shown().starts_with("2026-06-21"), "the date stays");
            assert_eq!(s.release(start), Some(Action::End));
            assert!(!s.clock.is_held());
        }
    }

    #[test]
    fn a_drag_leaves_the_clock_running_or_paused_at_the_same_rate() {
        for running in [true, false] {
            let mut s = sim(1280.0, 800.0, 1.0, "2026-06-21T16:00:00Z", true);
            s.clock.set_rate(600.0);
            if !running {
                s.clock.pause();
            }
            s.sync();
            let start = s.thumb(YEAR_SLIDER);
            s.press(start);
            s.clock.advance(100.0);
            s.moved(start + DVec2::new(-60.0, 0.0));
            let t = s.clock.time();
            s.clock.advance(200.0);
            assert_eq!(s.clock.time(), t, "the clock stands still while the slider is held");
            s.release(start);
            assert_eq!((s.clock.is_running(), s.clock.rate()), (running, 600.0));
            s.clock.advance(210.0);
            assert_eq!(s.clock.time() - t, if running { 6000.0 } else { 0.0 });
        }
    }

    #[test]
    fn a_click_on_the_bar_jumps_there_and_the_year_slider_keeps_month_day_and_time() {
        let mut s = sim(1280.0, 800.0, 2.0, "2024-02-29T10:30:00Z", true);
        let y = s.thumb(YEAR_SLIDER);
        // Centre of a 296 px (design) travel from the thumb: 1925 or so.
        let a = s.press(DVec2::new(y.x - 0.5 * 296.0 * 2.0, y.y));
        assert!(matches!(a, Some(Action::Begin(Track::Year, Some(_)))));
        let shown = s.shown();
        assert!(shown.ends_with("-02-28 10:30:00 UTC"), "{shown}: 29 February in a year without one");
        s.release(y);
    }

    #[test]
    fn the_wheel_steps_a_year_a_day_or_ten_minutes() {
        let mut s = sim(1280.0, 800.0, 1.0, "2026-06-21T16:00:00Z", true);
        for (id, want) in [(YEAR_SLIDER, "2027-06-21 16:00:00 UTC"), (DAY_SLIDER, "2027-06-22 16:00:00 UTC"), (TIME_SLIDER, "2027-06-22 16:10:00 UTC")] {
            let at = s.thumb(id);
            let r = s.ui.wheel(1.0, at);
            s.feed(r);
            assert_eq!(s.shown(), want);
        }
    }

    #[test]
    fn eclipse_buttons_and_the_eclipse_line_are_for_the_earth_only() {
        let earth = sim(1280.0, 800.0, 1.0, "2026-06-21T16:00:00Z", true);
        assert!(earth.labels().contains(&"Next eclipse".to_string()) && earth.labels().contains(&"Previous eclipse".to_string()));
        assert!(earth.labels().contains(&"No eclipse at this moment".to_string()));
        let moon = sim(1280.0, 800.0, 1.0, "2026-06-21T16:00:00Z", false);
        assert!(!moon.labels().iter().any(|l| l.contains("clipse")), "{:?}", moon.labels());
        assert!(moon.labels().contains(&"Now".to_string()) && moon.labels().contains(&"Pause".to_string()));
    }

    #[test]
    fn the_buttons_ask_for_their_actions_and_pause_says_run_while_paused() {
        let mut s = sim(1280.0, 800.0, 1.0, "2026-06-21T16:00:00Z", true);
        let centre_of = |s: &Sim, label: &str| {
            let p = s.panel();
            let w = p.fixed.iter().find(|w| matches!(&w.kind, Kind::Button { label: l, .. } if l == label)).unwrap();
            DVec2::new(p.rect.x + w.rect.x + w.rect.w / 2.0, p.rect.y + w.rect.y + w.rect.h / 2.0)
        };
        for (label, want) in [("Now", Action::Now), ("Pause", Action::PauseRun), ("Next eclipse", Action::NextEclipse), ("Previous eclipse", Action::PreviousEclipse)] {
            let at = centre_of(&s, label);
            s.press(at);
            assert_eq!(s.release(at), Some(want), "{label}");
            if label == "Pause" {
                assert!(s.labels().contains(&"Run".to_string()) && !s.clock.is_running());
            }
        }
    }

    #[test]
    fn dates_outside_the_range_show_the_warning_and_the_year_thumb_sits_at_its_end() {
        let s = sim(1280.0, 800.0, 1.0, "1700-01-01T12:00:00Z", true);
        assert!(s.labels().iter().any(|l| l.starts_with("Outside 1800-2050")), "{:?}", s.labels());
        let p = s.panel();
        let w = p.fixed.iter().find(|w| w.id == YEAR_SLIDER).unwrap();
        let Kind::Slider { pos, .. } = w.kind else { panic!() };
        assert_eq!(pos, 0.0);
        let s = sim(1280.0, 800.0, 1.0, "2026-06-21T12:00:00Z", true);
        assert!(!s.labels().iter().any(|l| l.contains("Outside") || l.contains("less exact")));
    }

    #[test]
    fn the_dialog_stays_above_the_credits_and_beside_the_options_at_usual_sizes() {
        for (w, h, scale) in [(1280.0, 800.0, 1.0), (2560.0, 1600.0, 2.0), (1024.0, 700.0, 1.0)] {
            let mut s = sim(w, h, scale, "1700-01-01T12:00:00Z", true);
            s.ui.set_credit_band(70.0 * scale);
            s.ui.set_visible(crate::options::WINDOW, true);
            let r = s.panel().rect;
            assert!(r.x >= 0.0 && r.x + r.w <= w && r.y >= 0.0, "{w}x{h}: {r:?}");
            assert!(s.ui.covering_credits().is_empty(), "{w}x{h}");
            if w >= 1280.0 {
                let o = s.ui.panels().iter().find(|p| p.id == crate::options::WINDOW).unwrap().rect;
                assert!(!o.intersects(&r), "{o:?} {r:?}");
            }
        }
    }

    #[test]
    fn the_icon_opens_and_the_close_button_closes_and_the_three_icons_sit_apart() {
        let mut s = sim(1280.0, 800.0, 1.0, "2026-06-21T16:00:00Z", true);
        s.ui.set_visible(WINDOW, false);
        let icon = s.ui.panels().iter().find(|p| p.id == ICON).unwrap().rect;
        let at = DVec2::new(icon.x + 5.0, icon.y + 5.0);
        s.press(at);
        s.release(at);
        assert!(s.ui.is_visible(WINDOW));
        let r = s.panel().rect;
        let close = DVec2::new(r.x + r.w - 12.0, r.y + 9.0);
        s.press(close);
        s.release(close);
        assert!(!s.ui.is_visible(WINDOW));
        let options = s.ui.panels().iter().find(|p| p.id == crate::options::ICON).unwrap().rect;
        assert!(!icon.intersects(&options) && icon.x < options.x);
    }
}
