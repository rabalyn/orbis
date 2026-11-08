//! The energy display window: the colours of the sources, how capacity that is not running
//! shows, and the size of the bars. It opens from the view options and from the day chart, and
//! every change is saved with the other settings.
//!
//! It shows a [`Model`] shared with the scene; a click or a slider comes back from [`handle`] as
//! an [`Action`] for the scene to carry out.

use crate::energy::{Idle, Palette, Source, Style, FIRST_FOSSIL, HEIGHT_RANGE, OPACITY_RANGE, WIDTH_RANGE};
use crate::ui::{Metrics, Panel, Rect, Tone, Ui, UiEvent, Widget, WidgetId};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "energy-style";
const CLOSE_BUTTON: WidgetId = 2;
const RESET_BUTTON: WidgetId = 3;
const PALETTE_BASE: WidgetId = 10;
const IDLE_BASE: WidgetId = 20;
const OPACITY_SLIDER: WidgetId = 30;
const HEIGHT_SLIDER: WidgetId = 31;
const WIDTH_SLIDER: WidgetId = 32;
const COLS: usize = 40;
/// Wheel notches from one end of a slider to the other.
const NOTCHES: f64 = 20.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub style: Style,
    /// Whether the bars show output, so that unused capacity shows at all.
    pub output: bool,
}

pub type Shared = Rc<RefCell<Model>>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// The style to use from now on.
    Set(Style),
}

pub fn window_panel(model: Shared) -> Panel {
    Panel::new(WINDOW, true, move |p, m| layout(p, m, &model.borrow()))
}

/// What a slider stands for: its value from a position (0 to 1) and back.
#[derive(Clone, Copy)]
enum Track {
    Opacity,
    Height,
    Width,
}

impl Track {
    fn of(widget: WidgetId) -> Option<Track> {
        match widget {
            OPACITY_SLIDER => Some(Track::Opacity),
            HEIGHT_SLIDER => Some(Track::Height),
            WIDTH_SLIDER => Some(Track::Width),
            _ => None,
        }
    }

    /// Opacity runs evenly; the factors on a log scale, so that halving and doubling are as far
    /// from the middle.
    fn value(self, pos: f64) -> f64 {
        let pos = pos.clamp(0.0, 1.0);
        match self {
            Track::Opacity => OPACITY_RANGE.0 + (OPACITY_RANGE.1 - OPACITY_RANGE.0) * pos,
            Track::Height => log_value(HEIGHT_RANGE, pos),
            Track::Width => log_value(WIDTH_RANGE, pos),
        }
    }

    fn pos(self, style: &Style) -> f64 {
        let p = match self {
            Track::Opacity => (style.idle_opacity - OPACITY_RANGE.0) / (OPACITY_RANGE.1 - OPACITY_RANGE.0),
            Track::Height => log_pos(HEIGHT_RANGE, style.height),
            Track::Width => log_pos(WIDTH_RANGE, style.width),
        };
        p.clamp(0.0, 1.0)
    }

    fn set(self, style: &mut Style, value: f64) {
        match self {
            Track::Opacity => style.idle_opacity = value,
            // Snapped to the usual size near it, so that it is easy to get back to.
            Track::Height => style.height = snap(value),
            Track::Width => style.width = snap(value),
        }
    }
}

fn log_value((lo, hi): (f64, f64), pos: f64) -> f64 {
    (lo.ln() + (hi.ln() - lo.ln()) * pos).exp()
}

fn log_pos((lo, hi): (f64, f64), v: f64) -> f64 {
    (v.max(1e-6).ln() - lo.ln()) / (hi.ln() - lo.ln())
}

fn snap(v: f64) -> f64 {
    if (v - 1.0).abs() < 0.04 {
        1.0
    } else {
        v
    }
}

/// Closes the window for its close button; anything else it was clicked or slid for comes
/// back as an action.
pub fn handle(ui: &mut Ui, model: &Shared, event: UiEvent) -> (bool, Option<Action>) {
    let mut style = model.borrow().style;
    match event {
        UiEvent::Clicked { panel: WINDOW, widget: CLOSE_BUTTON } => {
            ui.set_visible(WINDOW, false);
            return (true, None);
        }
        UiEvent::Clicked { panel: WINDOW, widget: RESET_BUTTON } => style = Style::default(),
        UiEvent::Clicked { panel: WINDOW, widget } if (PALETTE_BASE..PALETTE_BASE + Palette::ALL.len() as u32).contains(&widget) => {
            style.palette = Palette::ALL[(widget - PALETTE_BASE) as usize];
        }
        UiEvent::Clicked { panel: WINDOW, widget } if (IDLE_BASE..IDLE_BASE + Idle::ALL.len() as u32).contains(&widget) => {
            style.idle = Idle::ALL[(widget - IDLE_BASE) as usize];
        }
        UiEvent::Slid { panel: WINDOW, widget, pos, .. } => {
            let Some(track) = Track::of(widget) else { return (true, None) };
            track.set(&mut style, track.value(pos));
        }
        UiEvent::Stepped { panel: WINDOW, widget, steps } => {
            let Some(track) = Track::of(widget) else { return (true, None) };
            let pos = track.pos(&style) + steps as f64 / NOTCHES;
            track.set(&mut style, track.value(pos));
        }
        UiEvent::Clicked { panel: WINDOW, .. } => return (true, None),
        _ => return (false, None),
    }
    (true, (style != model.borrow().style).then_some(Action::Set(style)))
}

