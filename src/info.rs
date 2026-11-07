//! The information box that opens when a place on the map is clicked: a floating, scrollable
//! window with the title, the facts, the article's introduction and the credit for them.
//! The box shows a [`Model`] that the scene fills when an answer arrives.

use crate::ui::{Body, Metrics, Panel, Rect, Tone, Ui, UiEvent, Widget, WidgetId};
use crate::wiki;
use crate::wikilookup::{Article, NearItem};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "info-window";
const CLOSE_BUTTON: WidgetId = 1;
const OPEN_BUTTON: WidgetId = 2;
const NEAR_BASE: WidgetId = 100;
/// Text columns of the box at most.
const COLS: usize = 54;
/// Width of the column of fact names, with the space after it.
const KEY_COLS: usize = 11;

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Looking,
    Message(String),
    Article(Article),
    Nearby { items: Vec<NearItem>, radius: f64 },
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

pub fn window_panel(model: Shared) -> Panel {
    Panel::new(WINDOW, true, move |p, m| layout(p, m, &model.borrow()))
}

/// Closes the box for its close button; anything else clicked in it comes back as an action.
pub fn handle(ui: &mut Ui, event: UiEvent) -> (bool, Option<Action>) {
    let UiEvent::Clicked { panel, widget } = event else { return (false, None) };
    match (panel, widget) {
        (WINDOW, CLOSE_BUTTON) => ui.set_visible(WINDOW, false),
        (WINDOW, OPEN_BUTTON) => return (true, Some(Action::OpenBrowser)),
        (WINDOW, id) if id >= NEAR_BASE => return (true, Some(Action::Pick((id - NEAR_BASE) as usize))),
        _ => return (false, None),
    }
    (true, None)
}

/// Opens an article in the browser, without a shell. Only a plain Wikipedia address is passed on.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let url = wiki::article_url(url).ok_or("This address is not one that is safe to open.")?;
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

/// One line of the body: a fact's name if it starts one, the text, its tone, and the column it starts in.
type Line = (Option<String>, String, Tone, usize);

fn body_lines(m: &Model, cols: usize) -> Vec<Line> {
    fn wrapped(text: &str, tone: Tone, col: usize, cols: usize, lines: &mut Vec<Line>) {
        for l in wiki::wrap_lines(text, cols.saturating_sub(col).max(4)) {
            lines.push((None, l, tone, col));
        }
    }
    let blank = || (None, String::new(), Tone::Normal, 0);
    let mut lines: Vec<Line> = Vec::new();
    let coords = m.at.map(|(lat, lon)| wiki::coords_text(lat, lon));
    match &m.content {
        Content::Looking => {
            wrapped("Looking up...", Tone::Dim, 0, cols, &mut lines);
            if let Some(c) = &coords {
                wrapped(c, Tone::Dim, 0, cols, &mut lines);
            }
        }
        Content::Message(text) => {
            wrapped(text, Tone::Normal, 0, cols, &mut lines);
            if let Some(c) = &coords {
                lines.push(blank());
                wrapped(c, Tone::Dim, 0, cols, &mut lines);
            }
        }
        Content::Article(a) => {
            if !a.description.is_empty() {
                wrapped(&a.description, Tone::Dim, 0, cols, &mut lines);
                lines.push(blank());
            }
            for (k, v) in &a.facts {
                let before = lines.len();
                wrapped(v, Tone::Normal, KEY_COLS, cols, &mut lines);
                if let Some(first) = lines.get_mut(before) {
                    first.0 = Some(wiki::shorten(k, KEY_COLS - 1));
                }
            }
            if !a.facts.is_empty() {
                lines.push(blank());
            }
            if a.text.is_empty() {
                wrapped("This article has no introduction.", Tone::Dim, 0, cols, &mut lines);
            } else {
                // A blank line between paragraphs.
                for (i, para) in a.text.split('\n').filter(|p| !p.trim().is_empty()).enumerate() {
                    if i > 0 {
                        lines.push(blank());
                    }
                    wrapped(para, Tone::Normal, 0, cols, &mut lines);
                }
            }
        }
        Content::Nearby { items, radius } => {
            if let Some(c) = &coords {
                wrapped(c, Tone::Dim, 0, cols, &mut lines);
                lines.push(blank());
            }
            if items.is_empty() {
                wrapped(&format!("No articles within {} of this spot.", wiki::distance_text(*radius)), Tone::Normal, 0, cols, &mut lines);
            } else {
                wrapped("Articles nearby, closest first. Click one to read it.", Tone::Dim, 0, cols, &mut lines);
            }
        }
    }
    lines
}

