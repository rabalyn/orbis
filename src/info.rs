//! The information box that opens when a place on the map is clicked: a floating, scrollable
//! window with the title, the facts, the article's introduction and the credit for them.
//! The box shows a [`Model`] that the scene fills when an answer arrives.

use crate::country_facts::{self, Facts};
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
    /// Named features on another body around the spot, to choose one to read about.
    Features(Vec<crate::planet_names::Near>),
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
    /// A small line over the years, drawn after the bars.
    pub trend: Option<Trend>,
}

/// A small line chart: a heading, points of (year, value), the values at the top and bottom of the chart,
/// a mark at one year and a line of explanation below.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trend {
    pub title: String,
    pub points: Vec<(f32, f32)>,
    pub max: f32,
    /// The value at the bottom of the chart: 0, or below for figures that go negative.
    pub min: f32,
    pub mark: Option<f32>,
    pub caption: String,
}

/// The line of a [`Trend`], with its first and last year below and its top value at the left.
fn trend(ui: &mut egui::Ui, t: &Trend) {
    let (Some(first), Some(last)) = (t.points.first(), t.points.last()) else { return };
    ui.add_space(6.0);
    ui.label(egui::RichText::new(&t.title).color(gui::accent()));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::hover());
    let v = ui.visuals();
    let (ink, weak) = (v.text_color(), v.weak_text_color());
    let p = ui.painter();
    let font = egui::FontId::proportional(11.0);
    let plot = egui::Rect::from_min_max(egui::pos2(rect.left() + 30.0, rect.top() + 4.0), egui::pos2(rect.right() - 4.0, rect.bottom() - 14.0));
    p.rect_filled(plot, 2.0, v.extreme_bg_color);
    let span = (last.0 - first.0).max(1.0);
    let range = (t.max - t.min).max(1e-6);
    let at = |(x, y): (f32, f32)| egui::pos2(plot.left() + (x - first.0) / span * plot.width(), plot.bottom() - ((y - t.min) / range).clamp(0.0, 1.0) * plot.height());
    if t.min < 0.0 {
        let y = at((first.0, 0.0)).y;
        p.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], egui::Stroke::new(1.0, weak));
    }
    if let Some(m) = t.mark.filter(|m| (first.0..=last.0).contains(m)) {
        let x = at((m, 0.0)).x;
        p.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], egui::Stroke::new(1.0, gui::accent()));
    }
    let line: Vec<egui::Pos2> = t.points.iter().map(|&q| at(q)).collect();
    p.add(egui::Shape::line(line, egui::Stroke::new(1.5, ink)));
    p.text(egui::pos2(plot.left() - 4.0, plot.top()), egui::Align2::RIGHT_TOP, axis_text(t.max), font.clone(), weak);
    p.text(egui::pos2(plot.left() - 4.0, plot.bottom()), egui::Align2::RIGHT_BOTTOM, axis_text(t.min), font.clone(), weak);
    p.text(egui::pos2(plot.left(), rect.bottom()), egui::Align2::LEFT_BOTTOM, format!("{:.0}", first.0), font.clone(), weak);
    p.text(egui::pos2(plot.right(), rect.bottom()), egui::Align2::RIGHT_BOTTOM, format!("{:.0}", last.0), font, weak);
    if !t.caption.is_empty() {
        gui::note(ui, &t.caption);
    }
}

/// A value at the side of a [`Trend`]: whole numbers, and one decimal below 10.
fn axis_text(v: f32) -> String {
    if v.fract() == 0.0 || v.abs() >= 10.0 { format!("{v:.0}") } else { format!("{v:.1}") }
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
    /// The figures of the Data tab, when the box is about a country.
    pub data: Option<Facts>,
}

impl Default for Model {
    fn default() -> Self {
        Self { title: String::new(), at: None, content: Content::Looking, note: None, data: None }
    }
}

impl Model {
    pub fn near(&self, i: usize) -> Option<&NearItem> {
        match &self.content {
            Content::Nearby { items, .. } => items.get(i),
            _ => None,
        }
    }

