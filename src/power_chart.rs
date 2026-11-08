//! The day chart of Germany's electricity: generation by source stacked, consumption as a line,
//! and a marker at the clock, for the clock's day (UTC), from SMARD. It shows while the energy
//! bars show output; its close button turns it off in the view options.

use crate::energy::{Palette, Source};
use crate::power_feed::{Moment, FILTERS};
use crate::gui::{self, Gui};
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "power-chart";

/// One stacked area of a [`Chart`]: colour, name, values, and whether it is hatched rather than solid.
pub type ChartLayer = ([u8; 3], String, Vec<Option<f32>>, bool);

/// A day of stacked areas with a line over them: values per equal step from midnight to
/// midnight, `None` where there is none.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// Stacked from the bottom.
    pub layers: Vec<ChartLayer>,
    /// Drawn over the areas: name and values.
    pub line: (String, Vec<Option<f32>>),
    /// Where a vertical marker stands, from 0 (midnight) to 1 (the next midnight).
    pub marker: Option<f64>,
    /// The values' unit for the axis, and how many values one unit is: ("GW", 1000.0) for MW.
    pub unit: (String, f32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub title: String,
    pub chart: Option<Chart>,
    /// A line under the chart: what is shown, or why nothing is.
    pub note: String,
}

pub type Shared = Rc<RefCell<Model>>;

/// The areas from the bottom up: index into `FILTERS`, colour, name, and whether hatched (the
/// fossil and nuclear ones, as their bars are dashed). Steady sources first, so that the day's
/// swings of wind and sun ride on top.
fn layers(palette: Palette) -> [(usize, [u8; 3], &'static str, bool); 12] {
    let c = |s: Source| palette.colour(s);
    [
        (9, c(Source::Nuclear), "Nuclear", true),
        (5, c(Source::Lignite), "Lignite", true),
        (6, c(Source::HardCoal), "Hard coal", true),
        (7, c(Source::Gas), "Natural gas", true),
        (8, c(Source::Oil), "Oil, other", true),
        (4, c(Source::Biomass), "Biomass", false),
        (3, c(Source::Hydro), "Hydro", false),
        (2, c(Source::WindOffshore), "Wind at sea", false),
        (1, c(Source::WindOnshore), "Wind on land", false),
        (0, c(Source::Solar), "Solar", false),
        (10, [150, 215, 140], "Other renew.", false),
        (11, [210, 160, 220], "Pump storage", false),
    ]
}

/// The chart for a day of quarter hours, `None` where SMARD has none, with the marker at
/// `marker` (0 to 1) if the clock is in the day.
pub fn chart(moments: &[Option<Moment>], marker: Option<f64>, palette: Palette) -> Chart {
    let values = |i: usize| moments.iter().map(|m| m.as_ref().map(|m| m.mw[i])).collect::<Vec<_>>();
    // Series SMARD does not have for the day are left out of the legend.
    let layers = layers(palette)
        .into_iter()
        .map(|(i, colour, name, hatched)| (colour, name.to_string(), values(i), hatched))
        .filter(|l| l.2.iter().flatten().any(|&v| v > 0.0))
        .collect();
    Chart { layers, line: ("Consumption".to_string(), values(FILTERS.len() - 1)), marker, unit: ("GW".to_string(), 1000.0) }
}

/// What a click on the chart's buttons asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The close button: the chart is hidden and is to stay off.
    Closed,
    /// The style button: open the energy display window.
    Style,
}

