//! The search box: text editing, result choice and place parsing. Independent of the window
//! system and of the network; `geocode.rs` fetches, `main.rs` connects keys and drawing.

use crate::camera::FOV_Y;
use crate::geo::radius;
use std::f64::consts::PI;

/// Credit for results that come from the geocoding service, shown while they are in use.
pub const CREDIT: &str = "Search: Nominatim, © OpenStreetMap contributors";
const MAX_RESULTS: usize = 8;
const MAX_QUERY_CHARS: usize = 200;
const MAX_ADDRESS_CHARS: usize = crate::services::MAX_URL_CHARS;
const NAME_COLUMNS: usize = 60;
/// How far from a point the view stops when nothing says how big the place is, in metres.
const POINT_DISTANCE: f64 = 4000.0;
/// What a typed position flies to, in metres.
const COORDINATE_DISTANCE: f64 = 10_000.0;
/// Room around a place's bounding box.
const MARGIN: f64 = 1.4;
const NO_NAMED_SEARCH: &str = "Searching by name works on Earth only. Type coordinates such as 46.5, 11.8.";

/// A place found by name. Angles are in radians; the box is (south, west, north, east).
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub bbox: Option<[f64; 4]>,
}

/// Where to fly, as a view point and a distance in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub lat: f64,
    pub lon: f64,
    pub distance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Failure {
    /// Offline, and the query is not in the cache.
    Offline,
    /// The server could not be reached, or asked us to wait.
    Unreachable,
    /// The answer made no sense.
    Garbled,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Editing,
    Searching(String),
    Results(Vec<Place>),
    NoResult,
    Failed(Failure),
    Message(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Text(String),
    Backspace,
    Enter,
    Escape,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    /// Ask the geocoding service for this text.
    Request(String),
    Fly(Target),
    /// Save the current view under this name; empty asks for a default.
    Save(String),
    /// Add the map service at this address (and optional layer name) to the library.
    AddService(String),
}

pub struct Search {
    open: bool,
    text: String,
    state: State,
    selected: usize,
    /// Named results were shown, so their credit is owed.
    used_service: bool,
    can_name: bool,
    /// The box asks for a name for the view instead of searching.
    saving: bool,
    /// The box asks for the address of a map service instead of searching.
    adding: bool,
}

impl Search {
    /// `can_name` is false on bodies the service knows nothing about.
    pub fn new(can_name: bool) -> Self {
        Self { open: false, text: String::new(), state: State::Editing, selected: 0, used_service: false, can_name, saving: false, adding: false }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.saving = false;
        self.adding = false;
    }

    /// Opens the box to type the address of a map service.
    pub fn open_add(&mut self) {
        self.open = true;
        self.saving = false;
        self.adding = true;
        self.text.clear();
        self.state = State::Editing;
    }

    /// Opens the box to name the current view.
    pub fn open_save(&mut self) {
        self.open = true;
        self.saving = true;
        self.adding = false;
        self.text.clear();
        self.state = State::Editing;
    }

    pub fn is_searching(&self) -> bool {
        matches!(self.state, State::Searching(_))
    }

    /// Whether results from the service have been shown, so that the credit is due.
    pub fn credit_due(&self) -> bool {
        self.used_service
    }

    /// Where the chosen result is, while results are listed.
    pub fn selected_target(&self) -> Option<Target> {
        let State::Results(places) = &self.state else { return None };
        let p = places.get(self.selected)?;
        Some(Target { lat: p.lat, lon: p.lon, distance: distance_for(p) })
    }

    fn edited(&mut self) {
        self.state = State::Editing;
        self.selected = 0;
    }

    pub fn key(&mut self, key: Key) -> Action {
        if !self.open {
            return Action::None;
        }
        match key {
            Key::Text(s) => {
                for c in s.chars().filter(|c| !c.is_control()) {
                    if self.text.chars().count() < if self.adding { MAX_ADDRESS_CHARS } else { MAX_QUERY_CHARS } {
                        self.text.push(c);
                    }
                }
                self.edited();
                Action::None
            }
            Key::Backspace => {
                self.text.pop();
                self.edited();
                Action::None
            }
            Key::Escape => {
                self.open = false;
                self.saving = false;
                self.adding = false;
                Action::None
            }
            Key::Up | Key::Down => {
                if let State::Results(places) = &self.state {
                    let n = places.len();
                    self.selected = if key == Key::Up { (self.selected + n - 1) % n } else { (self.selected + 1) % n };
                }
                Action::None
            }
            Key::Enter => self.enter(),
        }
    }

    fn enter(&mut self) -> Action {
        if self.adding {
            self.open = false;
            self.adding = false;
            return Action::AddService(self.text.trim().to_string());
        }
        if self.saving {
            self.open = false;
            self.saving = false;
            return Action::Save(self.text.trim().to_string());
        }
        if matches!(self.state, State::Results(_)) {
            if let Some(target) = self.selected_target() {
                self.open = false;
                return Action::Fly(target);
            }
        }
        let query = self.text.trim().to_string();
        if query.is_empty() || self.is_searching() {
            return Action::None;
        }
        if let Some((lat, lon)) = parse_coordinates(&query) {
            self.open = false;
            return Action::Fly(Target { lat, lon, distance: crate::body::scaled(COORDINATE_DISTANCE) });
        }
        if !self.can_name {
            self.state = State::Message(NO_NAMED_SEARCH);
            return Action::None;
        }
        self.state = State::Searching(query.clone());
        Action::Request(query)
    }

    /// The service answered `query`. Answers to anything but the current search are dropped.
    pub fn arrived(&mut self, query: &str, answer: Result<Vec<Place>, Failure>) {
        if self.state != State::Searching(query.to_string()) {
            return;
        }
        self.selected = 0;
        self.state = match answer {
            Ok(mut places) => {
                places.truncate(MAX_RESULTS);
                if places.is_empty() {
                    State::NoResult
                } else {
                    self.used_service = true;
                    State::Results(places)
                }
            }
            Err(f) => State::Failed(f),
        };
    }

    /// The text of the panel, or nothing while the box is closed.
    pub fn panel_text(&self) -> String {
        if !self.open {
            return String::new();
        }
        if self.adding {
            return format!("Map service address (then a layer name, if it has layers): {}_\nEnter: add   Esc: cancel", self.text);
        }
        if self.saving {
            return format!("Save place as: {}_\nEnter: save (empty: name it by position)   Esc: cancel", self.text);
        }
        let mut s = format!("Search: {}_", self.text);
        let mut hint = "Enter: search   Esc: close";
        match &self.state {
            State::Editing => {}
            State::Searching(_) => s.push_str("\nSearching..."),
            State::NoResult => s.push_str("\nNo place found."),
            State::Failed(Failure::Offline) => s.push_str("\nOffline: only earlier searches can be repeated."),
            State::Failed(Failure::Unreachable) => s.push_str("\nThe search server cannot be reached. Try again later."),
            State::Failed(Failure::Garbled) => s.push_str("\nThe search server gave an answer that could not be read."),
            State::Message(m) => {
                s.push('\n');
                s.push_str(m);
            }
            State::Results(places) => {
                for (i, p) in places.iter().enumerate() {
                    let mark = if i == self.selected { '>' } else { ' ' };
                    s.push_str(&format!("\n{mark} {}", shorten(&p.name, NAME_COLUMNS)));
                }
                s.push('\n');
                s.push_str(CREDIT);
                hint = "Enter: fly there   Up/Down: choose   Esc: close";
            }
        }
        s.push('\n');
        s.push_str(hint);
        s
    }
}

fn shorten(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let mut out: String = name.chars().take(max - 3).collect();
    out.push_str("...");
    out
}

/// How far to be from a place so that its bounding box fills most of the view.
pub fn distance_for(p: &Place) -> f64 {
    let Some([south, west, north, east]) = p.bbox else {
        return crate::body::scaled(POINT_DISTANCE);
    };
    let mut dlon = east - west;
    if dlon < 0.0 {
        dlon += 2.0 * PI;
    }
    let dlat = north - south;
    let mid = (south + north) / 2.0;
    let angle = dlat.max(dlon * mid.cos().abs()).min(PI);
    let extent = angle * radius();
    let distance = extent * MARGIN / (2.0 * (FOV_Y / 2.0).tan());
    distance.clamp(crate::body::scaled(500.0), crate::nav::whole_body())
}

/// Reads a position typed as text: "46.5, 11.8", "46.5N 11.8E", "N 46°30' E 11°48'" or
/// "46°30'15\"N 11°48'E". Latitude comes first unless letters say otherwise.
pub fn parse_coordinates(text: &str) -> Option<(f64, f64)> {
    let tokens = tokenize(text)?;
    let hems = tokens.iter().filter(|t| matches!(t, Tok::Hem(_))).count();
    let (a, b) = if hems == 0 { split_plain(&tokens)? } else { split_lettered(&tokens, hems)? };
    let (lat, lon) = match (a.hem, b.hem) {
        (None, None) => (a.value()?, b.value()?),
        (Some(x), Some(y)) if "NS".contains(x) && "EW".contains(y) => (a.signed()?, b.signed()?),
        (Some(x), Some(y)) if "EW".contains(x) && "NS".contains(y) => (b.signed()?, a.signed()?),
        _ => return None,
    };
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then(|| (lat.to_radians(), lon.to_radians()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Unit {
    None,
    Deg,
    Min,
    Sec,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Num(f64, Unit),
    Hem(char),
    Sep,
}

fn tokenize(text: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            ',' | ';' => {
                out.push(Tok::Sep);
                i += 1;
            }
            '0'..='9' | '.' | '-' | '+' | '−' => {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().map(|&c| if c == '−' { '-' } else { c }).collect();
                let value: f64 = s.parse().ok()?;
                let unit = match chars.get(i) {
                    Some('°' | 'º') => Unit::Deg,
                    Some('\'' | '′' | '’') => Unit::Min,
                    Some('"' | '″' | '”') => Unit::Sec,
                    _ => Unit::None,
                };
                if unit != Unit::None {
                    i += 1;
                    // Two apostrophes stand for the seconds mark.
                    if unit == Unit::Min && matches!(chars.get(i), Some('\'' | '’')) {
                        i += 1;
                        out.push(Tok::Num(value, Unit::Sec));
                        continue;
                    }
                }
                out.push(Tok::Num(value, unit));
            }
            _ => {
                let h = c.to_ascii_uppercase();
                if "NSEW".contains(h) && c.is_ascii() {
                    out.push(Tok::Hem(h));
                    i += 1;
                } else {
                    return None;
                }
            }
        }
    }
    Some(out)
}

/// One half of a position: degrees, minutes and seconds as typed, and a letter.
#[derive(Debug, Default, Clone)]
struct Part {
    nums: Vec<f64>,
    hem: Option<char>,
}

impl Part {
    fn value(&self) -> Option<f64> {
        let (d, m, s) = (*self.nums.first()?, self.nums.get(1).copied().unwrap_or(0.0), self.nums.get(2).copied().unwrap_or(0.0));
        if self.nums.len() > 3 || (self.nums.len() > 1 && (d.fract() != 0.0 || !(0.0..60.0).contains(&m))) || !(0.0..60.0).contains(&s) {
            return None;
        }
        if self.nums.len() == 3 && m.fract() != 0.0 {
            return None;
        }
        let sign = if d < 0.0 || (d == 0.0 && d.is_sign_negative()) { -1.0 } else { 1.0 };
        Some(sign * (d.abs() + m / 60.0 + s / 3600.0))
    }

    /// The value for a part with a letter: S and W count as negative, and a minus sign too.
    fn signed(&self) -> Option<f64> {
        let v = self.value()?;
        let west = matches!(self.hem, Some('S' | 'W'));
        Some(if west { -v.abs() } else { v })
    }
}

fn split_plain(tokens: &[Tok]) -> Option<(Part, Part)> {
    let mut parts: Vec<Part> = vec![Part::default()];
    let mut any_unit = false;
    for t in tokens {
        match *t {
            Tok::Sep => {
                parts.push(Part::default());
            }
            Tok::Num(v, unit) => {
                any_unit |= unit != Unit::None;
                let cur = parts.last_mut()?;
                // A new degree mark, or a plain number after a plain number, starts the other half.
                let starts_new = !cur.nums.is_empty() && (unit == Unit::Deg || (!any_unit && unit == Unit::None));
                if starts_new {
                    parts.push(Part::default());
                }
                parts.last_mut()?.nums.push(v);
            }
            Tok::Hem(_) => return None,
        }
    }
    parts.retain(|p| !p.nums.is_empty());
    let [a, b] = <[Part; 2]>::try_from(parts).ok()?;
    Some((a, b))
}

fn split_lettered(tokens: &[Tok], hems: usize) -> Option<(Part, Part)> {
    if hems != 2 {
        return None;
    }
    let prefix = matches!(tokens.first(), Some(Tok::Hem(_)));
    let mut parts = vec![Part::default()];
    for t in tokens {
        match *t {
            Tok::Sep => {}
            Tok::Num(v, _) => {
                let cur = parts.last_mut()?;
                if prefix && cur.hem.is_none() {
                    return None;
                }
                if !prefix && cur.hem.is_some() {
                    parts.push(Part::default());
                }
                parts.last_mut()?.nums.push(v);
            }
            Tok::Hem(h) => {
                if prefix && (!parts.last()?.nums.is_empty() || parts.last()?.hem.is_some()) {
                    parts.push(Part::default());
                }
                let cur = parts.last_mut()?;
                if cur.hem.is_some() {
                    return None;
                }
                cur.hem = Some(h);
            }
        }
    }
    let [a, b] = <[Part; 2]>::try_from(parts).ok()?;
    (!a.nums.is_empty() && !b.nums.is_empty()).then_some((a, b))
}

/// Reads the answer of a Nominatim `search` request in `jsonv2` format. Entries without a usable
/// position are skipped; an answer that is not a list is an error.
pub fn parse_nominatim(body: &str) -> Result<Vec<Place>, Failure> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| Failure::Garbled)?;
    let list = value.as_array().ok_or(Failure::Garbled)?;
    Ok(list.iter().filter_map(place_from).collect())
}