    pub fn feature(&self, i: usize) -> Option<&crate::planet_names::Near> {
        match &self.content {
            Content::Features(items) => items.get(i),
            _ => None,
        }
    }

    /// The article's edition in the language with this number of its list.
    pub fn edition(&self, i: usize) -> Option<&wiki::Language> {
        match &self.content {
            Content::Article(a) => a.languages.get(i),
            _ => None,
        }
    }

    pub fn article_url(&self) -> Option<&str> {
        match &self.content {
            Content::Article(a) => Some(&a.url),
            _ => None,
        }
    }

}

/// A piece of the line that names the sources: text, or text that links to a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    Text(String),
    Link(String, String),
}

const CC_BY_SA: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
const CC0: &str = "https://creativecommons.org/publicdomain/zero/1.0/";
const GAZETTEER: &str = "https://planetarynames.wr.usgs.gov/";

impl Model {
    /// The sources of what is shown, with links to the article, the facts' item and the terms.
    pub fn sources(&self) -> Vec<Piece> {
        let t = |s: &str| Piece::Text(s.to_string());
        let l = |s: &str, u: &str| Piece::Link(s.to_string(), u.to_string());
        match &self.content {
            Content::Article(a) => {
                let mut v = vec![t("Text: Wikipedia, "), l(&format!("\"{}\" ({}.wikipedia.org)", a.title, a.lang), &a.url), t(", "), l("CC BY-SA 4.0", CC_BY_SA), t(".")];
                if !a.facts.is_empty() {
                    match &a.item {
                        Some(q) => v.extend([t(" Facts: "), l(&format!("Wikidata {q}"), &format!("https://www.wikidata.org/wiki/{q}"))]),
                        None => v.push(t(" Facts: Wikidata")),
                    }
                    v.extend([t(", "), l("CC0", CC0), t(".")]);
                }
                v
            }
            Content::Nearby { items, .. } if !items.is_empty() => {
                let lang = &items[0].lang;
                vec![t("Names: "), l(&format!("Wikipedia ({lang}.wikipedia.org)"), &format!("https://{lang}.wikipedia.org/")), t(", "), l("CC BY-SA 4.0", CC_BY_SA), t(".")]
            }
            Content::Features(items) if !items.is_empty() => vec![t("Names: IAU / "), l("USGS Gazetteer of Planetary Nomenclature", GAZETTEER), t(".")],
            _ => Vec::new(),
        }
     }

    /// The sources of the Data tab: Our World in Data and its licence.
    pub fn data_sources(&self) -> Vec<Piece> {
        if self.data.is_none() {
            return Vec::new();
        }
        vec![
            Piece::Text("Figures: ".into()),
            Piece::Link("Our World in Data".into(), country_facts::OWID_URL.into()),
            Piece::Text(", from Ember, the Energy Institute and the UN, ".into()),
            Piece::Link("CC BY 4.0".into(), country_facts::CC_BY.into()),
            Piece::Text(".".into()),
        ]
    }
}

/// The tab the box shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Main,
    Infobox,
    Data,
}

/// The Data tab: a country's figures, each a small line over the years.
fn data_tab(ui: &mut egui::Ui, f: &Facts) {
    ui.add(egui::Label::new(egui::RichText::new(format!("{} in {}", f.name, f.asked)).strong()).wrap());
    if let Some(n) = &f.note {
        gui::note(ui, n);
    }
    for r in &f.rows {
        trend(ui, &r.trend);
    }
}