fn layout(p: &mut Panel, m: &Metrics, model: &Model) {
    let s = m.scale;
    let (glyph, step, pad) = (8.0 * s, 10.0 * s, 6.0 * s);
    let margin = 8.0 * s;
    let bar_w = 6.0 * s;
    let header = 18.0 * s;
    let w = (m.screen.x - 2.0 * margin).min(COLS as f64 * glyph + 2.0 * pad + bar_w).max(12.0 * glyph);
    let cols = (((w - 2.0 * pad - bar_w) / glyph) as usize).max(8);

    // Body: the lines of text, then the nearby list's buttons.
    let lines = body_lines(model, cols);
    let mut widgets = Vec::new();
    let mut y = pad;
    for (key, text, tone, col) in &lines {
        if let Some(k) = key {
            widgets.push(Widget::label(Rect::new(pad, y, k.chars().count() as f64 * glyph, glyph), k, Tone::Key));
        }
        if !text.is_empty() {
            widgets.push(Widget::label(Rect::new(pad + *col as f64 * glyph, y, text.chars().count() as f64 * glyph, glyph), text, *tone));
        }
        y += step;
    }
    if let Content::Nearby { items, .. } = &model.content {
        y += 0.4 * step;
        for (i, it) in items.iter().enumerate() {
            let label = wiki::shorten(&format!("{}  {}", it.title, wiki::distance_text(it.distance)), cols.saturating_sub(2));
            let row_w = ((label.chars().count() + 2) as f64 * glyph).min(w - 2.0 * pad - bar_w);
            widgets.push(Widget::button(NEAR_BASE + i as u32, Rect::new(pad, y, row_w, 12.0 * s), &label, 1));
            y += 15.0 * s;
        }
    }
    let content_h = y + pad;

    // Footer: the credit, a note, and the button that opens the article.
    let mut footer: Vec<(String, Tone)> = Vec::new();
    if let Some(c) = model.credit() {
        footer.extend(wiki::wrap_lines(&c, cols).into_iter().map(|l| (l, Tone::Dim)));
    }
    if let Some(n) = &model.note {
        footer.extend(wiki::wrap_lines(n, cols).into_iter().map(|l| (l, Tone::Key)));
    }
    let has_open = model.article_url().is_some();
    let footer_h = if footer.is_empty() && !has_open { 0.0 } else { pad + footer.len() as f64 * step + if has_open { 16.0 * s } else { 0.0 } + pad * 0.5 };

    // Above the credits and the icons, at the bottom left; shorter when the screen is.
    let bottom = m.floor() - 4.0 * s;
    let max_h = (bottom - margin).min(m.screen.y * 0.7).max(header + footer_h + 4.0 * step);
    let h = (header + content_h + footer_h).min(max_h);
    p.rect = Rect::new(margin, bottom - h, w, h);
    let viewport_h = h - header - footer_h;

    let title = wiki::shorten(&model.title, cols.saturating_sub(3));
    p.fixed = vec![
        Widget::label(Rect::new(pad, (header - glyph) / 2.0, title.chars().count() as f64 * glyph, glyph), &title, Tone::Heading),
        Widget::button(CLOSE_BUTTON, Rect::new(w - pad - 12.0 * s, 3.0 * s, 12.0 * s, 12.0 * s), "x", 1),
    ];
    let mut fy = header + viewport_h + pad;
    for (text, tone) in &footer {
        p.fixed.push(Widget::label(Rect::new(pad, fy, text.chars().count() as f64 * glyph, glyph), text, *tone));
        fy += step;
    }
    if has_open {
        p.fixed.push(Widget::button(OPEN_BUTTON, Rect::new(pad, fy + 2.0 * s, 28.0 * glyph, 12.0 * s), "Open article in browser", 1));
    }
    let offset = p.body.as_ref().map_or(0.0, |b| b.offset);
    p.body = Some(Body { viewport: Rect::new(0.0, header, w, viewport_h), content_h, offset, step: 3.0 * step, bar_w, widgets });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::Button;
    use glam::DVec2;

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

    fn ui_with(model: Model, w: f64, h: f64) -> (Ui, Shared) {
        let shared = Rc::new(RefCell::new(model));
        let mut ui = Ui::new();
        ui.resize(DVec2::new(w, h), 1.0);
        ui.add(window_panel(shared.clone()));
        ui.set_visible(WINDOW, true);
        ui.refresh(WINDOW);
        (ui, shared)
    }

    fn model(content: Content) -> Model {
        Model { title: "Vienna".into(), at: Some((48.2, 16.4)), content, note: None }
    }

    #[test]
    fn the_box_fits_and_clears_the_credits_at_every_size() {
        let long = "Word ".repeat(400) + &"x".repeat(300);
        for content in [Content::Looking, Content::Message("No article found for X.".into()), Content::Article(article(&long))] {
            for (w, h) in [(1280.0, 800.0), (500.0, 350.0), (240.0, 200.0)] {
                let (mut ui, _) = ui_with(model(content.clone()), w, h);
                let r = ui.panels()[0].rect;
                assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= w && r.y + r.h <= h, "{w}x{h}: {r:?}");
                ui.set_credit_band(60.0);
                ui.refresh(WINDOW);
                assert!(ui.covering_credits().is_empty(), "{w}x{h}");
                let body = ui.panels()[0].body.as_ref().unwrap();
                for wd in &body.widgets {
                    assert!(wd.rect.x + wd.rect.w <= r.w + 0.5, "{w}x{h}: a line is wider than the box");
                }
            }
        }
    }

    #[test]
    fn a_long_article_scrolls_and_a_short_one_does_not() {
        let (ui, _) = ui_with(model(Content::Article(article(&"Word ".repeat(400)))), 800.0, 500.0);
        assert!(ui.panels()[0].body.as_ref().unwrap().scrolls());
        let (ui, _) = ui_with(model(Content::Message("No article found for X.".into())), 800.0, 500.0);
        assert!(!ui.panels()[0].body.as_ref().unwrap().scrolls());
    }

    #[test]
    fn the_credit_names_the_article_while_it_is_shown() {
        let m = model(Content::Article(article("Text")));
        let c = m.credit().unwrap();
        assert!(c.contains("\"Vienna\"") && c.contains("en.wikipedia.org") && c.contains("CC BY-SA 4.0") && c.contains("Wikidata, CC0"), "{c}");
        assert_eq!(model(Content::Looking).credit(), None);
        // The footer lines are on the panel itself, so they are in saved images.
        let (ui, _) = ui_with(m, 1280.0, 800.0);
        let texts: Vec<String> = ui.panels()[0].fixed.iter().filter_map(|w| match &w.kind {
            crate::ui::Kind::Label { text, .. } => Some(text.clone()),
            _ => None,
        }).collect();
        assert!(texts.iter().any(|t| t.contains("CC BY-SA 4.0") || t.contains("Wikipedia")), "{texts:?}");
    }

    fn click(ui: &mut Ui, p: DVec2) -> Option<UiEvent> {
        ui.press(Button::Left, p);
        ui.release(Button::Left, p).event
    }

    #[test]
    fn buttons_close_open_and_pick() {
        let items = vec![
            NearItem { lang: "en".into(), page_id: 1, title: "Alpha".into(), item: None, distance: 120.0 },
            NearItem { lang: "en".into(), page_id: 2, title: "Beta".into(), item: Some("Q2".into()), distance: 2500.0 },
        ];
        let (mut ui, shared) = ui_with(model(Content::Nearby { items, radius: 5000.0 }), 1280.0, 800.0);
        let r = ui.panels()[0].rect;
        let body = ui.panels()[0].body.clone().unwrap();
        let second = body.widgets.iter().find(|w| w.id == NEAR_BASE + 1).unwrap().rect;
        let at = DVec2::new(r.x + second.x + 5.0, r.y + body.viewport.y + second.y + 3.0);
        let e = click(&mut ui, at).expect("the list row is clickable");
        assert_eq!(handle(&mut ui, e), (true, Some(Action::Pick(1))));
        assert_eq!(shared.borrow().near(1).unwrap().title, "Beta");
        let close = DVec2::new(r.x + r.w - 6.0 - 6.0, r.y + 9.0);
        let e = click(&mut ui, close).unwrap();
        assert_eq!(handle(&mut ui, e), (true, None));
        assert!(!ui.is_visible(WINDOW));
    }

    #[test]
    fn the_open_button_exists_only_for_an_article() {
        let (mut ui, shared) = ui_with(model(Content::Article(article("Text"))), 1280.0, 800.0);
        let find = |ui: &Ui| ui.panels()[0].fixed.iter().find(|w| w.id == OPEN_BUTTON).map(|w| w.rect);
        let r = ui.panels()[0].rect;
        let b = find(&ui).expect("open button");
        let e = click(&mut ui, DVec2::new(r.x + b.x + 4.0, r.y + b.y + 4.0)).unwrap();
        assert_eq!(handle(&mut ui, e), (true, Some(Action::OpenBrowser)));
        shared.borrow_mut().content = Content::Looking;
        ui.refresh(WINDOW);
        assert!(find(&ui).is_none());
    }

    #[test]
    fn only_wikipedia_addresses_reach_the_browser() {
        assert!(open_in_browser("file:///etc/passwd").is_err());
        assert!(open_in_browser("https://evil.example/wiki/x").is_err());
        assert!(open_in_browser("-o /tmp/x").is_err());
    }

    #[test]
    fn odd_text_does_not_break_the_layout() {
        let weird = format!("{} {} {}", "\u{6771}\u{4eac}".repeat(100), "\u{627}\u{644}\u{642}\u{627}\u{647}\u{631}\u{629}".repeat(40), "W".repeat(500));
        let mut a = article(&wiki::display_text(&weird));
        a.title = "\u{6771}\u{4eac}".repeat(60);
        a.facts.push(("Website".into(), format!("https://example.org/{}", "a".repeat(300))));
        let (ui, _) = ui_with(model(Content::Article(a)), 500.0, 350.0);
        let r = ui.panels()[0].rect;
        for w in &ui.panels()[0].fixed {
            assert!(w.rect.x + w.rect.w <= r.w + 0.5);
        }
    }
}
