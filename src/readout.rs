//! What the screen tells besides the windows: the credits at the bottom, whose sources are links;
//! a quiet line at the top right with where and when the view is; the technical figures (tiles,
//! frame rate, downloads) on request; and the boxes for searching and for lists, which are typed
//! into and steered with the keyboard.

use crate::gui::{self, Gui};
use egui::{Align2, Frame, Margin, RichText, Vec2};

/// Sources named in the credits, with the page that tells about them and their terms.
const SOURCE_LINKS: &[(&str, &str)] = &[
    ("OpenFreeMap", "https://openfreemap.org"),
    ("OpenMapTiles", "https://openmaptiles.org"),
    ("OpenStreetMap", "https://www.openstreetmap.org/copyright"),
    ("Copernicus Sentinel data", "https://sentinels.copernicus.eu"),
    ("EOX IT Services GmbH", "https://eox.at"),
    ("HYG database", "https://github.com/astronexus/HYG-Database"),
    ("Marktstammdatenregister", "https://www.marktstammdatenregister.de"),
    ("Bundesnetzagentur", "https://www.bundesnetzagentur.de"),
    ("SMARD.de", "https://www.smard.de"),
    ("© BKG", "https://gdz.bkg.bund.de"),
    ("dl-de/by-2-0", "https://www.govdata.de/dl-de/by-2-0"),
    ("CC BY-SA 4.0", "https://creativecommons.org/licenses/by-sa/4.0/"),
    ("CC BY 4.0", "https://creativecommons.org/licenses/by/4.0/"),
    ("Wikipedia", "https://www.wikipedia.org"),
    ("Nominatim", "https://nominatim.org"),
    ("USGS", "https://earthquake.usgs.gov"),
];

/// One piece of a credit line: plain text, or text that links somewhere.
#[derive(Debug, PartialEq)]
pub enum Piece<'a> {
    Text(&'a str),
    Link(&'a str, &'a str),
}

/// Splits a line of credits into text and links: addresses written out link to themselves,
/// and the names of known sources to their pages.
pub fn pieces(line: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        // The earliest address or known name in what is left.
        let address = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min();
        let name = SOURCE_LINKS.iter().filter_map(|(n, url)| rest.find(n).map(|i| (i, *n, *url))).min_by_key(|m| (m.0, std::cmp::Reverse(m.1.len())));
        match (address, name) {
            (Some(a), n) if n.is_none_or(|n| a <= n.0) => {
                let end = rest[a..].find(char::is_whitespace).map_or(rest.len(), |e| a + e);
                // Punctuation after an address belongs to the sentence.
                let url = rest[a..end].trim_end_matches([')', ',', '.', ';']);
                if a > 0 {
                    out.push(Piece::Text(&rest[..a]));
                }
                out.push(Piece::Link(url, url));
                rest = &rest[a + url.len()..];
            }
            (_, Some((i, n, url))) => {
                if i > 0 {
                    out.push(Piece::Text(&rest[..i]));
                }
                out.push(Piece::Link(&rest[i..i + n.len()], url));
                rest = &rest[i + n.len()..];
            }
            _ => {
                out.push(Piece::Text(rest));
                break;
            }
        }
    }
    out
}

/// A rounded, see-through box for text over the imagery.
fn plate(ui: &egui::Ui) -> Frame {
    let v = ui.visuals();
    Frame::new().fill(v.window_fill.gamma_multiply(0.85)).corner_radius(6.0).inner_margin(Margin::symmetric(8, 4)).stroke(v.window_stroke)
}

/// The credits' area: the one overlay in the background whose links take clicks.
pub const CREDITS: &str = "credits";

/// The credits at the bottom left, leaving `right` points free for the icons.
pub fn credits(ctx: &egui::Context, gui: &mut Gui, text: &str, right: f32) {
    if text.is_empty() {
        return;
    }
    let screen = ctx.content_rect();
    let width = (screen.width() - right - 20.0).max(160.0);
    let shown = egui::Area::new(egui::Id::unique(CREDITS)).anchor(Align2::LEFT_BOTTOM, Vec2::new(8.0, -8.0)).order(egui::Order::Background).interactable(true).show(ctx, |ui| {
        plate(ui).show(ui, |ui| {
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            // Rows as tall as their text, not as a button.
            ui.spacing_mut().interact_size.y = 13.0;
            for line in text.lines() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for piece in pieces(line) {
                        match piece {
                            Piece::Text(t) => {
                                ui.label(RichText::new(t).size(11.0).weak());
                            }
                            Piece::Link(t, url) => {
                                ui.hyperlink_to(RichText::new(t).size(11.0), url).on_hover_text(url);
                            }
                        }
                    }
                });
            }
        });
    });
    gui.cover(shown.response.rect);
}

/// The line at the top right: where the view is, how high, and the clock.
pub fn status(ctx: &egui::Context, gui: &mut Gui, line: &str, busy: Option<&str>) {
    let shown = egui::Area::new(egui::Id::unique("status")).anchor(Align2::RIGHT_TOP, Vec2::new(-10.0, 10.0)).order(egui::Order::Background).show(ctx, |ui| {
        plate(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(b) = busy {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(RichText::new(b).size(12.0).weak());
                    ui.separator();
                }
                ui.label(RichText::new(line).size(12.5));
            });
        });
    });
    gui.cover(shown.response.rect);
}