fn layout(p: &mut Panel, m: &Metrics, model: &Model) {
    let s = m.scale;
    let (glyph, step, pad) = (8.0 * s, 10.0 * s, 6.0 * s);
    let (row, margin, header) = (12.0 * s, 8.0 * s, 18.0 * s);
    let w = (m.screen.x - 2.0 * margin).min(COLS as f64 * glyph + 2.0 * pad).max(28.0 * glyph);
    let inner = w - 2.0 * pad;
    let st = &model.style;
    let label = |x: f64, y: f64, text: &str, tone: Tone| Widget::label(Rect::new(x, y, text.chars().count() as f64 * glyph, glyph), text, tone);

    let mut ws = vec![
        label(pad, (header - glyph) / 2.0, "Energy display", Tone::Heading),
        Widget::button(CLOSE_BUTTON, Rect::new(w - pad - 12.0 * s, 3.0 * s, 12.0 * s, 12.0 * s), "x", 1),
    ];
    let mut y = header;
    let heading = |ws: &mut Vec<Widget>, y: &mut f64, text: &str| {
        *y += step * 0.5;
        ws.push(label(pad, *y, text, Tone::Heading));
        *y += step * 1.2;
    };
    let slider = |ws: &mut Vec<Widget>, y: &mut f64, id: WidgetId, text: &str, pos: f64| {
        let text_w = 17.0 * glyph;
        ws.push(label(pad, *y + (row - glyph) / 2.0, text, Tone::Normal));
        ws.push(Widget::slider(id, Rect::new(pad + text_w, *y, inner - text_w, row), pos, 8.0 * s));
        *y += row + 2.0 * s;
    };

    heading(&mut ws, &mut y, "Colours");
    for (i, palette) in Palette::ALL.iter().enumerate() {
        ws.push(Widget::choice(PALETTE_BASE + i as u32, Rect::new(pad, y, inner, row), palette.name(), st.palette == *palette, true));
        y += row;
    }
    // The legend: renewable sources on the left, fossil and nuclear (hatched) on the right, each
    // with its colour running and, beside it, not running.
    y += step * 0.4;
    let col_w = inner / 2.0;
    let sw = glyph;
    let rows = FIRST_FOSSIL.max(Source::ALL.len() - FIRST_FOSSIL);
    for (i, source) in Source::ALL.iter().enumerate() {
        let (col, r) = if source.fossil() { (1.0, i - FIRST_FOSSIL) } else { (0.0, i) };
        let (x, yy) = (pad + col * col_w, y + r as f64 * (sw + 2.0 * s));
        let [cr, cg, cb] = st.colour(*source);
        ws.push(Widget::swatch(Rect::new(x, yy, sw, sw), [cr, cg, cb, 255], source.fossil()));
        if let Some(([ir, ig, ib], a)) = st.idle_colour(*source).filter(|_| model.output) {
            ws.push(Widget::swatch(Rect::new(x + sw, yy, sw, sw), [ir, ig, ib, a], source.fossil()));
        }
        let name: String = source.name().chars().take(((col_w - 3.0 * sw) / glyph) as usize).collect();
        ws.push(label(x + 2.5 * sw, yy, &name, Tone::Normal));
    }
    y += rows as f64 * (sw + 2.0 * s) + step * 0.3;
    let key = if model.output { "Left: running now. Right: not running. Hatched: fossil or nuclear." } else { "Hatched: fossil or nuclear." };
    for line in crate::text::wrap(key, (inner / glyph) as usize) {
        ws.push(label(pad, y, &line, Tone::Dim));
        y += step;
    }

    heading(&mut ws, &mut y, "Capacity not running");
    for (i, idle) in Idle::ALL.iter().enumerate() {
        ws.push(Widget::choice(IDLE_BASE + i as u32, Rect::new(pad, y, inner, row), idle.name(), st.idle == *idle, true));
        y += row;
    }
    let opacity = format!("Opacity {:>3.0}%", st.idle_opacity * 100.0);
    slider(&mut ws, &mut y, OPACITY_SLIDER, &opacity, Track::Opacity.pos(st));
    if !model.output {
        for line in crate::text::wrap("Shown while the bars show output at the clock's time (view options).", (inner / glyph) as usize) {
            ws.push(label(pad, y, &line, Tone::Dim));
            y += step;
        }
    }

    heading(&mut ws, &mut y, "Bar size");
    slider(&mut ws, &mut y, HEIGHT_SLIDER, &format!("Height x{:.2}", st.height), Track::Height.pos(st));
    slider(&mut ws, &mut y, WIDTH_SLIDER, &format!("Width  x{:.2}", st.width), Track::Width.pos(st));
    y += step * 0.5;
    let reset = "Reset";
    ws.push(Widget::button(RESET_BUTTON, Rect::new(pad, y, (reset.chars().count() + 2) as f64 * glyph, 14.0 * s), reset, 1));
    ws.push(label(pad + 8.0 * glyph, y + 3.0 * s, "Saved as you change it.", Tone::Dim));
    y += 14.0 * s + pad;

    // At the bottom right, above the icons, where the help window also opens.
    let bottom = m.floor() - 4.0 * s;
    let h = y.min(bottom - margin);
    p.rect = Rect::new(m.screen.x - margin - w, bottom - h, w, h);
    p.fixed = ws;
    p.body = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::Button;
    use crate::ui::Kind;
    use glam::DVec2;

    fn setup(model: Model) -> (Ui, Shared) {
        let shared: Shared = Rc::new(RefCell::new(model));
        let mut ui = Ui::new();
        ui.resize(DVec2::new(1280.0, 800.0), 1.0);
        ui.add(window_panel(shared.clone()));
        ui.set_visible(WINDOW, true);
        (ui, shared)
    }

    fn at(ui: &Ui, pick: impl Fn(&Widget) -> bool) -> DVec2 {
        let p = &ui.panels()[0];
        let w = p.fixed.iter().find(|w| pick(w)).expect("widget");
        DVec2::new(p.rect.x + w.rect.x + 4.0, p.rect.y + w.rect.y + w.rect.h / 2.0)
    }

    fn click(ui: &mut Ui, shared: &Shared, p: DVec2) -> Option<Action> {
        ui.pointer_moved(p);
        ui.press(Button::Left, p);
        let e = ui.release(Button::Left, p).event?;
        handle(ui, shared, e).1
    }

    #[test]
    fn choices_and_reset_set_the_style() {
        let (mut ui, shared) = setup(Model { style: Style::default(), output: true });
        let p = at(&ui, |w| matches!(&w.kind, Kind::Choice { label, .. } if label == "Colour-blind safe"));
        let Some(Action::Set(style)) = click(&mut ui, &shared, p) else { panic!() };
        assert_eq!(style.palette, Palette::ColourBlind);
        let p = at(&ui, |w| matches!(&w.kind, Kind::Choice { label, .. } if label.starts_with("Hidden")));
        let Some(Action::Set(style)) = click(&mut ui, &shared, p) else { panic!() };
        assert_eq!(style.idle, Idle::Hidden);
        shared.borrow_mut().style = Style { height: 2.0, ..Style::default() };
        ui.relayout_for_test();
        let p = at(&ui, |w| w.id == RESET_BUTTON);
        assert_eq!(click(&mut ui, &shared, p), Some(Action::Set(Style::default())));
        let p = at(&ui, |w| w.id == CLOSE_BUTTON);
        assert_eq!(click(&mut ui, &shared, p), None);
        assert!(!ui.is_visible(WINDOW));
    }

    #[test]
    fn sliders_run_over_their_ranges_and_the_wheel_steps_them() {
        assert!((Track::Height.value(0.0) - HEIGHT_RANGE.0).abs() < 1e-9 && (Track::Height.value(1.0) - HEIGHT_RANGE.1).abs() < 1e-9);
        assert!((Track::Height.value(0.5) - 1.0).abs() < 1e-9, "the usual height in the middle");
        let st = Style { width: 1.7, idle_opacity: 0.5, ..Style::default() };
        for t in [Track::Opacity, Track::Height, Track::Width] {
            let mut back = st;
            t.set(&mut back, t.value(t.pos(&st)));
            assert!((back.width - st.width).abs() < 1e-9 && (back.idle_opacity - st.idle_opacity).abs() < 1e-9);
        }
        let (mut ui, shared) = setup(Model::default());
        let (_, action) = handle(&mut ui, &shared, UiEvent::Stepped { panel: WINDOW, widget: HEIGHT_SLIDER, steps: 2 });
        let Some(Action::Set(style)) = action else { panic!() };
        assert!(style.height > 1.2, "{}", style.height);
    }

    #[test]
    fn it_fits_and_shows_a_legend_for_every_source() {
        let (ui, _) = setup(Model { style: Style::default(), output: true });
        let r = ui.panels()[0].rect;
        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 1280.0 && r.y + r.h <= 800.0, "{r:?}");
        let swatches = ui.panels()[0].fixed.iter().filter(|w| matches!(w.kind, Kind::Swatch { .. })).count();
        assert_eq!(swatches, 2 * Source::ALL.len(), "running and not running");
        let (w, h, px) = crate::ui_draw::render(&ui, 0, 1);
        assert_eq!((w as usize) * (h as usize) * 4, px.len());
    }
}
