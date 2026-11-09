//! The day chart of Germany's electricity: generation by source stacked, consumption as a line,
//! and a marker at the clock, for the clock's day (UTC), from SMARD. It shows while the energy
//! bars show output; its close button turns it off in the view options.
//!
//! While another country is selected (its bar, or an information box about it), the window shows
//! that country's months instead, from the Ember figures of `owid.rs` built into the program, or
//! its years where Ember has no months.

use crate::energy::{Palette, Source};
use crate::power_feed::{Moment, FILTERS};
use crate::gui::{self, Gui};
use crate::owid::{Country, Series, CREDIT};
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
    /// A thin line on an axis of its own at the right: the carbon intensity in g per kWh.
    pub intensity: Option<Vec<Option<f32>>>,
    /// How far apart the lines of the left axis are, in values.
    pub tick: f32,
    /// The labels under the plot: where (0 to 1 across it) and what.
    pub labels: Vec<(f64, String)>,
    /// Whether each step is a bar of its own with a gap, as months and years are, rather than
    /// part of an area.
    pub bars: bool,
}

/// The colour of the intensity line and its axis.
const INTENSITY: Color32 = Color32::from_rgb(255, 160, 140);

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
pub fn chart(moments: &[Option<Moment>], intensity: Option<Vec<Option<f32>>>, marker: Option<f64>, palette: Palette) -> Chart {
    let values = |i: usize| moments.iter().map(|m| m.as_ref().map(|m| m.mw[i])).collect::<Vec<_>>();
    // Series SMARD does not have for the day are left out of the legend.
    let layers = layers(palette)
        .into_iter()
        .map(|(i, colour, name, hatched)| (colour, name.to_string(), values(i), hatched))
        .filter(|l| l.2.iter().flatten().any(|&v| v > 0.0))
        .collect();
    let intensity = intensity.filter(|v| v.iter().flatten().any(|&g| g > 0.0));
    let labels = (0..=24).step_by(3).map(|h| (h as f64 / 24.0, format!("{h:02}"))).collect();
    Chart {
        layers,
        line: ("Consumption".to_string(), values(FILTERS.len() - 1)),
        marker,
        unit: ("GW".to_string(), 1000.0),
        intensity,
        // The axis goes up in whole steps of 10 GW.
        tick: 10_000.0,
        labels,
        bars: false,
    }
}

/// How many months a country's chart shows, and how many years where Ember has no months.
pub const MONTHS_SHOWN: i32 = 24;
pub const YEARS_SHOWN: i32 = 10;

/// What a selected country's chart can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// Ember's months, up to and including this one.
    Months(i32, u32),
    /// Only years, up to and including this one.
    Years(i32),
    /// No figures at all.
    Nothing,
}

/// The months up to the clock's month where Ember has them (the latest ones when the clock is
/// before them), else the years up to the clock's year in the same way.
pub fn span(c: Option<&Country>, (year, month): (i32, u32)) -> Span {
    let Some(c) = c else { return Span::Nothing };
    let g = Series::Generation;
    if c.has_monthly() {
        if let Some(((y, m), _)) = c.latest_month(g, year, month).or_else(|| c.latest_month(g, 9999, 12)) {
            return Span::Months(y, m);
        }
    }
    match c.latest_year(g, year).map(|(y, _)| y).or_else(|| c.years(g).map(|(_, last)| last)) {
        Some(y) => Span::Years(y),
        None => Span::Nothing,
    }
}

/// A country's sources from the bottom up: the series, the entry of [`layers`] whose colour and
/// hatching it takes, and its name. What the total has beyond them goes with oil.
const COUNTRY_LAYERS: [(Series, usize, &str); 9] = [
    (Series::Nuclear, 0, "Nuclear"),
    (Series::Coal, 2, "Coal"),
    (Series::Gas, 3, "Gas"),
    (Series::Oil, 4, "Oil, other"),
    (Series::Bioenergy, 5, "Bioenergy"),
    (Series::Hydro, 6, "Hydro"),
    (Series::Wind, 8, "Wind"),
    (Series::Solar, 9, "Solar"),
    (Series::OtherRenewables, 10, "Other renew."),
];
/// Where in [`COUNTRY_LAYERS`] the rest of the total goes.
const REST_LAYER: usize = 3;
/// The rest counts once it is this share of the total; below, it is rounding.
const REST_SHARE: f64 = 0.005;