/// An article's infobox: headings in the accent colour over rows of name and value. The names
/// take a column as wide as the longest, but at most two fifths of the box, and wrap in it.
fn infobox(ui: &mut egui::Ui, rows: &[wiki::BoxRow]) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let widest = rows
        .iter()
        .filter_map(|r| if let wiki::BoxRow::Fact(k, _) = r { Some(k) } else { None })
        .map(|k| ui.painter().layout_no_wrap(k.clone(), font.clone(), egui::Color32::WHITE).size().x)
        .fold(0.0f32, f32::max);
    let gap = 12.0;
    let name_w = widest.min(ui.available_width() * 0.4).ceil();
    for r in rows {
        match r {
            wiki::BoxRow::Fact(k, v) => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.allocate_ui_with_layout(egui::vec2(name_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(name_w);
                        ui.add(egui::Label::new(egui::RichText::new(k).weak()).wrap());
                    });
                    ui.add(egui::Label::new(v).wrap());
                });
                ui.add_space(2.0);
            }
            wiki::BoxRow::Heading(h) => {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(h).color(gui::accent()).strong());
            }
            wiki::BoxRow::Text(t) => {
                ui.add(egui::Label::new(t).wrap());
            }
        }
    }
}

/// The sources as a line of text and links.
fn sources(ui: &mut egui::Ui, pieces: &[Piece]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for p in pieces {
            match p {
                Piece::Text(s) => {
                    ui.label(egui::RichText::new(s).weak().size(12.0));
                }
                Piece::Link(s, url) => {
                    ui.hyperlink_to(egui::RichText::new(s).size(12.0), url).on_hover_text(url);
                }
            }
        }
    });
}

pub type Shared = Rc<RefCell<Model>>;

/// What a click in the box asks the scene to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenBrowser,
    /// Show the article with this number of the nearby list.
    Pick(usize),
    /// Show the article in the language with this number of its list.
    Language(usize),
}

