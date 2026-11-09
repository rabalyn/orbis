//! The information box that opens when a place on the map is clicked: a floating, scrollable
//! window with the title, the facts, the article's introduction and the credit for them.
//! The box shows a [`Model`] that the scene fills when an answer arrives.

use crate::gui::{self, Gui};
use crate::wiki;
use crate::wikilookup::{Article, NearItem};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "info-window";

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Looking,
    Message(String),
    /// Figures in a table under a one-line summary, with explanations in grey below.
    Facts(Card),
    Article(Article),
    Nearby { items: Vec<NearItem>, radius: f64 },
}

/// What the box shows about something measured: the gist first, then the figures, then how to
/// read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub summary: String,
    /// Rows of name and value; an empty row leaves a gap.
    pub facts: Vec<(String, String)>,
    /// Short paragraphs, in grey.
    pub notes: Vec<String>,
    /// Groups of bars drawn between the summary and the facts.
    pub meters: Vec<Meters>,
}

/// Bars on one common scale under a small heading: each a label, a bar of stacked segments and a
/// figure, with a line of explanation below.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meters {
    pub title: String,
    /// The value that fills a whole bar.
    pub max: f32,
    pub rows: Vec<Meter>,
    pub caption: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meter {
    pub label: String,
    pub segments: Vec<Segment>,
    pub text: String,
}

/// A part of a bar, in the units of [`Meters::max`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub value: f32,
    pub colour: [u8; 3],
    /// 255 is solid.
    pub alpha: u8,
    pub hatched: bool,
    /// Drawn in the text's colour instead of `colour`, as lines in charts are.
    pub ink: bool,
}

impl Segment {
    pub fn solid(value: f32, colour: [u8; 3]) -> Segment {
        Segment { value, colour, alpha: 255, hatched: false, ink: false }
    }
}

/// The groups of bars, as [`Card::meters`] holds them.
fn meters(ui: &mut egui::Ui, groups: &[Meters]) {
    const LABEL: f32 = 92.0;
    const ROW: f32 = 18.0;
    let font = egui::FontId::proportional(12.5);
    for g in groups {
        // The figures' column is as wide as its longest figure.
        let text = g.rows.iter().map(|m| ui.painter().layout_no_wrap(m.text.clone(), font.clone(), egui::Color32::WHITE).size().x).fold(0.0f32, f32::max) + 10.0;
        ui.add_space(6.0);
        if !g.title.is_empty() {
            ui.label(egui::RichText::new(&g.title).color(gui::accent()));
        }
        for m in &g.rows {
            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::hover());
            let v = ui.visuals();
            let (ink, weak) = (v.text_color(), v.weak_text_color());
            let p = ui.painter();
            p.text(egui::pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, &m.label, font.clone(), weak);
            p.text(egui::pos2(row.right(), row.center().y), egui::Align2::RIGHT_CENTER, &m.text, font.clone(), ink);
            let track = egui::Rect::from_min_max(egui::pos2(row.left() + LABEL, row.top() + 3.0), egui::pos2((row.right() - text).max(row.left() + LABEL + 20.0), row.bottom() - 3.0));
            p.rect_filled(track, 2.0, v.extreme_bg_color);
            let mut x = track.left();
            for s in &m.segments {
                if s.value <= 0.0 || g.max <= 0.0 {
                    continue;
                }
                // Something present never vanishes, however small next to the rest.
                let w = (s.value / g.max * track.width()).max(2.0).min(track.right() - x);
                if w <= 0.0 {
                    break;
                }
                let r = egui::Rect::from_min_max(egui::pos2(x, track.top()), egui::pos2(x + w, track.bottom()));
                let c = if s.ink { ink } else { egui::Color32::from_rgb(s.colour[0], s.colour[1], s.colour[2]) };
                p.rect_filled(r, 0.0, c.gamma_multiply(s.alpha as f32 / 255.0));
                if s.hatched {
                    gui::hatch(&p.with_clip_rect(r), r, s.colour);
                }
                x += w;
            }
        }
        if !g.caption.is_empty() {
            gui::note(ui, &g.caption);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub title: String,
    /// Where the click was, shown when there is no article to show.
    pub at: Option<(f64, f64)>,
    pub content: Content,
    /// Something to tell about the last button, such as a browser that would not start.
    pub note: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        Self { title: String::new(), at: None, content: Content::Looking, note: None }
    }
}