const MONTH_NAMES: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A step from 1, 2 or 5 times a power of ten that puts about four lines under `top`.
fn nice_tick(top: f32) -> f32 {
    if top <= 0.0 || !top.is_finite() {
        return 1.0;
    }
    let raw = top / 4.0;
    let pow = 10f32.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|k| k * pow).find(|&t| t >= raw).unwrap_or(10.0 * pow)
}

/// A country's chart over `span` in TWh: generation stacked by source as bars, demand as the line
/// and the carbon intensity at the right, with the marker on the clock's month or year.
/// `None` for [`Span::Nothing`].
pub fn country_chart(c: &Country, span: Span, (year, month): (i32, u32), palette: Palette) -> Option<Chart> {
    // The steps as (year, month), month 0 for a whole year.
    let steps: Vec<(i32, u32)> = match span {
        Span::Months(y, m) => {
            let end = y * 12 + m as i32 - 1;
            (end - MONTHS_SHOWN + 1..=end).map(|t| (t.div_euclid(12), t.rem_euclid(12) as u32 + 1)).collect()
        }
        Span::Years(y) => (y - YEARS_SHOWN + 1..=y).map(|y| (y, 0)).collect(),
        Span::Nothing => return None,
    };
    let get = |s: Series, (y, m): (i32, u32)| if m == 0 { c.year(s, y) } else { c.month(s, y, m) };
    let all = layers(palette);
    let mut values: Vec<Vec<Option<f32>>> = COUNTRY_LAYERS.iter().map(|&(s, _, _)| steps.iter().map(|&t| get(s, t).map(|v| v.max(0.0) as f32)).collect()).collect();
    for (k, &t) in steps.iter().enumerate() {
        let Some(total) = get(Series::Generation, t) else { continue };
        let sum: f64 = COUNTRY_LAYERS.iter().filter_map(|&(s, _, _)| get(s, t)).map(|v| v.max(0.0)).sum();
        let rest = total - sum;
        if rest > REST_SHARE * total {
            let v = &mut values[REST_LAYER][k];
            *v = Some(v.unwrap_or(0.0) + rest as f32);
        }
    }
    let layers: Vec<ChartLayer> = COUNTRY_LAYERS
        .iter()
        .zip(values)
        .map(|(&(_, i, name), v)| (all[i].1, name.to_string(), v, all[i].3))
        .filter(|l| l.2.iter().flatten().any(|&v| v > 0.0))
        .collect();
    let line: Vec<Option<f32>> = steps.iter().map(|&t| get(Series::Demand, t).map(|v| v as f32)).collect();
    let intensity: Vec<Option<f32>> = steps.iter().map(|&t| get(Series::CarbonIntensity, t).map(|v| v as f32)).collect();
    let n = steps.len() as f64;
    let centre = |k: usize| (k as f64 + 0.5) / n;
    let now = if matches!(span, Span::Years(_)) { (year, 0) } else { (year, month) };
    let marker = steps.iter().position(|&t| t == now).map(centre);
    let labels = steps
        .iter()
        .enumerate()
        .filter_map(|(k, &(y, m))| match m {
            0 => ((steps.len() - 1 - k).is_multiple_of(2) || steps.len() <= 6).then(|| (centre(k), y.to_string())),
            1 => Some((centre(k), y.to_string())),
            7 => Some((centre(k), MONTH_NAMES[6].to_string())),
            _ => None,
        })
        .collect();
    let top = (0..steps.len())
        .map(|k| {
            let stack: f32 = layers.iter().filter_map(|l| l.2[k]).sum();
            stack.max(line[k].unwrap_or(0.0))
        })
        .fold(0.0f32, f32::max);
    Some(Chart {
        layers,
        line: ("Demand".to_string(), line),
        marker,
        unit: ("TWh".to_string(), 1.0),
        intensity: Some(intensity).filter(|v| v.iter().flatten().any(|&g| g > 0.0)),
        tick: nice_tick(top),
        labels,
        bars: true,
    })
}