/// The chart window, while it is open.
pub fn show(ctx: &egui::Context, gui: &mut Gui, model: &Model) -> Option<Event> {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return None;
    }
    let mut event = None;
    let screen = ctx.content_rect();
    let width = (screen.width() - 40.0).clamp(260.0, 440.0);
    let shown = gui::window(&model.title, WINDOW)
        .open(&mut open)
        // Low on the left, above the credits, clear of the view options at the top.
        .pivot(egui::Align2::LEFT_BOTTOM)
        .default_pos(egui::pos2(screen.left() + 12.0, screen.bottom() - 150.0))
        .show(ctx, |ui| {
            ui.set_width(width);
            if let Some(c) = &model.chart {
                let height = (screen.height() * 0.25).clamp(110.0, 170.0);
                plot(ui, c, Vec2::new(width, height));
                legend(ui, c);
            }
            if !model.note.is_empty() {
                gui::note(ui, &model.note);
            }
            if ui.button("Colours and bar size\u{2026}").clicked() {
                event = Some(Event::Style);
            }
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    if !open {
        event = Some(Event::Closed);
    }
    gui.set_visible(WINDOW, open);
    event
}

/// The stacked areas, the line, the axes and the clock's marker.
fn plot(ui: &mut egui::Ui, c: &Chart, size: Vec2) {
    let (r, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let v = ui.visuals();
    let (ink, weak, grid) = (v.text_color(), v.weak_text_color(), v.widgets.noninteractive.bg_stroke.color);
    let small = egui::FontId::proportional(11.0);
    let plot = Rect::from_min_max(Pos2::new(r.left() + 28.0, r.top() + 6.0), Pos2::new(r.right() - 4.0, r.bottom() - 18.0));
    let p = ui.painter().with_clip_rect(r);
    let steps = c.line.1.len().max(c.layers.iter().map(|l| l.2.len()).max().unwrap_or(0)).max(1);
    let stack = |i: usize| c.layers.iter().map(|l| l.2.get(i).copied().flatten().unwrap_or(0.0)).sum::<f32>();
    let top = (0..steps).map(|i| stack(i).max(c.line.1.get(i).copied().flatten().unwrap_or(0.0))).fold(0.0f32, f32::max);
    // The axis goes up in whole steps of 10 units.
    let tick = 10.0 * c.unit.1;
    let max = (top / tick).ceil().max(1.0) * tick;
    let y_of = |v: f32| plot.bottom() - v / max * plot.height();
    let x_of = |i: f32| plot.left() + i / steps as f32 * plot.width();
    p.rect_filled(plot, 3.0, v.extreme_bg_color.gamma_multiply(0.5));
    let mut t = 0.0;
    while t <= max + 1.0 {
        let y = y_of(t);
        p.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0, grid));
        p.text(Pos2::new(plot.left() - 4.0, y), egui::Align2::RIGHT_CENTER, format!("{:.0}", t / c.unit.1), small.clone(), weak);
        t += tick;
    }
    // The areas, one step at a time.
    for i in 0..steps {
        let (x0, x1) = (x_of(i as f32), x_of(i as f32 + 1.0) + 0.5);
        let mut base = 0.0f32;
        for (colour, _, values, hatched) in &c.layers {
            let Some(v) = values.get(i).copied().flatten() else { continue };
            let cell = Rect::from_min_max(Pos2::new(x0, y_of(base + v)), Pos2::new(x1, y_of(base)));
            p.rect_filled(cell, 0.0, Color32::from_rgb(colour[0], colour[1], colour[2]));
            if *hatched {
                gui::hatch(&p.with_clip_rect(cell), cell, *colour);
            }
            base += v;
        }
    }
    // The line, broken where values are missing.
    let mut run: Vec<Pos2> = Vec::new();
    let flush = |run: &mut Vec<Pos2>| {
        if run.len() > 1 {
            p.add(egui::Shape::line(std::mem::take(run), Stroke::new(2.0, ink)));
        }
        run.clear();
    };
    for i in 0..steps {
        match c.line.1.get(i).copied().flatten() {
            Some(v) => run.push(Pos2::new(x_of(i as f32 + 0.5), y_of(v))),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    if let Some(m) = c.marker {
        let x = plot.left() + m.clamp(0.0, 1.0) as f32 * plot.width();
        p.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.5, Color32::from_rgb(255, 214, 70)));
    }
    for h in (0..=24).step_by(3) {
        let x = plot.left() + plot.width() * h as f32 / 24.0;
        p.text(Pos2::new(x, plot.bottom() + 3.0), egui::Align2::CENTER_TOP, format!("{h:02}"), small.clone(), weak);
    }
}

/// The legend: the line first, then the areas from the top of the stack down.
fn legend(ui: &mut egui::Ui, c: &Chart) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 2.0);
        let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), egui::Sense::hover());
        ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(2.0, ui.visuals().text_color()));
        ui.label(egui::RichText::new(&c.line.0).small());
        ui.add_space(8.0);
        for (colour, name, _, hatched) in c.layers.iter().rev() {
            gui::legend_entry(ui, [colour[0], colour[1], colour[2], 255], *hatched, name);
            ui.add_space(6.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let c = chart(&day, Some(0.5), Palette::Standard);
        let names: Vec<&str> = c.layers.iter().map(|l| l.1.as_str()).collect();
        assert_eq!(names, ["Lignite", "Solar"], "stacked steady first, and only what ran");
        assert_eq!(c.line.1.len(), 96);
        assert_eq!(c.line.1[10], Some(40_000.0));
        assert_eq!(c.line.1[80], None);
        assert_eq!(c.layers[1].2[10], Some(1000.0));
    }
}