impl Model {
    pub fn near(&self, i: usize) -> Option<&NearItem> {
        match &self.content {
            Content::Nearby { items, .. } => items.get(i),
            _ => None,
        }
    }

    pub fn article_url(&self) -> Option<&str> {
        match &self.content {
            Content::Article(a) => Some(&a.url),
            _ => None,
        }
    }

    /// The credit that goes with what is shown, if anything from Wikipedia is.
    pub fn credit(&self) -> Option<String> {
        match &self.content {
            Content::Article(a) => {
                let mut s = format!("Text: Wikipedia, \"{}\" ({}.wikipedia.org), CC BY-SA 4.0.", a.title, a.lang);
                if !a.facts.is_empty() {
                    s.push_str(" Facts: Wikidata, CC0.");
                }
                Some(s)
            }
            Content::Nearby { items, .. } if !items.is_empty() => Some(format!("Names: Wikipedia ({}.wikipedia.org), CC BY-SA 4.0.", items[0].lang)),
            _ => None,
        }
    }
}

pub type Shared = Rc<RefCell<Model>>;

/// What a click in the box asks the scene to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenBrowser,
    /// Show the article with this number of the nearby list.
    Pick(usize),
}

/// Opens an article in the browser. Only a plain Wikipedia address is passed on.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let url = wiki::article_url(url).ok_or("This address is not one that is safe to open.")?;
    launch(&url)
}

/// Whether an address is a plain web page: http or https, no spaces or control characters,
/// and not overly long. Nothing else is handed to the browser.
pub fn is_web_address(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/') && url.len() <= 2048 && !url.chars().any(|c| c.is_whitespace() || c.is_control()))
}

/// Opens a web page from the interface, such as a source named in the credits.
pub fn open_web_page(url: &str) -> Result<(), String> {
    if !is_web_address(url) {
        return Err("This address is not one that is safe to open.".to_string());
    }
    launch(url)
}

#[cfg(target_arch = "wasm32")]
fn launch(url: &str) -> Result<(), String> {
    crate::web::open(url)
}

/// Starts the desktop's browser on an address, without a shell.
#[cfg(not(target_arch = "wasm32"))]
fn launch(url: &str) -> Result<(), String> {
    let child = std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("The browser could not be started ({e})."))?;
    // Reaps the helper when it ends, so that it does not linger.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// The box, while it is open; a click on its buttons comes back as an action. Closing it is
