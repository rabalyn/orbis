//! The row of icons at the bottom right: help, view options, the time window, and the compass,
//! whose red end points north on the screen and which turns the view back to north when clicked.
//! The flat map is always north up, so there the compass is hidden.

use crate::gui::{self, Gui};
use egui::{Color32, Pos2, Sense, Stroke, Vec2};

/// Side of an icon, in points, and the space around the row.
pub const ICON: f32 = 34.0;
const GAP: f32 = 6.0;
pub const MARGIN: f32 = 10.0;
const ID: &str = "icons";
const NEEDLE_NORTH: Color32 = Color32::from_rgb(230, 60, 50);

/// What a click on an icon asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clicked {
    Help,
    Options,
    Time,
    /// Turn the view to face north.
    North,
}

/// How wide the row is with or without the compass, in points, margins included.
pub fn row_width(compass: bool) -> f32 {
    let n = if compass { 4.0 } else { 3.0 };
    n * ICON + (n - 1.0) * GAP + 2.0 * MARGIN
}

enum Glyph {
    Compass(f64),
    Clock,
    Lines,
    Question,
}

/// The icons, with the compass for a view `heading` (radians clockwise from north) or without it
/// for `None`. `open` tells which of help, options and time have their window open.
pub fn show(ctx: &egui::Context, gui: &mut Gui, heading: Option<f64>, open: [bool; 3]) -> Option<Clicked> {
    let mut clicked = None;
    let shown = egui::Area::new(egui::Id::unique(ID))
        .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-MARGIN, -MARGIN))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                if let Some(h) = heading {
                    if icon(ui, Glyph::Compass(h), false, "Turn to face north").clicked() {
                        clicked = Some(Clicked::North);
                    }
                }
                if icon(ui, Glyph::Clock, open[2], "Time window (C)").clicked() {
                    clicked = Some(Clicked::Time);
                }
                if icon(ui, Glyph::Lines, open[1], "View options (V)").clicked() {
                    clicked = Some(Clicked::Options);
                }
                if icon(ui, Glyph::Question, open[0], "Keyboard and mouse (H)").clicked() {
                    clicked = Some(Clicked::Help);
                }
            });
        });
    gui.cover(shown.response.rect);
    clicked
}

/// One round icon button, lit while its window is open.
fn icon(ui: &mut egui::Ui, glyph: Glyph, lit: bool, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(ICON), Sense::click());
    let v = ui.visuals();
    let fill = v.window_fill;
    let stroke = if response.hovered() || lit { Stroke::new(1.5, gui::accent()) } else { v.window_stroke };
    let ink = v.text_color();
    let p = ui.painter();
    p.circle(rect.center(), ICON / 2.0, fill, stroke);
    let c = rect.center();
    let r = ICON / 2.0;
    match glyph {
        Glyph::Question => {
            p.text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::new(18.0, egui::FontFamily::Name(gui::BOLD.into())), ink);
        }
        Glyph::Lines => {
            for dy in [-5.0, 0.0, 5.0] {
                p.line_segment([Pos2::new(c.x - 7.0, c.y + dy), Pos2::new(c.x + 7.0, c.y + dy)], Stroke::new(1.8, ink));
            }
        }
        Glyph::Clock => {
            p.circle_stroke(c, r * 0.55, Stroke::new(1.5, ink));
            p.line_segment([c, Pos2::new(c.x, c.y - r * 0.4)], Stroke::new(1.5, ink));
            p.line_segment([c, Pos2::new(c.x + r * 0.28, c.y + r * 0.12)], Stroke::new(1.5, ink));
        }
        Glyph::Compass(heading) => {
            // North on the screen, and the needle's side; y grows downwards.
            let (s, co) = (heading as f32).sin_cos();
            let north = Vec2::new(-s, -co);
            let side = Vec2::new(co, -s);
            let at = |along: f32, across: f32| c + north * along + side * across;
            let (tip, half) = (r * 0.62, r * 0.2);
            p.add(egui::Shape::convex_polygon(vec![at(tip, 0.0), at(0.0, half), at(0.0, -half)], NEEDLE_NORTH, Stroke::NONE));
            p.add(egui::Shape::convex_polygon(vec![at(-tip, 0.0), at(0.0, -half), at(0.0, half)], ink, Stroke::NONE));
        }
    }
    response.on_hover_text(tip)
}
