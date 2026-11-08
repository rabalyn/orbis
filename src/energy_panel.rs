//! The energy card at the top right while the energy bars are on: what one bar stands for, a
//! slider with fixed stops for the level of detail, the scale, and the figures at the clock's
//! time. In automatic mode the stop in effect is lit, so it is always clear that more detail
//! exists and how to get it.

use crate::energy::{Detail, Level};
use crate::gui::{self, Gui};
use egui::{Pos2, Rect, Sense, Stroke, Vec2};

pub const WINDOW: &str = "energy-card";

/// What the card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub detail: Detail,
    /// The level shown now.
    pub level: Level,
    /// Bars per level with all sources.
    pub counts: [usize; 4],
    pub month: String,
    /// The height of 1 GW, in words.
    pub scale: String,
    /// The figures at the clock's time, if output is shown.
    pub output: Option<String>,
    /// Further lines, such as about units without a position.
    pub notes: Vec<String>,
    /// Whether the bars' heights can be seen: not in a view straight down.
    pub flat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Detail(Detail),
    Style,
}

/// The card, while the bars are on.
/// The card can be closed on its own; it comes back when the bars are switched on again.
pub fn show(ctx: &egui::Context, gui: &mut Gui, m: &Model) -> Option<Action> {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return None;
    }
    let mut action = None;
    let screen = ctx.content_rect();
    let shown = gui::window("Renewable energy, Germany", WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.right() - 330.0, screen.top() + 50.0))
        .default_width(290.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Detail");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut auto = m.detail == Detail::Auto;
                    if ui.checkbox(&mut auto, "Follow zoom").on_hover_text(crate::energy::auto_bands()).changed() {
                        action = Some(Action::Detail(if auto { Detail::Auto } else { Detail::Fixed(m.level) }));
                    }
                });
            });
            if let Some(level) = stops(ui, m) {
                action = Some(Action::Detail(Detail::Fixed(level)));
            }
            let count = m.counts[m.level.index()];
            let what = match m.level {
                Level::States => "One bar per state",
                Level::Districts => "One bar per district",
                Level::Towns => "One bar per municipality",
                Level::Plants => "One bar per plant site",
            };
            ui.label(format!("{what}  \u{00b7}  {} in all", group(count)));
            if m.detail == Detail::Auto {
                gui::note(ui, "Zoom in for more detail, or pick a level.");
            } else {
                gui::note(ui, "Held at this level while you zoom.");
            }
            ui.separator();
            egui::Grid::new("energy-figures").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
                ui.label(egui::RichText::new("Built by").weak());
                ui.label(&m.month);
                ui.end_row();
                ui.label(egui::RichText::new("Height").weak());
                ui.label(format!("1 GW = {}", m.scale));
                ui.end_row();
            });
            if m.flat {
                gui::note(ui, "Tilt the view to see the bars' heights.");
            }
            if let Some(o) = &m.output {
                ui.add(egui::Label::new(o).wrap());
            }
            for n in &m.notes {
                gui::note(ui, n);
            }
            if ui.button("Colours and bar size\u{2026}").clicked() {
                action = Some(Action::Style);
            }
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    gui.set_visible(WINDOW, open);
    action
}

/// The stops of the detail slider; a click or drag on one holds that level.
fn stops(ui: &mut egui::Ui, m: &Model) -> Option<Level> {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 44.0), Sense::click_and_drag());
    let n = Level::ALL.len();
    let inset = 22.0;
    let x_of = |i: usize| rect.left() + inset + (rect.width() - 2.0 * inset) * i as f32 / (n - 1) as f32;
    let y = rect.top() + 10.0;
    let v = ui.visuals();
    let p = ui.painter();
    let current = m.level.index();
    p.line_segment([Pos2::new(x_of(0), y), Pos2::new(x_of(n - 1), y)], Stroke::new(3.0, v.widgets.inactive.bg_fill.gamma_multiply(1.6)));
    p.line_segment([Pos2::new(x_of(0), y), Pos2::new(x_of(current), y)], Stroke::new(3.0, gui::accent()));
    for (i, level) in Level::ALL.iter().enumerate() {
        let at = Pos2::new(x_of(i), y);
        let on = i == current;
        if on {
            p.circle(at, 7.0, gui::accent(), Stroke::new(2.0, v.window_fill));
        } else {
            p.circle(at, 4.0, if i < current { gui::accent() } else { v.widgets.inactive.bg_fill.gamma_multiply(1.8) }, Stroke::NONE);
        }
        let label = Rect::from_center_size(Pos2::new(at.x, rect.bottom() - 12.0), Vec2::new(80.0, 20.0));
        let colour = if on { v.text_color() } else { v.weak_text_color() };
        let name = if *level == Level::Towns { "Municip." } else { level.plural().split(' ').next().unwrap_or("") };
        p.text(label.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(11.5), colour);
    }
    let pointer = response.interact_pointer_pos()?;
    if !(response.clicked() || response.dragged()) {
        return None;
    }
    let nearest = (0..n).min_by(|a, b| (x_of(*a) - pointer.x).abs().total_cmp(&(x_of(*b) - pointer.x).abs()))?;
    (nearest != current || m.detail == Detail::Auto).then_some(Level::ALL[nearest])
}

/// A count with thin spaces between thousands.
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(group(16), "16");
        assert_eq!(group(10_789), "10\u{2009}789");
        assert_eq!(group(1_234_567), "1\u{2009}234\u{2009}567");
    }

    #[test]
    fn details_read_back_and_fixed_levels_ignore_the_altitude() {
        for d in [Detail::Auto, Detail::Fixed(Level::States), Detail::Fixed(Level::Districts), Detail::Fixed(Level::Towns), Detail::Fixed(Level::Plants)] {
            assert_eq!(Detail::parse(d.key()), Some(d));
        }
        assert_eq!(Detail::Fixed(Level::Towns).level(5e6, None), Level::Towns);
        assert_eq!(Detail::Auto.level(5e6, None), Level::States);
    }
}