/// What a language edition is called in itself, for the one being read; its list of other
/// editions names the rest.
fn edition_name(code: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("en", "English"), ("de", "Deutsch"), ("fr", "Français"), ("es", "Español"), ("it", "Italiano"),
        ("nl", "Nederlands"), ("pl", "Polski"), ("pt", "Português"), ("ru", "Русский"), ("sv", "Svenska"),
        ("uk", "Українська"), ("cs", "Čeština"), ("da", "Dansk"), ("fi", "Suomi"), ("no", "Norsk bokmål"),
        ("hu", "Magyar"), ("ro", "Română"), ("tr", "Türkçe"), ("el", "Ελληνικά"), ("ca", "Català"),
    ];
    NAMES.iter().find(|(c, _)| *c == code).map_or_else(|| code.to_string(), |(_, n)| n.to_string())
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
    let box_height = height.min(screen.bottom() - 110.0 - top).max(160.0);
    // The text scrolls; the sources and the button below it stay in view.
    let text_height = (box_height - 80.0).max(80.0);
    // The tab is kept from box to box, where the new one has it.
    let tab_id = egui::Id::unique("info-tab");
    let mut tab = ctx.data(|d| d.get_temp::<Tab>(tab_id)).unwrap_or(Tab::Main);
    let has_infobox = matches!(&m.content, Content::Article(a) if !a.infobox.is_empty());
    if (tab == Tab::Infobox && !has_infobox) || (tab == Tab::Data && m.data.is_none()) {
        tab = Tab::Main;
    }
    let shown = gui::window(&title, WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.left() + 12.0, top))
        .default_width((screen.width() - 40.0).clamp(240.0, 420.0))
        .default_height(box_height)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(text_height).show(ui, |ui| {
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
                // Boxes other than an article's have tabs only for a country's figures.
                if m.data.is_some() && !matches!(m.content, Content::Article(_)) {
                    ui.horizontal(|ui| {
                        let main = if matches!(m.content, Content::Nearby { .. }) { "Nearby" } else { "Article" };
                        ui.selectable_value(&mut tab, Tab::Main, main);
                        ui.selectable_value(&mut tab, Tab::Data, "Data");
                    });
                    ui.add_space(2.0);
                }
                match &m.content {
                    _ if tab == Tab::Data && !matches!(m.content, Content::Article(_)) => {
                        if let Some(f) = &m.data {
                            data_tab(ui, f);
                        }
                    }
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
                        if let Some(t) = &card.trend {
                            trend(ui, t);
                        }
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
                        ui.horizontal(|ui| {
                            if !a.infobox.is_empty() || m.data.is_some() {
                                ui.selectable_value(&mut tab, Tab::Main, "Article");
                                if !a.infobox.is_empty() {
                                    ui.selectable_value(&mut tab, Tab::Infobox, "Infobox");
                                }
                                if m.data.is_some() {
                                    ui.selectable_value(&mut tab, Tab::Data, "Data");
                                }
                                ui.add_space(8.0);
                            }
                            if !a.languages.is_empty() {
                                egui::ComboBox::from_id_salt("info-language").selected_text(edition_name(&a.lang)).height(320.0).show_ui(ui, |ui| {
                                    for (i, l) in a.languages.iter().enumerate() {
                                        if ui.selectable_label(false, &l.name).on_hover_text(&l.title).clicked() {
                                            action = Some(Action::Language(i));
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(format!("Also in {} other languages", a.languages.len()));
                            }
                        });
                        ui.add_space(2.0);
                        if let (Tab::Data, Some(f)) = (tab, &m.data) {
                            data_tab(ui, f);
                        } else if tab == Tab::Infobox {
                            infobox(ui, &a.infobox);
                        } else {
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
                    }
                    Content::Features(items) => {
                        gui::note(ui, "Named features at this spot, then the nearest. Click one to read about it.");
                        for (i, it) in items.iter().enumerate() {
                            let size = wiki::distance_text(it.diameter);
                            let text = if it.inside {
                                format!("{}  \u{00b7}  {size} across", it.name)
                            } else {
                                format!("{}  \u{00b7}  {size} across, {} away", it.name, wiki::distance_text(it.distance))
                            };
                            if ui.add(egui::Button::new(text).wrap()).clicked() {
                                action = Some(Action::Pick(i));
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
            let pieces = if tab == Tab::Data { m.data_sources() } else { m.sources() };
            if !pieces.is_empty() || m.note.is_some() || m.article_url().is_some() {
                ui.separator();
            }
            if !pieces.is_empty() {
                sources(ui, &pieces);
            }
            if let Some(n) = &m.note {
                ui.label(egui::RichText::new(n).color(gui::warn_colour(ui)));
            }
            if m.article_url().is_some() && ui.button("Open article in browser").clicked() {
                action = Some(Action::OpenBrowser);
            }
        });
    ctx.data_mut(|d| d.insert_temp(tab_id, tab));
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
            item: Some("Q1741".into()),
            languages: Vec::new(),
            infobox: Vec::new(),
        }
    }

    fn model(content: Content) -> Model {
        Model { title: "Vienna".into(), at: Some((48.2, 16.4)), content, note: None, data: None }
    }

    #[test]
    fn the_article_address_is_known_while_it_is_shown() {
        let m = model(Content::Article(article("Text")));
        assert_eq!(m.article_url(), Some("https://en.wikipedia.org/wiki/Vienna"));
        assert_eq!(model(Content::Looking).article_url(), None);
    }

    #[test]
    fn the_sources_link_to_the_article_the_item_and_the_terms() {
        let m = model(Content::Article(article("Text")));
        let links: Vec<_> = m.sources().into_iter().filter_map(|p| if let Piece::Link(_, u) = p { Some(u) } else { None }).collect();
        assert_eq!(links, vec!["https://en.wikipedia.org/wiki/Vienna", CC_BY_SA, "https://www.wikidata.org/wiki/Q1741", CC0]);
        assert!(links.iter().all(|u| is_web_address(u)));
        assert!(model(Content::Looking).sources().is_empty());
    }

    #[test]
    fn only_wikipedia_addresses_reach_the_browser() {
        assert!(open_in_browser("file:///etc/passwd").is_err());
        assert!(open_in_browser("https://evil.example/wiki/x").is_err());
        assert!(open_in_browser("-o /tmp/x").is_err());
    }
}