fn number(v: &serde_json::Value) -> Option<f64> {
    let n = match v {
        serde_json::Value::String(s) => s.trim().parse().ok()?,
        serde_json::Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    f64::is_finite(n).then_some(n)
}

fn place_from(v: &serde_json::Value) -> Option<Place> {
    let lat = number(v.get("lat")?)?;
    let lon = number(v.get("lon")?)?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let name = v
        .get("display_name")
        .or_else(|| v.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())?;
    // The service lists south, north, west, east.
    let bbox = v.get("boundingbox").and_then(|b| b.as_array()).and_then(|b| {
        let n: Vec<f64> = b.iter().filter_map(number).collect();
        let [s, nth, w, e] = <[f64; 4]>::try_from(n).ok()?;
        let ok = (-90.0..=90.0).contains(&s) && (-90.0..=90.0).contains(&nth) && s <= nth && (-180.0..=180.0).contains(&w) && (-180.0..=180.0).contains(&e);
        ok.then(|| [s.to_radians(), w.to_radians(), nth.to_radians(), e.to_radians()])
    });
    Some(Place { name, lat: lat.to_radians(), lon: lon.to_radians(), bbox })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deg(c: Option<(f64, f64)>) -> Option<(f64, f64)> {
        c.map(|(a, b)| ((a.to_degrees() * 1e6).round() / 1e6, (b.to_degrees() * 1e6).round() / 1e6))
    }

    #[test]
    fn plain_coordinates() {
        assert_eq!(deg(parse_coordinates("46.5, 11.8")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates("46.5 11.8")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates(" -33.86,151.21 ")), Some((-33.86, 151.21)));
        assert_eq!(deg(parse_coordinates("46.5;-11.8")), Some((46.5, -11.8)));
    }

    #[test]
    fn lettered_coordinates() {
        assert_eq!(deg(parse_coordinates("46.5N 11.8E")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates("46.5 S, 11.8 w")), Some((-46.5, -11.8)));
        assert_eq!(deg(parse_coordinates("N46.5 E11.8")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates("11.8E 46.5N")), Some((46.5, 11.8)));
    }

    #[test]
    fn degrees_minutes_seconds() {
        assert_eq!(deg(parse_coordinates("46°30'N 11°48'E")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates("46°30′00″N, 11°48′36″E")), Some((46.5, 11.81)));
        assert_eq!(deg(parse_coordinates("46°30'00\"S 11°48'W")), Some((-46.5, -11.8)));
        assert_eq!(deg(parse_coordinates("N 46 30 E 11 48")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates("46°30' 11°48'")), Some((46.5, 11.8)));
        assert_eq!(deg(parse_coordinates("46°30'N 11°48'E")), deg(parse_coordinates("46 30 N 11 48 E")));
    }

    #[test]
    fn not_coordinates() {
        for t in ["", "Paris", "80331", "46.5", "91, 10", "10, 181", "46.5N 11.8N", "46.5E 11.8E", "1 2 3", "N 46 30 61 E 11", "Paris 12, 14", "46.5N 11.8", "1,2,3", "46°75'N 11E"] {
            assert_eq!(parse_coordinates(t), None, "{t}");
        }
    }

    #[test]
    fn typing_and_searching() {
        let mut s = Search::new(true);
        assert_eq!(s.key(Key::Text("x".into())), Action::None, "closed box ignores keys");
        s.open();
        s.key(Key::Text("Zürich".into()));
        s.key(Key::Text("\u{1b}\n".into()));
        s.key(Key::Text(" 日本".into()));
        assert!(s.panel_text().contains("Zürich 日本_"));
        s.key(Key::Backspace);
        s.key(Key::Backspace);
        assert!(s.panel_text().contains("Zürich _"));
        assert_eq!(s.key(Key::Enter), Action::Request("Zürich".into()));
        assert!(s.is_searching());
        assert_eq!(s.key(Key::Enter), Action::None, "no second request while one is out");
        assert!(s.panel_text().contains("Searching"));
    }

    fn two_places() -> Vec<Place> {
        vec![
            Place { name: "A".into(), lat: 0.1, lon: 0.2, bbox: None },
            Place { name: "B".into(), lat: 0.3, lon: 0.4, bbox: Some([0.29, 0.39, 0.31, 0.41]) },
        ]
    }

    fn with_results() -> Search {
        let mut s = Search::new(true);
        s.open();
        s.key(Key::Text("ab".into()));
        s.key(Key::Enter);
        s.arrived("ab", Ok(two_places()));
        s
    }

    #[test]
    fn choosing_a_result() {
        let mut s = with_results();
        assert!(s.credit_due());
        assert!(s.panel_text().contains("> A") && s.panel_text().contains(CREDIT));
        s.key(Key::Down);
        assert!(s.panel_text().contains("> B"));
        s.key(Key::Down);
        assert!(s.panel_text().contains("> A"), "wraps around");
        s.key(Key::Up);
        let Action::Fly(t) = s.key(Key::Enter) else { panic!("should fly") };
        assert_eq!((t.lat, t.lon), (0.3, 0.4));
        assert!(!s.is_open());
        assert_eq!(s.panel_text(), "");
    }

    #[test]
    fn editing_discards_results() {
        let mut s = with_results();
        s.key(Key::Backspace);
        assert!(!s.panel_text().contains("> A"));
        assert_eq!(s.key(Key::Enter), Action::Request("a".into()));
    }

    #[test]
    fn answers_for_an_old_text_are_dropped() {
        let mut s = Search::new(true);
        s.open();
        s.key(Key::Text("one".into()));
        s.key(Key::Enter);
        s.key(Key::Backspace);
        s.arrived("one", Ok(two_places()));
        assert!(!s.panel_text().contains("> A"));
        assert!(!s.credit_due());
        // Closing the box does not cancel the search; the answer is there on reopening.
        s.key(Key::Enter);
        s.key(Key::Escape);
        s.open();
        s.arrived("on", Ok(two_places()));
        assert!(s.panel_text().contains("> A"));
    }

    #[test]
    fn failures_have_words() {
        for (f, word) in [(Failure::Offline, "Offline"), (Failure::Unreachable, "cannot be reached"), (Failure::Garbled, "could not be read")] {
            let mut s = Search::new(true);
            s.open();
            s.key(Key::Text("q".into()));
            s.key(Key::Enter);
            s.arrived("q", Err(f));
            assert!(s.panel_text().contains(word));
        }
        let mut s = Search::new(true);
        s.open();
        s.key(Key::Text("q".into()));
        s.key(Key::Enter);
        s.arrived("q", Ok(vec![]));
        assert!(s.panel_text().contains("No place found"));
        assert!(!s.credit_due());
    }

    #[test]
    fn coordinates_need_no_request_and_names_may_be_disabled() {
        let mut s = Search::new(false);
        s.open();
        s.key(Key::Text("46.5, 11.8".into()));
        assert!(matches!(s.key(Key::Enter), Action::Fly(_)));
        s.open();
        s.key(Key::Text(" x".into()));
        assert_eq!(s.key(Key::Enter), Action::None);
        assert!(s.panel_text().contains("Earth only"));
    }

    #[test]
    fn escape_closes_and_empty_does_nothing() {
        let mut s = Search::new(true);
        s.open();
        assert_eq!(s.key(Key::Enter), Action::None);
        s.key(Key::Escape);
        assert!(!s.is_open());
    }

    #[test]
    fn long_names_are_shortened() {
        let long = "x".repeat(100);
        assert_eq!(shorten(&long, 20).chars().count(), 20);
        assert_eq!(shorten("short", 20), "short");
    }

    #[test]
    fn distance_follows_the_box() {
        let small = Place { name: "s".into(), lat: 0.8, lon: 0.2, bbox: Some([0.79999, 0.19999, 0.80001, 0.20001]) };
        let town = Place { name: "t".into(), lat: 0.8, lon: 0.2, bbox: Some([0.7984, 0.1984, 0.8016, 0.2016]) };
        let country = Place { name: "c".into(), lat: 0.8, lon: 0.2, bbox: Some([0.6, 0.0, 1.0, 0.4]) };
        let point = Place { bbox: None, ..small.clone() };
        assert!(distance_for(&small) < distance_for(&town));
        assert!(distance_for(&town) < distance_for(&country));
        assert!(distance_for(&country) <= crate::nav::whole_body());
        assert_eq!(distance_for(&point), POINT_DISTANCE);
        assert_eq!(distance_for(&small), 500.0);
        // The box of a 20 km town fills the view: about 20 km * 1.4 / (2 tan 22.5°).
        let m = distance_for(&town);
        assert!(m > 25_000.0 && m < 60_000.0, "{m}");
    }

    #[test]
    fn a_box_across_the_antimeridian_is_narrow() {
        let p = Place { name: "f".into(), lat: 0.0, lon: PI, bbox: Some([-0.01, 3.13, 0.01, -3.13]) };
        assert!(distance_for(&p) < 1.0e6);
    }

    const FIXTURE: &str = r#"[
      {"place_id":1,"lat":"46.4983","lon":"11.3548","display_name":"Bolzano - Bozen, South Tyrol, Italy","boundingbox":["46.4,"]},
      {"place_id":2,"lat":"46.5","lon":"11.35","display_name":"  Bolzano\n Hill ","boundingbox":["46.4","46.6","11.2","11.5"]},
      {"lat":"x","lon":"1","display_name":"bad position"},
      {"lat":"95","lon":"1","display_name":"off the globe"},
      {"lat":"10","lon":"20"},
      {"lat":10,"lon":20.5,"name":"Numbers"},
      "junk", null, 7
    ]"#;

    #[test]
    fn nominatim_answers_are_read_defensively() {
        let places = parse_nominatim(FIXTURE).unwrap();
        let names: Vec<_> = places.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Bolzano - Bozen, South Tyrol, Italy", "Bolzano Hill", "Numbers"]);
        assert_eq!(places[0].bbox, None, "a short box is ignored, the place kept");
        let b = places[1].bbox.unwrap();
        assert!((b[0].to_degrees() - 46.4).abs() < 1e-9 && (b[3].to_degrees() - 11.5).abs() < 1e-9);
        assert!((places[1].lat.to_degrees() - 46.5).abs() < 1e-9);
    }

    #[test]
    fn nominatim_garbage_is_an_error() {
        assert_eq!(parse_nominatim("[]"), Ok(vec![]));
        for t in ["", "<html>", r#"{"error":{"code":400}}"#, "[{", "42"] {
            assert_eq!(parse_nominatim(t), Err(Failure::Garbled), "{t}");
        }
    }

    #[test]
    fn boxes_that_make_no_sense_are_dropped() {
        let t = r#"[{"lat":"1","lon":"2","display_name":"x","boundingbox":["5","1","0","3"]}]"#;
        assert_eq!(parse_nominatim(t).unwrap()[0].bbox, None);
    }

    #[test]
    fn the_box_can_name_a_view() {
        let mut s = Search::new(true);
        s.open_save();
        assert!(s.panel_text().starts_with("Save place as: _"));
        s.key(Key::Text("My hut".into()));
        assert_eq!(s.key(Key::Enter), Action::Save("My hut".into()));
        assert!(!s.is_open());
        s.open_save();
        assert_eq!(s.key(Key::Enter), Action::Save(String::new()), "an empty name asks for the default");
        s.open_save();
        s.key(Key::Escape);
        s.open();
        s.key(Key::Text("46.5, 11.8".into()));
        assert!(matches!(s.key(Key::Enter), Action::Fly(_)), "search works again after saving");
    }
}
