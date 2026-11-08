//! The day chart of Germany's electricity: generation by source stacked, consumption as a line,
//! and a marker at the clock, for the clock's day (UTC), from SMARD. It shows while the energy
//! bars show output; its close button turns it off in the view options.

use crate::energy::Source;
use crate::power_feed::{Moment, FILTERS};
use crate::ui::{Chart, Metrics, Panel, Rect, Tone, Ui, UiEvent, Widget};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "power-chart";
const CLOSE_BUTTON: u32 = 2;
const COLS: usize = 50;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub title: String,
    pub chart: Option<Chart>,
    /// A line under the chart: what is shown, or why nothing is.
    pub note: String,
}

pub type Shared = Rc<RefCell<Model>>;

/// The areas from the bottom up: index into `FILTERS`, colour and name. Steady sources first, so
/// that the day's swings of wind and sun ride on top.
fn layers() -> [(usize, [u8; 3], &'static str); 12] {
    let c = |s: Source| s.colour();
    [
        (9, c(Source::Nuclear), "Nuclear"),
        (5, c(Source::Lignite), "Lignite"),
        (6, c(Source::HardCoal), "Hard coal"),
        (7, c(Source::Gas), "Natural gas"),
        (8, c(Source::Oil), "Oil, other"),
        (4, c(Source::Biomass), "Biomass"),
        (3, c(Source::Hydro), "Hydro"),
        (2, c(Source::WindOffshore), "Wind at sea"),
        (1, c(Source::WindOnshore), "Wind on land"),
        (0, c(Source::Solar), "Solar"),
        (10, [150, 215, 140], "Other renew."),
        (11, [210, 160, 220], "Pump storage"),
    ]
}

/// The chart for a day of quarter hours, `None` where SMARD has none, with the marker at
/// `marker` (0 to 1) if the clock is in the day.
pub fn chart(moments: &[Option<Moment>], marker: Option<f64>) -> Chart {
    let values = |i: usize| moments.iter().map(|m| m.as_ref().map(|m| m.mw[i])).collect::<Vec<_>>();
    // Series SMARD does not have for the day are left out of the legend.
    let layers = layers()
        .into_iter()
        .map(|(i, colour, name)| (colour, name.to_string(), values(i)))
        .filter(|l| l.2.iter().flatten().any(|&v| v > 0.0))
        .collect();
    Chart { layers, line: ("Consumption".to_string(), values(FILTERS.len() - 1)), marker, unit: ("GW".to_string(), 1000.0) }
}

pub fn window_panel(model: Shared) -> Panel {
    Panel::new(WINDOW, true, move |p, m| layout(p, m, &model.borrow()))
}

/// Returns whether the event was the close button.
pub fn handle(ui: &mut Ui, event: UiEvent) -> bool {
    if let UiEvent::Clicked { panel: WINDOW, widget: CLOSE_BUTTON } = event {
        ui.set_visible(WINDOW, false);
        return true;
    }
    false
}

fn layout(p: &mut Panel, m: &Metrics, model: &Model) {
    let s = m.scale;
    let (glyph, step, pad, margin, header) = (8.0 * s, 10.0 * s, 6.0 * s, 8.0 * s, 18.0 * s);
    let w = (m.screen.x - 2.0 * margin).min(COLS as f64 * glyph + 2.0 * pad).max(30.0 * glyph);
    let inner = w - 2.0 * pad;
    let cols = (inner / glyph) as usize;
    let note: Vec<String> = crate::text::wrap(&model.note, cols);
    let chart_h = match &model.chart {
        Some(c) => {
            let (_, rows) = c.legend_rows(inner, s);
            (130.0 * s).min(m.screen.y * 0.3) + 2.5 * glyph + rows as f64 * 11.0 * s
        }
        None => 0.0,
    };
    let h = header + chart_h + note.len() as f64 * step + 2.0 * pad;
    // At the top left, below where the search box opens.
    p.rect = Rect::new(margin, margin + 22.0 * s, w, h);
    let title: String = model.title.chars().take(cols.saturating_sub(3)).collect();
    p.fixed = vec![
        Widget::label(Rect::new(pad, (header - glyph) / 2.0, title.chars().count() as f64 * glyph, glyph), &title, Tone::Heading),
        Widget::button(CLOSE_BUTTON, Rect::new(w - pad - 12.0 * s, 3.0 * s, 12.0 * s, 12.0 * s), "x", 1),
    ];
    let mut y = header + pad;
    if let Some(c) = &model.chart {
        p.fixed.push(Widget::chart(Rect::new(pad, y, inner, chart_h), c.clone()));
        y += chart_h;
    }
    for line in &note {
        p.fixed.push(Widget::label(Rect::new(pad, y, line.chars().count() as f64 * glyph, glyph), line, Tone::Dim));
        y += step;
    }
    p.body = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::Button;
    use glam::DVec2;

    fn moment(solar: f32, load: f32) -> Moment {
        let mut mw = vec![0.0; FILTERS.len()];
        mw[0] = solar;
        mw[5] = 5000.0;
        mw[FILTERS.len() - 1] = load;
        Moment { time: 0.0, mw }
    }

    #[test]
    fn the_chart_holds_the_sources_that_ran_and_the_consumption() {
        let day: Vec<Option<Moment>> = (0..96).map(|i| (i < 60).then(|| moment(i as f32 * 100.0, 40_000.0))).collect();
        let c = chart(&day, Some(0.5));
        let names: Vec<&str> = c.layers.iter().map(|l| l.1.as_str()).collect();
        assert_eq!(names, ["Lignite", "Solar"], "stacked steady first, and only what ran");
        assert_eq!(c.line.1.len(), 96);
        assert_eq!(c.line.1[10], Some(40_000.0));
        assert_eq!(c.line.1[80], None);
        assert_eq!(c.layers[1].2[10], Some(1000.0));
    }

    #[test]
    fn the_window_draws_and_closes() {
        let model: Shared = Rc::new(RefCell::new(Model {
            title: "Germany".into(),
            chart: Some(chart(&vec![Some(moment(20_000.0, 50_000.0)); 96], Some(0.25))),
            note: "A note".into(),
        }));
        let mut ui = Ui::new();
        ui.resize(DVec2::new(1280.0, 800.0), 1.0);
        ui.add(window_panel(model));
        ui.set_visible(WINDOW, true);
        let r = ui.panels()[0].rect;
        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= 1280.0 && r.y + r.h <= 800.0, "{r:?}");
        let (w, h, px) = crate::ui_draw::render(&ui, 0, 1);
        assert_eq!((w as usize) * (h as usize) * 4, px.len());
        // Somewhere the solar colour is painted.
        let sun = Source::Solar.colour();
        assert!(px.chunks(4).any(|p| p[0].abs_diff(sun[0]) < 12 && p[1].abs_diff(sun[1]) < 12 && p[2].abs_diff(sun[2]) < 12));
        let close = DVec2::new(r.x + r.w - 6.0 - 6.0, r.y + 9.0);
        ui.press(Button::Left, close);
        let e = ui.release(Button::Left, close).event.unwrap();
        assert!(handle(&mut ui, e));
        assert!(!ui.is_visible(WINDOW));
    }
}