/// reported as closed, since the scene stops the lookup then.
pub fn show(ctx: &egui::Context, gui: &mut Gui, m: &Model) -> (bool, Option<Action>) {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return (false, None);
    }
    let mut action = None;
    let screen = ctx.content_rect();
    let title = wiki::shorten(&m.title, 60);
    let coords = m.at.map(|(lat, lon)| wiki::coords_text(lat, lon));
    // Above the credits at the bottom left, but below the day chart when that is open there.
    let height = (screen.height() * 0.55).min(480.0);
    let below_chart = gui
        .is_visible(crate::power_chart::WINDOW)
        .then(|| ctx.memory(|m| m.area_rect(egui::Id::unique(crate::power_chart::WINDOW))))
        .flatten()
        .map_or(f32::NEG_INFINITY, |r| r.bottom() + 10.0);
    let top = (screen.bottom() - 120.0 - height).max(below_chart);
    let shown = gui::window(&title, WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.left() + 12.0, top))
        .default_width((screen.width() - 40.0).clamp(240.0, 420.0))
        .default_height(height.min(screen.bottom() - 110.0 - top).max(160.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let facts = |ui: &mut egui::Ui, rows: &[(String, String)]| {
                    egui::Grid::new("facts").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
                        for (k, v) in rows {
                            if k.is_empty() && v.is_empty() {
                                ui.end_row();
                                continue;
                            }
                            ui.label(egui::RichText::new(k).color(gui::accent()));
                            ui.add(egui::Label::new(v).wrap());
                            ui.end_row();
                        }
                    });
                };
                match &m.content {
                    Content::Looking => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            gui::note(ui, "Looking up\u{2026}");
                        });
                    }
                    Content::Message(text) => {
                        ui.add(egui::Label::new(text).wrap());
                    }
                    Content::Facts(card) => {
                        ui.add(egui::Label::new(egui::RichText::new(&card.summary).strong()).wrap());
                        meters(ui, &card.meters);
                        if !card.facts.is_empty() {
                            ui.add_space(6.0);
                            facts(ui, &card.facts);
                        }
                        for note in &card.notes {
                            ui.add_space(4.0);
                            gui::note(ui, note);
                        }
                    }
                    Content::Article(a) => {
                        if !a.description.is_empty() {
                            gui::note(ui, &a.description);
                        }
                        if !a.facts.is_empty() {
                            ui.add_space(4.0);
                            facts(ui, &a.facts);
                            ui.add_space(4.0);
                        }
                        if a.text.is_empty() {
                            gui::note(ui, "This article has no introduction.");
                        } else {
                            for para in a.text.split('\n').filter(|p| !p.trim().is_empty()) {
                                ui.add(egui::Label::new(para).wrap());
                                ui.add_space(4.0);
                            }
                        }
                    }
                    Content::Nearby { items, radius } => {
                        if items.is_empty() {
                            ui.label(format!("No articles within {} of this spot.", wiki::distance_text(*radius)));
                        } else {
                            gui::note(ui, "Articles nearby, closest first. Click one to read it.");
                            for (i, it) in items.iter().enumerate() {
                                let text = format!("{}  \u{00b7}  {}", it.title, wiki::distance_text(it.distance));
                                if ui.add(egui::Button::new(text).wrap()).clicked() {
                                    action = Some(Action::Pick(i));
                                }
                            }
                        }
                    }
                }
                if let Some(c) = &coords {
                    ui.add_space(4.0);
                    gui::note(ui, c);
                }
            });
            if m.credit().is_some() || m.note.is_some() || m.article_url().is_some() {
                ui.separator();
            }
            if let Some(c) = m.credit() {
                gui::note(ui, &c);
            }
            if let Some(n) = &m.note {
                ui.label(egui::RichText::new(n).color(gui::warn_colour(ui)));
            }
            if m.article_url().is_some() && ui.button("Open article in browser").clicked() {
                action = Some(Action::OpenBrowser);
            }
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    gui.set_visible(WINDOW, open);
    (!open, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_web_addresses_are_opened() {
        assert!(is_web_address("https://openfreemap.org"));
        assert!(is_web_address("http://example.org/a?b=c"));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://", "https:///x", "https://a b", "-https://x", "https://x\n"] {
            assert!(!is_web_address(bad), "{bad}");
        }
    }

    fn article(text: &str) -> Article {
        Article {
            lang: "en".into(),
            title: "Vienna".into(),
            description: "Capital and largest city of Austria".into(),
            facts: vec![("Population".into(), "2,028,289 (2025)".into()), ("Country".into(), "Austria".into())],
            text: text.into(),
            url: "https://en.wikipedia.org/wiki/Vienna".into(),
        }
    }

    fn model(content: Content) -> Model {
        Model { title: "Vienna".into(), at: Some((48.2, 16.4)), content, note: None }
    }

    #[test]
    fn the_credit_names_the_article_while_it_is_shown() {
        let m = model(Content::Article(article("Text")));
        let c = m.credit().unwrap();
        assert!(c.contains("\"Vienna\"") && c.contains("en.wikipedia.org") && c.contains("CC BY-SA 4.0") && c.contains("Wikidata, CC0"), "{c}");
        assert_eq!(model(Content::Looking).credit(), None);
        assert_eq!(m.article_url(), Some("https://en.wikipedia.org/wiki/Vienna"));
        assert_eq!(model(Content::Looking).article_url(), None);
    }

    #[test]
    fn only_wikipedia_addresses_reach_the_browser() {
        assert!(open_in_browser("file:///etc/passwd").is_err());
        assert!(open_in_browser("https://evil.example/wiki/x").is_err());
        assert!(open_in_browser("-o /tmp/x").is_err());
    }
}