/// The window's content for a selected country other than Germany: its months, or its years
/// where Ember has no months, or a word that there is nothing.
pub fn country_model(c: Option<&Country>, name: &str, clock: (i32, u32), palette: Palette) -> Model {
    let span = span(c, clock);
    let chart = c.and_then(|c| country_chart(c, span, clock, palette));
    let (title, note) = match span {
        Span::Months(y, m) => (format!("{name}, monthly, TWh"), format!("Each bar a month's generation, to {} {y}. {CREDIT}.", MONTH_NAMES[m as usize - 1])),
        Span::Years(y) => (format!("{name}, yearly, TWh"), format!("Ember has no monthly figures for {name}: each bar a year's generation, to {y}. {CREDIT}.")),
        Span::Nothing => (format!("{name}, electricity"), format!("No electricity figures for {name}.")),
    };
    Model { title, chart, note }
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
/// The gap between the window and the edges of the screen.
pub const MARGIN: f32 = 12.0;

/// How wide the window opens on a screen this wide.
pub fn default_width(screen: f32) -> f32 {
    (screen * 0.25).clamp(260.0, 360.0)
}

pub fn show(ctx: &egui::Context, gui: &mut Gui, model: &Model) -> Option<Event> {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return None;
    }
    let mut event = None;
    let screen = ctx.content_rect();
    let shown = gui::window(&model.title, WINDOW)
        .open(&mut open)
        // At the top left, with the energy card beside it.
        .default_pos(egui::pos2(screen.left() + MARGIN, screen.top() + MARGIN))
        .default_width(default_width(screen.width()))
        .show(ctx, |ui| {
            let width = ui.available_width();
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
    let right = if c.intensity.is_some() { 30.0 } else { 4.0 };
    let plot = Rect::from_min_max(Pos2::new(r.left() + 28.0, r.top() + 6.0), Pos2::new(r.right() - right, r.bottom() - 18.0));
    let p = ui.painter().with_clip_rect(r);
    let steps = c.line.1.len().max(c.layers.iter().map(|l| l.2.len()).max().unwrap_or(0)).max(1);
    let stack = |i: usize| c.layers.iter().map(|l| l.2.get(i).copied().flatten().unwrap_or(0.0)).sum::<f32>();
    let top = (0..steps).map(|i| stack(i).max(c.line.1.get(i).copied().flatten().unwrap_or(0.0))).fold(0.0f32, f32::max);
    let tick = c.tick.max(f32::MIN_POSITIVE);
    let max = (top / tick).ceil().max(1.0) * tick;
    // As many decimals as the step needs.
    let decimals = (-(tick / c.unit.1).log10().floor()).clamp(0.0, 3.0) as usize;
    let y_of = |v: f32| plot.bottom() - v / max * plot.height();
    let x_of = |i: f32| plot.left() + i / steps as f32 * plot.width();
    p.rect_filled(plot, 3.0, v.extreme_bg_color.gamma_multiply(0.5));
    let mut t = 0.0;
    while t <= max + 1.0 {
        let y = y_of(t);
        p.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0, grid));
        p.text(Pos2::new(plot.left() - 4.0, y), egui::Align2::RIGHT_CENTER, format!("{:.*}", decimals, t / c.unit.1), small.clone(), weak);
        t += tick;
    }
    // The areas, or the bars, one step at a time.
    let gap = if c.bars { (plot.width() / steps as f32 * 0.12).min(3.0) } else { 0.0 };
    for i in 0..steps {
        let (x0, x1) = if c.bars { (x_of(i as f32) + gap, x_of(i as f32 + 1.0) - gap) } else { (x_of(i as f32), x_of(i as f32 + 1.0) + 0.5) };
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
    // The intensity on its own axis, in steps of 100 g with as many steps as the left axis has.
    if let Some(g) = &c.intensity {
        let ticks = (max / tick).round().max(1.0);
        let top = g.iter().flatten().fold(0.0f32, |a, &b| a.max(b));
        let step = (top / ticks / 100.0).ceil().max(1.0) * 100.0;
        let gmax = step * ticks;
        let gy = |v: f32| plot.bottom() - v / gmax * plot.height();
        for k in 0..=ticks as usize {
            let v = k as f32 * step;
            p.text(Pos2::new(plot.right() + 3.0, gy(v)), egui::Align2::LEFT_CENTER, format!("{v:.0}"), small.clone(), INTENSITY.gamma_multiply(0.85));
        }
        let mut run: Vec<Pos2> = Vec::new();
        let flush = |run: &mut Vec<Pos2>| {
            if run.len() > 1 {
                // A dark edge keeps the thin line apart from the areas under it.
                p.add(egui::Shape::line(run.clone(), Stroke::new(3.0, Color32::from_black_alpha(150))));
                p.add(egui::Shape::line(std::mem::take(run), Stroke::new(1.4, INTENSITY)));
            }
            run.clear();
        };
        for i in 0..g.len() {
            match g[i] {
                Some(v) => run.push(Pos2::new(plot.left() + (i as f32 + 0.5) / g.len() as f32 * plot.width(), gy(v))),
                None => flush(&mut run),
            }
        }
        flush(&mut run);
    }
    if let Some(m) = c.marker {
        let x = plot.left() + m.clamp(0.0, 1.0) as f32 * plot.width();
        p.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.5, Color32::from_rgb(255, 214, 70)));
    }
    for (at, text) in &c.labels {
        let x = plot.left() + plot.width() * *at as f32;
        p.text(Pos2::new(x, plot.bottom() + 3.0), egui::Align2::CENTER_TOP, text, small.clone(), weak);
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
        if c.intensity.is_some() {
            let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), egui::Sense::hover());
            ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.2, INTENSITY));
            ui.label(egui::RichText::new("g CO\u{2082}/kWh, right").small());
            ui.add_space(8.0);
        }
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
        let c = chart(&day, None, Some(0.5), Palette::Standard);
        let names: Vec<&str> = c.layers.iter().map(|l| l.1.as_str()).collect();
        assert_eq!(names, ["Lignite", "Solar"], "stacked steady first, and only what ran");
        assert_eq!(c.line.1.len(), 96);
        assert_eq!(c.line.1[10], Some(40_000.0));
        assert_eq!(c.line.1[80], None);
        assert_eq!(c.layers[1].2[10], Some(1000.0));
        assert!(c.intensity.is_none());
        let g: Vec<Option<f32>> = day.iter().map(|m| m.as_ref().and_then(|m| m.intensity())).collect();
        let c = chart(&day, Some(g), Some(0.5), Palette::Standard);
        assert_eq!(c.intensity.as_ref().unwrap()[0], Some(820.0), "lignite alone at midnight");
    }

    use crate::owid::Owid;

    fn fixture() -> Owid {
        Owid::parse(include_bytes!("../tests/fixtures/owid/owid_energy.bin")).unwrap()
    }

    #[test]
    fn a_country_with_months_gets_months_one_without_years_and_none_nothing() {
        let o = fixture();
        let (de, af) = (o.country("DEU").unwrap(), o.country("AFG").unwrap());
        assert_eq!(span(Some(de), (2025, 6)), Span::Months(2025, 6));
        // Past the last month, the latest; before the first, the latest too.
        let Span::Months(y, m) = span(Some(de), (2100, 1)) else { panic!() };
        assert!(y > 2025 && (1..=12).contains(&m));
        assert_eq!(span(Some(de), (1900, 1)), Span::Months(y, m));
        assert_eq!(span(Some(af), (2024, 6)), Span::Years(2024));
        assert_eq!(span(None, (2024, 6)), Span::Nothing);
    }

    #[test]
    fn the_months_of_a_country_stack_its_sources_with_demand_and_intensity() {
        let o = fixture();
        let de = o.country("DEU").unwrap();
        let c = country_chart(de, Span::Months(2025, 12), (2025, 6), Palette::Standard).unwrap();
        assert!(c.bars);
        assert_eq!(c.unit.0, "TWh");
        assert_eq!(c.line.1.len(), MONTHS_SHOWN as usize);
        assert!(c.layers.iter().all(|l| l.2.len() == MONTHS_SHOWN as usize));
        let names: Vec<&str> = c.layers.iter().map(|l| l.1.as_str()).collect();
        assert!(names.contains(&"Solar") && names.contains(&"Coal") && names.contains(&"Wind"));
        // Germany shut its last reactors in April 2023, before these months.
        assert!(!names.contains(&"Nuclear"));
        // The colours are those of the day chart.
        let solar = c.layers.iter().find(|l| l.1 == "Solar").unwrap();
        assert_eq!(solar.0, Palette::Standard.colour(Source::Solar));
        assert!(c.layers.iter().find(|l| l.1 == "Coal").unwrap().3, "fossil hatched");
        // The last step is December 2025: the sources sum to the month's generation.
        let stack: f32 = c.layers.iter().filter_map(|l| l.2[23]).sum();
        let total = de.month(Series::Generation, 2025, 12).unwrap() as f32;
        assert!((stack - total).abs() < 0.01 * total, "{stack} {total}");
        assert_eq!(c.line.1[23], de.month(Series::Demand, 2025, 12).map(|v| v as f32));
        assert!(c.intensity.as_ref().unwrap()[23].unwrap() > 100.0);
        // June 2025 is the 18th of the 24 months from January 2024.
        assert_eq!(c.marker, Some(17.5 / 24.0));
        let labels: Vec<&str> = c.labels.iter().map(|l| l.1.as_str()).collect();
        assert_eq!(labels, ["2024", "Jul", "2025", "Jul"]);
        assert!(c.tick > 0.0 && c.tick < 50.0);
    }

    #[test]
    fn a_country_with_only_years_gets_ten_years() {
        let o = fixture();
        let af = o.country("AFG").unwrap();
        let m = country_model(Some(af), "Afghanistan", (2024, 6), Palette::Standard);
        assert_eq!(m.title, "Afghanistan, yearly, TWh");
        assert!(m.note.contains("no monthly") && m.note.contains(CREDIT));
        let c = m.chart.unwrap();
        assert_eq!(c.line.1.len(), YEARS_SHOWN as usize);
        assert_eq!(c.marker, Some(9.5 / 10.0));
        assert!(c.layers.iter().any(|l| l.2[9].is_some_and(|v| v > 0.0)));
        let m = country_model(None, "Atlantis", (2024, 6), Palette::Standard);
        assert_eq!((m.title.as_str(), m.chart), ("Atlantis, electricity", None));
        assert!(m.note.starts_with("No electricity"));
        let m = country_model(o.country("DEU"), "Germany", (2025, 6), Palette::Standard);
        assert_eq!(m.title, "Germany, monthly, TWh");
        assert!(m.note.contains("to Jun 2025"));
    }

    #[test]
    fn ticks_are_round() {
        assert_eq!(nice_tick(14.0), 5.0);
        assert!((nice_tick(0.3) - 0.1).abs() < 1e-6);
        assert_eq!(nice_tick(40.0), 10.0);
        assert_eq!(nice_tick(0.0), 1.0);
    }
}