/// The technical figures, below the status line, while they are asked for (F12).
pub fn technical(ctx: &egui::Context, gui: &mut Gui, lines: &[String]) {
    let shown = egui::Area::new(egui::Id::unique("technical")).anchor(Align2::RIGHT_TOP, Vec2::new(-10.0, 46.0)).order(egui::Order::Background).show(ctx, |ui| {
        plate(ui).show(ui, |ui| {
            ui.label(RichText::new("Technical (F12 hides)").size(11.0).weak());
            for l in lines {
                ui.label(RichText::new(l).monospace().size(11.5));
            }
        });
    });
    gui.cover(shown.response.rect);
}

/// A box of preformatted text at the top left, for the search box and the lists: `y` points down.
pub fn text_box(ctx: &egui::Context, gui: &mut Gui, id: &str, text: &str, y: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let shown = egui::Area::new(egui::Id::unique(id)).fixed_pos(egui::pos2(10.0, y)).order(egui::Order::Foreground).show(ctx, |ui| {
        Frame::window(ui.style()).show(ui, |ui| {
            for (i, line) in text.lines().enumerate() {
                let t = RichText::new(line).monospace();
                ui.label(if i == 0 { t.color(gui::accent()) } else { t });
            }
        });
    });
    gui.cover(shown.response.rect);
    shown.response.rect.height()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::Button;

    #[test]
    fn addresses_and_known_sources_become_links() {
        let p = pieces("Terrain: © Mapterhorn https://mapterhorn.com/attribution");
        assert_eq!(p, vec![Piece::Text("Terrain: © Mapterhorn "), Piece::Link("https://mapterhorn.com/attribution", "https://mapterhorn.com/attribution")]);
        let p = pieces("Map: OpenFreeMap © OpenMapTiles Data from OpenStreetMap");
        assert_eq!(
            p,
            vec![
                Piece::Text("Map: "),
                Piece::Link("OpenFreeMap", "https://openfreemap.org"),
                Piece::Text(" © "),
                Piece::Link("OpenMapTiles", "https://openmaptiles.org"),
                Piece::Text(" Data from "),
                Piece::Link("OpenStreetMap", "https://www.openstreetmap.org/copyright"),
            ]
        );
        // Punctuation after an address stays text.
        let p = pieces("see (https://example.org).");
        assert_eq!(p, vec![Piece::Text("see ("), Piece::Link("https://example.org", "https://example.org"), Piece::Text(").")]);
        assert_eq!(pieces("plain"), vec![Piece::Text("plain")]);
        assert!(pieces("").is_empty());
    }

    /// Builds one frame of the interface with the credits and the status line, as the scene does.
    fn frame(gui: &mut Gui) {
        let raw = gui.begin((800, 600), 1.0);
        let ctx = gui.ctx.clone();
        let out = ctx.run_pass(raw, |ctx| {
            credits(ctx, gui, "OpenFreeMap and more text after the link", 0.0);
            status(ctx, gui, "30.0000° N 10.0000° E", None);
        });
        gui.finish(out);
    }

    fn area(gui: &Gui, id: &str) -> egui::Rect {
        gui.ctx.memory(|m| m.area_rect(egui::Id::unique(id))).expect("the area was drawn")
    }

    #[test]
    fn a_click_on_a_credit_link_opens_its_page() {
        let mut gui = Gui::new();
        // Areas find their size over the first frames.
        for _ in 0..3 {
            frame(&mut gui);
        }
        let r = area(&gui, CREDITS);
        // The line starts with the link, just inside the plate's margin.
        let on_link = glam::DVec2::new(r.min.x as f64 + 14.0, r.center().y as f64);
        assert!(gui.pointer_moved(on_link), "the credits take the pointer");
        frame(&mut gui);
        assert!(gui.press(Button::Left), "the credits take the press, not the map");
        frame(&mut gui);
        assert!(gui.release(Button::Left));
        frame(&mut gui);
        assert_eq!(gui.links, vec!["https://openfreemap.org".to_string()]);
    }

    #[test]
    fn other_overlays_in_the_background_leave_clicks_to_the_map() {
        let mut gui = Gui::new();
        for _ in 0..3 {
            frame(&mut gui);
        }
        let r = area(&gui, "status");
        let on_status = glam::DVec2::new(r.center().x as f64, r.center().y as f64);
        assert!(!gui.pointer_moved(on_status));
        assert!(!gui.press(Button::Left), "a press on the status line goes to the map");
        let on_map = glam::DVec2::new(400.0, 300.0);
        assert!(!gui.pointer_moved(on_map));
        assert!(!gui.press(Button::Left));
        assert!(gui.links.is_empty());
    }

    #[test]
    fn every_credit_line_keeps_its_text() {
        for line in ["Plants: Marktstammdatenregister, Bundesnetzagentur, dl-de/by-2-0 (summed)", "Stars: HYG database by David Nash, CC BY-SA 4.0"] {
            let joined: String = pieces(line).iter().map(|p| match p {
                Piece::Text(t) | Piece::Link(t, _) => *t,
            }).collect();
            assert_eq!(joined, line);
        }
    }
}
