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
    let width = (screen.width() - 40.0).clamp(240.0, 420.0);
    let title = wiki::shorten(&m.title, 60);
    let coords = m.at.map(|(lat, lon)| wiki::coords_text(lat, lon));
    let shown = gui::window(&title, WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.left() + 12.0, screen.bottom() - 120.0 - (screen.height() * 0.55).min(480.0)))
        .show(ctx, |ui| {
            ui.set_width(width);
            egui::ScrollArea::vertical().max_height((screen.height() * 0.55).min(480.0)).show(ui, |ui| {
                ui.set_width(width);
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
                        ui.add_space(4.0);
                        facts(ui, &card.facts);
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
