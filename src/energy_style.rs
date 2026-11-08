//! The energy display window: the colours of the sources, how capacity that is not running
//! shows, and the size of the bars. It opens from the view options and from the day chart, and
//! every change is saved with the other settings.
//!
//! It shows a [`Model`] shared with the scene; a click or a slider comes back from [`show`] as
//! an [`Action`] for the scene to carry out.

use crate::energy::{Idle, Palette, Source, Style, HEIGHT_RANGE, OPACITY_RANGE, WIDTH_RANGE};
use crate::gui::{self, Gui, Slide};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "energy-style";
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

/// What a slider stands for: its value from a position (0 to 1) and back.
#[derive(Clone, Copy)]
enum Track {
    Opacity,
    Height,
    Width,
}

impl Track {
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

/// The window, while it is open; a change comes back as the new style.
pub fn show(ctx: &egui::Context, gui: &mut Gui, model: &Model) -> Option<Action> {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return None;
    }
    let mut style = model.style;
    let screen = ctx.content_rect();
    let shown = gui::window("Energy display", WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.right() - 360.0, screen.top() + 220.0))
        .show(ctx, |ui| {
            ui.set_width(310.0);
            gui::section(ui, "Colours");
            for palette in Palette::ALL {
                if ui.radio(style.palette == palette, palette.name()).clicked() {
                    style.palette = palette;
                }
            }
            ui.add_space(4.0);
            // The legend: renewable sources on the left, fossil and nuclear (hatched) on the right,
            // each with its colour running and, beside it, not running.
            ui.columns(2, |cols| {
                for source in Source::ALL {
                    let col = &mut cols[if source.fossil() { 1 } else { 0 }];
                    col.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let [r, g, b] = style.colour(source);
                        gui::swatch(ui, [r, g, b, 255], source.fossil());
                        if let Some(([ir, ig, ib], a)) = style.idle_colour(source).filter(|_| model.output) {
                            gui::swatch(ui, [ir, ig, ib, a], source.fossil());
                        }
                        ui.add_space(4.0);
                        ui.label(source.name());
                    });
                }
            });
            gui::note(ui, if model.output { "Left: running now. Right: not running. Hatched: fossil or nuclear." } else { "Hatched: fossil or nuclear." });

            gui::section(ui, "Capacity not running");
            for idle in Idle::ALL {
                if ui.radio(style.idle == idle, idle.name()).clicked() {
                    style.idle = idle;
                }
            }
            let label = format!("Opacity {:.0}%", style.idle_opacity * 100.0);
            slider(ui, &mut style, Track::Opacity, &label);
            if !model.output {
                gui::note(ui, "Shown while the bars show output at the clock's time (view options).");
            }

            gui::section(ui, "Bar size");
            let label = format!("Height \u{00d7}{:.2}", style.height);
            slider(ui, &mut style, Track::Height, &label);
            let label = format!("Width \u{00d7}{:.2}", style.width);
            slider(ui, &mut style, Track::Width, &label);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    style = Style::default();
                }
                gui::note(ui, "Saved as you change it.");
            });
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    gui.set_visible(WINDOW, open);
    (style != model.style).then_some(Action::Set(style))
}

/// A labelled slider for one of the style's values.
fn slider(ui: &mut egui::Ui, style: &mut Style, track: Track, label: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([110.0, 20.0], egui::Label::new(label));
        let width = ui.available_width().max(80.0);
        for slide in gui::track(ui, track.pos(style), width) {
            match slide {
                Slide::Jump(p) | Slide::Move(p) => track.set(style, track.value(p)),
                Slide::Step(n) => {
                    let pos = track.pos(style) + n as f64 / NOTCHES;
                    track.set(style, track.value(pos));
                }
                Slide::Grab | Slide::End => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_run_over_their_ranges() {
        assert!((Track::Height.value(0.0) - HEIGHT_RANGE.0).abs() < 1e-9 && (Track::Height.value(1.0) - HEIGHT_RANGE.1).abs() < 1e-9);
        assert!((Track::Height.value(0.5) - 1.0).abs() < 1e-9, "the usual height in the middle");
        let st = Style { width: 1.7, idle_opacity: 0.5, ..Style::default() };
        for t in [Track::Opacity, Track::Height, Track::Width] {
            let mut back = st;
            t.set(&mut back, t.value(t.pos(&st)));
            assert!((back.width - st.width).abs() < 1e-9 && (back.idle_opacity - st.idle_opacity).abs() < 1e-9);
        }
    }
}
