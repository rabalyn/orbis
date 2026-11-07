//! Wikipedia and Wikidata, without the network: the addresses to ask, reading the answers, and
//! deciding whether an article is really about the place that was clicked. Everything here is
//! pure and is tested on saved answers (`tests/fixtures/wiki/`).
//!
//! Sources and terms are in docs/PROVENANCE.md. Wikipedia's Action API
//! (https://www.mediawiki.org/wiki/API:Main_page) gives candidates and the text; Wikidata's query
//! service (https://www.mediawiki.org/wiki/Wikidata_Query_Service/User_Manual) gives the facts.

use crate::geocode::percent_encode;
use crate::vector::LabelStyle;
use serde_json::Value;

pub const SPARQL_SERVICE: &str = "https://query.wikidata.org/sparql";
/// The most the API allows for a geographic search (https://www.mediawiki.org/wiki/Extension:GeoData).
pub const MAX_GEO_RADIUS: f64 = 10_000.0;

#[derive(Clone, Debug, PartialEq)]
pub enum WikiError {
    /// Not JSON, or not shaped like an answer.
    Garbled,
    /// The service asked clients to wait (replication lag, rate limit).
    Busy,
    /// Anything else the API said was wrong.
    Api(String),
    Missing,
}

/// A language edition: lower-case letters only, else English.
pub fn language(lang: &str) -> String {
    let code: String = lang.trim().chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
    if (2..=3).contains(&code.len()) {
        code
    } else {
        "en".to_string()
    }
}

const PAGE_FIELDS: &str = "prop=coordinates%7Cpageprops%7Cdescription%7Cinfo&coprimary=primary&colimit=max&ppprop=wikibase_item%7Cdisambiguation&inprop=url";

fn api(lang: &str) -> String {
    // maxlag asks the service to refuse us while its servers are behind, which is its etiquette for programs.
    format!("https://{}.wikipedia.org/w/api.php?action=query&format=json&formatversion=2&maxlag=5", language(lang))
}

/// Pages whose title or text fits the name, with their coordinates.
pub fn search_url(lang: &str, name: &str) -> String {
    format!("{}&generator=search&gsrsearch={}&gsrnamespace=0&gsrlimit=10&{PAGE_FIELDS}", api(lang), percent_encode(name.trim()))
}

/// Pages near a point, nearest first. `radius` is limited to [`MAX_GEO_RADIUS`].
pub fn geo_url(lang: &str, lat: f64, lon: f64, radius: f64, limit: u32) -> String {
    let radius = radius.clamp(10.0, MAX_GEO_RADIUS).round();
    format!(
        "{}&generator=geosearch&ggscoord={lat:.5}%7C{lon:.5}&ggsradius={radius}&ggslimit={}&ggsnamespace=0&{PAGE_FIELDS}",
        api(lang),
        limit.clamp(1, 50)
    )
}

/// The text before the first heading, as plain text.
pub fn extract_url(lang: &str, page_id: u64) -> String {
    format!(
        "{}&pageids={page_id}&prop=extracts%7Cinfo&exintro=1&explaintext=1&exsectionformat=plain&inprop=url",
        api(lang)
    )
}

/// Facts about a Wikidata item, in one query. `None` unless `item` is of the form Q123.
pub fn sparql_url(item: &str, lang: &str) -> Option<String> {
    if !is_item(item) {
        return None;
    }
    let query = SPARQL.replace("@Q@", item).replace("@L@", &language(lang));
    Some(format!("{SPARQL_SERVICE}?format=json&query={}", percent_encode(&query)))
}

pub fn is_item(s: &str) -> bool {
    s.len() > 1 && s.len() < 14 && s.starts_with('Q') && s[1..].bytes().all(|b| b.is_ascii_digit())
}

/// Class, countries, latest population, elevation and area of the best statements, website and
/// time zone. Area and elevation are the normalised values (square metres, metres).
const SPARQL: &str = r#"SELECT ?classes ?classLabels ?countryLabel ?pop ?popDate ?elev ?area ?site ?tzLabel WHERE {
  OPTIONAL { SELECT (GROUP_CONCAT(DISTINCT STRAFTER(STR(?c),"entity/");separator=" ") AS ?classes) (GROUP_CONCAT(DISTINCT ?cl;separator=", ") AS ?classLabels) WHERE { wd:@Q@ wdt:P31 ?c . OPTIONAL { ?c rdfs:label ?cll . FILTER(LANG(?cll)="@L@") } OPTIONAL { ?c rdfs:label ?cle . FILTER(LANG(?cle)="en") } BIND(COALESCE(?cll,?cle) AS ?cl) } }
  OPTIONAL { SELECT ?pop ?popDate WHERE { wd:@Q@ p:P1082 ?s . ?s ps:P1082 ?pop . ?s wikibase:rank ?rk . FILTER(?rk != wikibase:DeprecatedRank) OPTIONAL { ?s pq:P585 ?popDate } } ORDER BY DESC(?popDate) LIMIT 1 }
  OPTIONAL { wd:@Q@ wdt:P17 ?country }
  OPTIONAL { wd:@Q@ wdt:P2044 ?e0 . wd:@Q@ p:P2044 ?es . ?es ps:P2044 ?e0 . ?es psn:P2044 ?ev . ?ev wikibase:quantityAmount ?elev }
  OPTIONAL { wd:@Q@ wdt:P2046 ?a0 . wd:@Q@ p:P2046 ?as . ?as ps:P2046 ?a0 . ?as psn:P2046 ?av . ?av wikibase:quantityAmount ?area }
  OPTIONAL { wd:@Q@ wdt:P856 ?site }
  OPTIONAL { wd:@Q@ wdt:P421 ?tz }
  SERVICE wikibase:label { bd:serviceParam wikibase:language "@L@,en". }
} LIMIT 6"#;

/// An article that might be the one meant.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub page_id: u64,
    pub title: String,
    pub description: String,
    /// The article's own position, on Earth only.
    pub at: Option<(f64, f64)>,
    pub item: Option<String>,
    pub disambiguation: bool,
    pub url: String,
    pub rank: i64,
}

fn check_error(v: &Value) -> Result<(), WikiError> {
    if let Some(e) = v.get("error") {
        let code = e.get("code").and_then(Value::as_str).unwrap_or("");
        return Err(match code {
            "maxlag" | "ratelimited" | "readonly" => WikiError::Busy,
            _ => WikiError::Api(code.chars().take(40).collect()),
        });
    }
    Ok(())
}

fn parse_json(body: &str) -> Result<Value, WikiError> {
    let v: Value = serde_json::from_str(body).map_err(|_| WikiError::Garbled)?;
    check_error(&v)?;
    Ok(v)
}

fn pages(v: &Value) -> Result<&[Value], WikiError> {
    match v.get("query") {
        // Nothing found: the answer has no query part at all.
        None => Ok(&[]),
        Some(q) => q.get("pages").and_then(Value::as_array).map(|a| a.as_slice()).ok_or(WikiError::Garbled),
    }
}

/// Reads the answer of a search or a geographic search, best first.
pub fn parse_candidates(body: &str) -> Result<Vec<Candidate>, WikiError> {
    let v = parse_json(body)?;
    let mut out = Vec::new();
    for p in pages(&v)? {
        if p.get("missing").is_some() || p.get("invalid").is_some() {
            continue;
        }
        let (Some(page_id), Some(title)) = (p.get("pageid").and_then(Value::as_u64), p.get("title").and_then(Value::as_str)) else { continue };
        let url = p.get("fullurl").or_else(|| p.get("canonicalurl")).and_then(Value::as_str).and_then(article_url);
        // An article whose address cannot be shown or opened safely cannot be credited either.
        let Some(url) = url else { continue };
        let at = p.get("coordinates").and_then(Value::as_array).and_then(|list| {
            list.iter().find_map(|c| {
                let earth = c.get("globe").and_then(Value::as_str).is_none_or(|g| g == "earth");
                let (lat, lon) = (c.get("lat")?.as_f64()?, c.get("lon")?.as_f64()?);
                (earth && lat.abs() <= 90.0 && lon.abs() <= 360.0).then_some((lat, lon))
            })
        });
        let props = p.get("pageprops");
        out.push(Candidate {
            page_id,
            title: title.to_string(),
            description: p.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
            at,
            item: props.and_then(|x| x.get("wikibase_item")).and_then(Value::as_str).filter(|s| is_item(s)).map(str::to_string),
            disambiguation: props.is_some_and(|x| x.get("disambiguation").is_some()),
            url,
            rank: p.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX),
        });
    }
    out.sort_by_key(|c| c.rank);
    Ok(out)
}

/// An article's address if it is a plain https address of a Wikipedia language edition and so safe
/// to hand to the browser: no user name, no port, no spaces or quotes.
pub fn article_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let lang = host.strip_suffix(".wikipedia.org")?;
    let lang_ok = !lang.is_empty() && lang.len() <= 12 && lang.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !lang.starts_with('-');
    let path_ok = !path.is_empty() && path.len() < 1500 && path.bytes().all(|b| b.is_ascii_graphic() && !b"\"'<>\\^`{|}".contains(&b));
    (lang_ok && path_ok && (path.starts_with("wiki/") || path.starts_with("w/index.php"))).then(|| url.to_string())
}

/// The text of one article.
#[derive(Clone, Debug, PartialEq)]
pub struct Extract {
    pub title: String,
    pub text: String,
    pub url: String,
}

pub fn parse_extract(body: &str) -> Result<Extract, WikiError> {
    let v = parse_json(body)?;
    let p = pages(&v)?.first().ok_or(WikiError::Missing)?;
    if p.get("missing").is_some() {
        return Err(WikiError::Missing);
    }
    let title = p.get("title").and_then(Value::as_str).ok_or(WikiError::Garbled)?;
    let url = p.get("fullurl").and_then(Value::as_str).and_then(article_url).ok_or(WikiError::Garbled)?;
    let text = p.get("extract").and_then(Value::as_str).unwrap_or("");
    Ok(Extract { title: title.to_string(), text: text.chars().take(20_000).collect(), url })
}

/// What Wikidata knows about an item. Raw values; [`fact_lines`] words them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// Item numbers of what it is an instance of.
    pub classes: Vec<String>,
    pub class_labels: Vec<String>,
    pub countries: Vec<String>,
    /// People, and the year of the count.
    pub population: Option<(f64, Option<i32>)>,
    pub elevation: Option<f64>,
    pub area_m2: Option<f64>,
    pub website: Option<String>,
    pub timezone: Option<String>,
}

pub fn parse_facts(body: &str) -> Result<Facts, WikiError> {
    let v: Value = serde_json::from_str(body).map_err(|_| WikiError::Garbled)?;
    let rows = v.get("results").and_then(|r| r.get("bindings")).and_then(Value::as_array).ok_or(WikiError::Garbled)?;
    let get = |row: &Value, key: &str| row.get(key).and_then(|c| c.get("value")).and_then(Value::as_str).map(str::to_string);
    let mut f = Facts::default();
    let Some(first) = rows.first() else { return Ok(f) };
    f.classes = get(first, "classes").unwrap_or_default().split_whitespace().filter(|q| is_item(q)).map(str::to_string).collect();
    f.class_labels = get(first, "classLabels").unwrap_or_default().split(", ").filter(|s| !s.trim().is_empty()).take(3).map(|s| s.trim().to_string()).collect();
    for row in rows {
        if let Some(c) = get(row, "countryLabel").filter(|c| !c.trim().is_empty() && !is_item(c)) {
            if !f.countries.contains(&c) && f.countries.len() < 3 {
                f.countries.push(c);
            }
        }
    }
    let number = |key: &str| get(first, key).and_then(|s| s.parse::<f64>().ok()).filter(|n| n.is_finite());
    f.population = number("pop").filter(|p| *p >= 0.0).map(|p| (p, get(first, "popDate").and_then(|d| year_of(&d))));
    f.elevation = number("elev");
    f.area_m2 = number("area").filter(|a| *a > 0.0);
    f.website = get(first, "site").filter(|s| (s.starts_with("https://") || s.starts_with("http://")) && s.len() < 200 && s.bytes().all(|b| b.is_ascii_graphic()));
    f.timezone = get(first, "tzLabel").filter(|t| !t.trim().is_empty() && !is_item(t));
    Ok(f)
}

fn year_of(date: &str) -> Option<i32> {
    let digits: String = date.chars().take(4).collect();
    (digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.parse().ok()).flatten()
}

/// Whether Wikidata's classes fit the kind of label. Used for places that have no coordinates, where
/// the name alone must not decide. Item numbers: Q6256 country, Q3624078 sovereign state, Q165 sea,
/// Q9430 ocean, Q1973404 adjacent sea, Q37901 strait, Q39594 bay, Q1210950 channel.
pub fn class_ok(style: LabelStyle, classes: &[String]) -> bool {
    let wanted: &[&str] = match style {
        LabelStyle::Country => &["Q6256", "Q3624078"],
        LabelStyle::Sea => &["Q165", "Q9430", "Q1973404", "Q37901", "Q39594", "Q1210950"],
        _ => return false,
    };
    classes.iter().any(|c| wanted.contains(&c.as_str()))
}

/// Facts as (name, value) rows for the box.
pub fn fact_lines(f: &Facts, lang: &str) -> Vec<(&'static str, String)> {
    let sep = group_separator(lang);
    let mut rows = Vec::new();
    if !f.class_labels.is_empty() {
        rows.push(("Type", f.class_labels.join(", ")));
    }
    match f.countries.len() {
        0 => {}
        1 => rows.push(("Country", f.countries[0].clone())),
        _ => rows.push(("Countries", f.countries.join(", "))),
    }
    if let Some((n, year)) = f.population {
        let year = year.map_or(String::new(), |y| format!(" ({y})"));
        rows.push(("Population", format!("{}{year}", group_digits(n.round(), sep))));
    }
    // A sea level of zero is what Wikidata says about water, not about a place worth stating.
    if let Some(e) = f.elevation.filter(|e| *e != 0.0) {
        rows.push(("Elevation", format!("{} m", short_number(e, sep))));
    }
    if let Some(a) = f.area_m2 {
        rows.push(("Area", if a >= 1.0e6 { format!("{} km\u{b2}", short_number(a / 1.0e6, sep)) } else { format!("{} m\u{b2}", short_number(a, sep)) }));
    }
    if let Some(w) = &f.website {
        rows.push(("Website", w.clone()));
    }
    if let Some(t) = &f.timezone {
        rows.push(("Time zone", t.clone()));
    }
    rows
}

fn group_separator(lang: &str) -> &'static str {
    match language(lang).as_str() {
        "de" | "es" | "it" | "nl" | "pt" | "tr" | "id" | "da" | "el" => ".",
        "fr" | "ru" | "pl" | "cs" | "sv" | "fi" | "nb" | "no" | "uk" | "hu" | "bg" => " ",
        _ => ",",
    }
}

fn group_digits(n: f64, sep: &str) -> String {
    let digits = format!("{:.0}", n.abs());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(c);
    }
    if n < 0.0 {
        out.insert(0, '-');
    }
    out
}

/// Whole numbers from 100 up, one decimal below.
fn short_number(n: f64, sep: &str) -> String {
    if n.abs() >= 100.0 {
        group_digits(n.round(), sep)
    } else {
        let s = format!("{n:.1}");
        let s = s.strip_suffix(".0").unwrap_or(&s).to_string();
        if sep == "," { s } else { s.replace('.', ",") }
    }
}

/// How far a click may be from an article's position and still mean that article, by the kind of label.
/// Wikipedia gives a city one point and a country a rough centre, so the kinds differ a lot.
pub fn radius_m(style: LabelStyle) -> Option<f64> {
    Some(match style {
        LabelStyle::Peak => 3.0e3,
        LabelStyle::Local => 4.0e3,
        LabelStyle::Village => 6.0e3,
        LabelStyle::Town => 1.2e4,
        LabelStyle::City => 3.0e4,
        LabelStyle::Lake => 1.0e5,
        LabelStyle::State => 1.0e6,
        LabelStyle::Sea => 2.5e6,
        LabelStyle::Country => 3.0e6,
        LabelStyle::Road => return None,
    })
}

/// Countries and seas are accepted only when Wikidata also says the article is that kind of thing.
pub fn needs_class(style: LabelStyle) -> bool {
    matches!(style, LabelStyle::Country | LabelStyle::Sea)
}

/// Labels small enough for a search of the surroundings to find their article.
pub fn small(style: LabelStyle) -> bool {
    matches!(style, LabelStyle::Peak | LabelStyle::Local | LabelStyle::Village | LabelStyle::Town | LabelStyle::City)
}

/// Great-circle distance in metres on a sphere of Earth's mean radius.
pub fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let (dp, dl) = ((b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().min(1.0).asin()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// This candidate is the article.
    Match(usize),
    /// Same name and no position of its own: it is it only if Wikidata says it is the right kind of thing.
    NeedClass(usize),
    None,
}

/// Decides which candidate, if any, is about the label `name` of this kind at this position. The name
/// must be the article's title (ignoring a bracketed or comma-separated qualifier), and the article's
/// position must lie within the kind's radius; when two same-named places both could be meant and
/// one is not clearly nearer, nothing is chosen. A wrong article is worse than none.
pub fn choose(name: &str, style: LabelStyle, at: (f64, f64), cands: &[Candidate]) -> Choice {
    let Some(radius) = radius_m(style) else { return Choice::None };
    let named: Vec<(usize, &Candidate)> = cands.iter().enumerate().filter(|(_, c)| !c.disambiguation && name_matches(name, &c.title)).collect();
    let mut near: Vec<(usize, f64)> = named.iter().filter_map(|(i, c)| c.at.map(|p| (*i, distance_m(at, p)))).filter(|(_, d)| *d <= radius).collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    if let Some(&(best, d)) = near.first() {
        let rival = near.get(1).is_some_and(|(_, d2)| *d2 < 3.0 * d + 1000.0);
        return if rival { Choice::None } else { Choice::Match(best) };
    }
    if matches!(style, LabelStyle::Country | LabelStyle::Sea) {
        let bare: Vec<usize> = named.iter().filter(|(_, c)| c.at.is_none() && c.item.is_some()).map(|(i, _)| *i).collect();
        // Another article of that name with a position elsewhere would be a different place.
        let elsewhere = named.iter().any(|(_, c)| c.at.is_some());
        if let [only] = bare[..] {
            if !elsewhere {
                return Choice::NeedClass(only);
            }
        }
    }
    Choice::None
}

/// Whether a label is the title of an article, once the qualifier Wikipedia adds to tell
/// same-named places apart ("Paris (Texas)", "Paris, Texas") is taken off.
pub fn name_matches(label: &str, title: &str) -> bool {
    let want = fold_name(label);
    if want.is_empty() {
        return false;
    }
    let mut forms = vec![title.to_string()];
    if let Some((head, _)) = title.rsplit_once(" (") {
        forms.push(head.to_string());
    }
    if let Some((head, _)) = title.split_once(", ") {
        forms.push(head.to_string());
    }
    forms.iter().any(|f| fold_name(f) == want)
}

/// Lower case, accents off, punctuation and spacing reduced to single spaces.
pub fn fold_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        for c in c.to_lowercase() {
            let mapped = match fold_char(c) {
                Some(f) => f.to_lowercase(),
                None => c.to_string(),
            };
            for m in mapped.chars() {
                if m.is_alphanumeric() {
                    out.push(m);
                } else if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
        }
    }
    out.trim().to_string()
}

const EXT_A: [&str; 128] = [
    "A", "a", "A", "a", "A", "a", "C", "c", "C", "c", "C", "c", "C", "c", "D", "d", //
    "D", "d", "E", "e", "E", "e", "E", "e", "E", "e", "E", "e", "G", "g", "G", "g", //
    "G", "g", "G", "g", "H", "h", "H", "h", "I", "i", "I", "i", "I", "i", "I", "i", //
    "I", "i", "IJ", "ij", "J", "j", "K", "k", "k", "L", "l", "L", "l", "L", "l", "L", //
    "l", "L", "l", "N", "n", "N", "n", "N", "n", "n", "N", "n", "O", "o", "O", "o", //
    "O", "o", "OE", "oe", "R", "r", "R", "r", "R", "r", "S", "s", "S", "s", "S", "s", //
    "S", "s", "T", "t", "T", "t", "T", "t", "U", "u", "U", "u", "U", "u", "U", "u", //
    "U", "u", "U", "u", "W", "w", "Y", "y", "Y", "Z", "z", "Z", "z", "Z", "z", "s",
];

/// The plain Latin letters for an accented one outside Latin-1 (Latin Extended-A, some of B), if
/// it has any, and for the Latin-1 letters with accents too.
fn fold_char(c: char) -> Option<String> {
    let code = c as u32;
    match code {
        0xC0..=0xC5 => Some("A".into()),
        0xE0..=0xE5 => Some("a".into()),
        0xC7 => Some("C".into()),
        0xE7 => Some("c".into()),
        0xC8..=0xCB => Some("E".into()),
        0xE8..=0xEB => Some("e".into()),
        0xCC..=0xCF => Some("I".into()),
        0xEC..=0xEF => Some("i".into()),
        0xD1 => Some("N".into()),
        0xF1 => Some("n".into()),
        0xD2..=0xD6 | 0xD8 => Some("O".into()),
        0xF2..=0xF6 | 0xF8 => Some("o".into()),
        0xD9..=0xDC => Some("U".into()),
        0xF9..=0xFC => Some("u".into()),
        0xDD => Some("Y".into()),
        0xFD | 0xFF => Some("y".into()),
        0xC6 => Some("AE".into()),
        0xE6 => Some("ae".into()),
        0xDF => Some("ss".into()),
        0xD0 => Some("D".into()),
        0xF0 => Some("d".into()),
        0xDE => Some("Th".into()),
        0xFE => Some("th".into()),
        0x100..=0x17F => Some(EXT_A[(code - 0x100) as usize].into()),
        0x218 => Some("S".into()),
        0x219 => Some("s".into()),
        0x21A => Some("T".into()),
        0x21B => Some("t".into()),
        _ => None,
    }
}

/// Makes text fit the built-in font (ASCII and Latin-1): accented letters outside Latin-1 become
/// plain ones, typographic punctuation its simple form, and pronunciation guides are dropped. What
/// cannot be shown becomes one `?` per run.
pub fn display_text(text: &str) -> String {
    let text = text.replace('\r', "");
    let text = drop_pronunciations(&text);
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let mapped: String = match c {
            '\n' => "\n".into(),
            '\t' => " ".into(),
            c if (c as u32) < 0x20 || c == '\u{7f}' => String::new(),
            c if (c as u32) < 0x7f || (0xA0..=0xFF).contains(&(c as u32)) => {
                // Latin-1 letters are in the font; only the soft hyphen has to go.
                if c == '\u{ad}' { String::new() } else { c.to_string() }
            }
            '\u{2013}' | '\u{2014}' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2212}' => "-".into(),
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' | '\u{2bc}' => "'".into(),
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => "\"".into(),
            '\u{2026}' => "...".into(),
            '\u{2022}' | '\u{b7}' => "*".into(),
            '\u{2002}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => " ".into(),
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}' | '\u{feff}' | '\u{2066}'..='\u{2069}' | '\u{301}' | '\u{300}' | '\u{308}' => String::new(),
            c => fold_char(c).unwrap_or_else(|| "\u{1}".into()),
        };
        out.push_str(&mapped);
    }
    tidy(&collapse_unknown(&out))
}

/// Runs of characters the font lacks become a single `?`.
fn collapse_unknown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = false;
    for c in s.chars() {
        if c == '\u{1}' {
            if !run {
                out.push('?');
            }
            run = true;
        } else if c == ' ' && run {
            // Spaces inside a foreign phrase stay part of the one question mark.
        } else {
            if run && c != '\n' && !out.ends_with('?') {
                out.push(' ');
            } else if run {
                // keep the run's question mark
            }
            run = false;
            out.push(c);
        }
    }
    out
}

fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.split('\n') {
        let mut l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        for (from, to) in [("( ", "("), (" )", ")"), (" ,", ","), (" ;", ";"), (" .", "."), ("(, ", "("), ("(; ", "(")] {
            while l.contains(from) {
                l = l.replace(from, to);
            }
        }
        for empty in ["()", "(?)", "(? )"] {
            l = l.replace(empty, "");
        }
        out.push_str(l.trim());
        out.push('\n');
    }
    // No more than one empty line in a row, and none at the ends.
    let mut result = String::new();
    let mut blanks = 0;
    for line in out.lines() {
        if line.is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        result.push_str(line);
        result.push('\n');
    }
    result.trim().to_string()
}

fn is_phonetic(c: char) -> bool {
    matches!(c as u32, 0x250..=0x2FF | 0x300..=0x36F | 0x1D00..=0x1DBF)
}

/// Removes pronunciation guides: bracketed text with phonetic letters, and the parts of a
/// parenthesis (between semicolons) that have them.
fn drop_pronunciations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let (open, close) = match chars[i] {
            '(' => ('(', ')'),
            '[' => ('[', ']'),
            // Full-width brackets of East Asian text.
            '\u{ff08}' => ('\u{ff08}', '\u{ff09}'),
            c => {
                out.push(c);
                i += 1;
                continue;
            }
        };
        // The group ends at its matching bracket, at most 400 characters on; else it is plain text.
        let mut depth = 0;
        let mut end = None;
        for (j, c) in chars.iter().enumerate().skip(i).take(400) {
            if *c == open {
                depth += 1;
            } else if *c == close {
                depth -= 1;
                if depth == 0 {
                    end = Some(j);
                    break;
                }
            }
        }
        let Some(end) = end else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let inner: String = chars[i + 1..end].iter().collect();
        let inner = drop_pronunciations(&inner);
        if open == '[' {
            if !inner.chars().any(is_phonetic) && !text_has_phonetic(&chars[i + 1..end]) {
                out.push('[');
                out.push_str(&inner);
                out.push(']');
            }
        } else {
            let kept: Vec<&str> = inner.split(';').filter(|seg| !seg.chars().any(is_phonetic)).collect();
            if !kept.is_empty() && !kept.join("").trim().is_empty() {
                out.push(open);
                out.push_str(&kept.join(";"));
                out.push(close);
            }
        }
        i = end + 1;
    }
    out
}

fn text_has_phonetic(chars: &[char]) -> bool {
    chars.iter().any(|c| is_phonetic(*c))
}

/// Breaks text into lines of at most `cols` characters, at spaces, and inside a word that is longer
/// than a line. Empty lines (paragraph breaks) are kept.
pub fn wrap_lines(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut lines = Vec::new();
    for source in text.split('\n') {
        let mut line = String::new();
        let mut len = 0;
        for word in source.split(' ').filter(|w| !w.is_empty()) {
            let mut chars: Vec<char> = word.chars().collect();
            while chars.len() > cols {
                if len > 0 {
                    lines.push(std::mem::take(&mut line));
                    len = 0;
                }
                lines.push(chars.drain(..cols).collect());
            }
            let n = chars.len();
            if len > 0 && len + 1 + n > cols {
                lines.push(std::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.extend(chars);
            len += n;
        }
        lines.push(line);
    }
    lines
}

/// The place's name as written on its label. Peak labels read "▲ Name 1126 m": the sign and
/// the height are not part of the name.
pub fn label_name(text: &str, style: LabelStyle) -> String {
    let mut name = text.trim().trim_start_matches(|c: char| !c.is_alphanumeric()).trim().to_string();
    if style == LabelStyle::Peak {
        if let Some(rest) = name.strip_suffix(" m") {
            let digits = rest.chars().rev().take_while(|c| c.is_ascii_digit()).count();
            if digits > 0 && rest[..rest.len() - digits].ends_with(' ') {
                name = rest[..rest.len() - digits].trim_end().to_string();
            }
        }
    }
    name
}

/// Cuts a text to `cols` characters with three dots if it is longer.
pub fn shorten(text: &str, cols: usize) -> String {
    if text.chars().count() <= cols {
        return text.to_string();
    }
    let keep = cols.saturating_sub(3);
    let mut s: String = text.chars().take(keep).collect();
    s.push_str("...");
    s.chars().take(cols).collect()
}

/// "47.0012 N 12.6945 E".
pub fn coords_text(lat: f64, lon: f64) -> String {
    format!("{:.4}\u{b0} {}  {:.4}\u{b0} {}", lat.abs(), if lat < 0.0 { 'S' } else { 'N' }, lon.abs(), if lon < 0.0 { 'W' } else { 'E' })
}

/// "850 m", "12.3 km".
pub fn distance_text(m: f64) -> String {
    if m < 1000.0 {
        format!("{} m", (m / 10.0).round() as i64 * 10)
    } else if m < 100_000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{} km", (m / 1000.0).round() as i64)
    }
}

/// How far to look around a click on open ground from this height (metres): a few times what is
/// seen under the pointer, within what the service allows.
pub fn ground_radius(altitude_m: f64) -> f64 {
    (altitude_m * 0.4).clamp(1000.0, MAX_GEO_RADIUS)
}

/// An article near a point.
#[derive(Clone, Debug, PartialEq)]
pub struct Near {
    pub cand: Candidate,
    pub distance: f64,
}

/// The `n` nearest real articles (not disambiguation pages) that have a position.
pub fn nearest(cands: &[Candidate], at: (f64, f64), n: usize) -> Vec<Near> {
    let mut list: Vec<Near> = cands.iter().filter(|c| !c.disambiguation).filter_map(|c| c.at.map(|p| Near { cand: c.clone(), distance: distance_m(at, p) })).collect();
    list.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    list.truncate(n);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/wiki/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn cands(name: &str) -> Vec<Candidate> {
        parse_candidates(&fixture(name)).unwrap()
    }

    fn pick(label: &str, style: LabelStyle, at: (f64, f64), file: &str) -> Option<String> {
        let list = cands(file);
        match choose(label, style, at, &list) {
            Choice::Match(i) | Choice::NeedClass(i) => Some(list[i].title.clone()),
            Choice::None => None,
        }
    }

    #[test]
    fn labels_choose_the_article_of_the_right_place() {
        use LabelStyle::*;
        // (label, kind, position, saved answer, expected article)
        type Row<'a> = (&'a str, LabelStyle, (f64, f64), &'a str, Option<&'a str>);
        let table: &[Row] = &[
            ("Vienna", City, (48.208, 16.372), "search_vienna_en.json", Some("Vienna")),
            // Articles about Vienna are not about the city: the name must be the title.
            ("Vienna", City, (48.208, 16.372), "search_paris_en.json", None),
            ("Paris", City, (48.857, 2.352), "search_paris_en.json", Some("Paris")),
            // The text search does not list Paris, Texas; a search around the label finds it, and
            // the same label in France finds nothing there.
            ("Paris", Town, (33.66, -95.555), "search_paris_en.json", None),
            ("Paris", Town, (33.66, -95.555), "geo_paris_tx_en.json", Some("Paris, Texas")),
            ("Paris", City, (48.857, 2.352), "geo_paris_tx_en.json", None),
            ("Springfield", City, (39.80, -89.64), "search_springfield_en.json", Some("Springfield, Illinois")),
            ("Springfield", Town, (0.0, 0.0), "search_springfield_en.json", None),
            ("Georgia", Country, (42.0, 43.5), "search_georgia_en.json", Some("Georgia (country)")),
            ("Georgia", State, (33.0, -83.0), "search_georgia_en.json", Some("Georgia (U.S. state)")),
            // By position and name alone this would pass; a country label also has to be a country (below).
            ("Georgia", Country, (33.0, -83.0), "search_georgia_en.json", Some("Georgia (U.S. state)")),
            ("Mediterranean Sea", Sea, (35.0, 18.0), "search_mediterranean_en.json", Some("Mediterranean Sea")),
            // A sea label on the other side of the world does not get that sea's article.
            ("Mediterranean Sea", Sea, (-30.0, -150.0), "search_mediterranean_en.json", None),
            ("Matterhorn", Peak, (45.9766, 7.6585), "search_matterhorn_en.json", Some("Matterhorn")),
            ("Matterhorn", Peak, (46.5, 8.5), "search_matterhorn_en.json", None),
            // Mercury: the planet, the element and the god are not a place in Nevada.
            ("Mercury", Town, (36.66, -115.99), "search_mercury_en.json", None),
            ("Mercury", Town, (36.66, -115.99), "geo_mercury_nv_en.json", Some("Mercury, Nevada")),
            ("Mercury", Town, (36.66, -115.99), "geo_paris_tx_en.json", None),
            ("Hallstatt", Village, (47.562, 13.649), "search_hallstatt_de.json", Some("Hallstatt")),
        ];
        for (label, style, at, file, want) in table {
            assert_eq!(pick(label, *style, *at, file).as_deref(), *want, "{label} {style:?} {at:?} in {file}");
        }
    }

    #[test]
    fn same_name_places_close_together_are_not_guessed() {
        let c = |title: &str, lat: f64, lon: f64| Candidate {
            page_id: 1,
            title: title.into(),
            description: String::new(),
            at: Some((lat, lon)),
            item: None,
            disambiguation: false,
            url: "https://en.wikipedia.org/wiki/X".into(),
            rank: 0,
        };
        let two = [c("Sprungfeld, Alpha", 10.0, 10.0), c("Sprungfeld, Beta", 10.0, 10.03)];
        // The label lies between two places of that name, 3 km apart: no choice.
        assert_eq!(choose("Sprungfeld", LabelStyle::Town, (10.0, 10.015), &two), Choice::None);
        // Right on one of them, the other 3 km away is clearly the second.
        assert_eq!(choose("Sprungfeld", LabelStyle::Town, (10.0, 10.0), &two), Choice::Match(0));
    }

    #[test]
    fn disambiguation_pages_and_other_planets_never_match() {
        let list = cands("search_georgia_en.json");
        let page = list.iter().find(|c| c.title == "Georgia").unwrap();
        assert!(page.disambiguation);
        let moon = br#"{"query":{"pages":[{"pageid":5,"title":"Tycho","index":1,"fullurl":"https://en.wikipedia.org/wiki/Tycho","coordinates":[{"lat":-43.3,"lon":-11.2,"primary":true,"globe":"moon"}]}]}}"#;
        let list = parse_candidates(std::str::from_utf8(moon).unwrap()).unwrap();
        assert_eq!(list[0].at, None);
        assert_eq!(choose("Tycho", LabelStyle::Peak, (-43.3, -11.2), &list), Choice::None);
    }

    #[test]
    fn countries_and_seas_without_position_need_the_right_kind() {
        let bare = |title: &str, item: &str, at: Option<(f64, f64)>| Candidate {
            page_id: 2,
            title: title.into(),
            description: String::new(),
            at,
            item: Some(item.into()),
            disambiguation: false,
            url: "https://en.wikipedia.org/wiki/X".into(),
            rank: 0,
        };
        let one = [bare("Atlantis Sea", "Q9", None)];
        assert_eq!(choose("Atlantis Sea", LabelStyle::Sea, (0.0, 0.0), &one), Choice::NeedClass(0));
        assert_eq!(choose("Atlantis Sea", LabelStyle::Lake, (0.0, 0.0), &one), Choice::None);
        // The same name with a position elsewhere is another thing entirely.
        let two = [bare("Atlantis Sea", "Q9", None), bare("Atlantis Sea", "Q10", Some((60.0, 20.0)))];
        assert_eq!(choose("Atlantis Sea", LabelStyle::Sea, (0.0, 0.0), &two), Choice::None);
        let q = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(class_ok(LabelStyle::Sea, &q(&["Q166620", "Q1973404"])));
        assert!(class_ok(LabelStyle::Country, &q(&["Q3624078", "Q6256"])));
        assert!(!class_ok(LabelStyle::Country, &q(&["Q35657"])));
        assert!(!class_ok(LabelStyle::Peak, &q(&["Q6256"])));
        // The state Georgia under a country label is turned down by its class.
        assert!(needs_class(LabelStyle::Country) && !needs_class(LabelStyle::City));
        let state = parse_facts(&fixture("sparql_q1428_en.json")).unwrap();
        assert!(!class_ok(LabelStyle::Country, &state.classes));
    }

    #[test]
    fn roads_are_never_looked_up() {
        let list = cands("search_vienna_en.json");
        assert_eq!(choose("Vienna", LabelStyle::Road, (48.2, 16.37), &list), Choice::None);
    }

    #[test]
    fn names_compare_without_case_accents_or_qualifiers() {
        assert!(name_matches("Zürich", "Zurich"));
        assert!(name_matches("MÜNCHEN", "München"));
        assert!(name_matches("Łódź", "Lodz"));
        assert!(name_matches("St. Louis", "St. Louis, Missouri"));
        assert!(name_matches("Mercury", "Mercury (planet)"));
        assert!(!name_matches("Paris", "Paris Hilton"));
        assert!(!name_matches("Vienna", "Congress of Vienna"));
        assert!(!name_matches("", ""));
        assert!(name_matches("東京都", "東京都"));
        assert!(!name_matches("東京", "大阪"));
    }

    #[test]
    fn candidates_are_read_from_a_saved_answer() {
        let list = cands("search_vienna_en.json");
        assert_eq!(list[0].title, "Vienna");
        assert_eq!(list[0].item.as_deref(), Some("Q1741"));
        assert_eq!(list[0].url, "https://en.wikipedia.org/wiki/Vienna");
        assert!(list[0].at.is_some() && !list[0].disambiguation);
        assert!(list.windows(2).all(|w| w[0].rank <= w[1].rank));
        assert!(list.iter().any(|c| c.disambiguation));
    }

    #[test]
    fn broken_answers_are_errors_not_panics() {
        for body in ["", "<html>", "[]", "{\"query\":{\"pages\":3}}", "{\"query\":{\"pages\":[1,\"a\",null]}}", "{\"query\":{\"pages\":[{\"pageid\":\"x\"}]}}"] {
            let r = parse_candidates(body);
            assert!(r.is_err() || r.unwrap().is_empty(), "{body}");
            let _ = parse_extract(body);
            let _ = parse_facts(body);
        }
        assert_eq!(parse_candidates("{\"batchcomplete\":true}"), Ok(vec![]));
        assert_eq!(parse_candidates(r#"{"error":{"code":"maxlag","info":"waiting"}}"#), Err(WikiError::Busy));
        assert_eq!(parse_extract(r#"{"query":{"pages":[{"pageid":1,"missing":true}]}}"#), Err(WikiError::Missing));
        // Truncated in the middle.
        let text = fixture("search_vienna_en.json");
        assert!(parse_candidates(&text[..text.len() / 2]).is_err());
    }

    #[test]
    fn unsafe_article_addresses_are_refused() {
        assert!(article_url("https://en.wikipedia.org/wiki/Vienna").is_some());
        assert!(article_url("https://de.wikipedia.org/wiki/K%C3%B6ln").is_some());
        for bad in [
            "http://en.wikipedia.org/wiki/Vienna",
            "https://en.wikipedia.org.evil.example/wiki/Vienna",
            "https://evil.example/wiki/Vienna",
            "https://user@en.wikipedia.org/wiki/Vienna",
            "https://en.wikipedia.org:8443/wiki/Vienna",
            "https://en.wikipedia.org/wiki/a b",
            "https://en.wikipedia.org/wiki/a\"b",
            "https://en.wikipedia.org/",
            "https://wikipedia.org.example/wiki/x",
            "https://EN.wikipedia.org/wiki/x",
            "https://en.wikipedia.org/other",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(article_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn extract_and_facts_are_read_from_saved_answers() {
        let e = parse_extract(&fixture("extract_vienna_en.json")).unwrap();
        assert_eq!(e.title, "Vienna");
        assert!(e.text.starts_with("Vienna ("));
        let f = parse_facts(&fixture("sparql_q1741_en.json")).unwrap();
        assert!(f.classes.contains(&"Q133442".to_string()));
        assert_eq!(f.countries, ["Austria"]);
        assert_eq!(f.population, Some((2028289.0, Some(2025))));
        assert_eq!(f.area_m2, Some(414_830_000.0));
        let rows = fact_lines(&f, "en");
        assert!(rows.contains(&("Population", "2,028,289 (2025)".to_string())), "{rows:?}");
        assert!(rows.contains(&("Area", "415 km\u{b2}".to_string())), "{rows:?}");
        assert!(rows.contains(&("Country", "Austria".to_string())));
        let de = fact_lines(&f, "de");
        assert!(de.contains(&("Population", "2.028.289 (2025)".to_string())));
        let sea = parse_facts(&fixture("sparql_q4918_en.json")).unwrap();
        assert!(class_ok(LabelStyle::Sea, &sea.classes));
        assert_eq!(sea.countries, ["Spain", "Italy", "Greece"]);
        assert!(fact_lines(&sea, "en").iter().all(|r| r.0 != "Elevation"));
        let de = parse_facts(&fixture("sparql_q183_en.json")).unwrap();
        assert!(class_ok(LabelStyle::Country, &de.classes) && !class_ok(LabelStyle::Sea, &de.classes));
        let ocean = parse_facts(&fixture("sparql_q98_en.json")).unwrap();
        assert!(class_ok(LabelStyle::Sea, &ocean.classes));
        assert_eq!(parse_facts(r#"{"results":{"bindings":[]}}"#), Ok(Facts::default()));
    }

    #[test]
    fn urls_are_encoded_and_bounded() {
        let u = search_url("de", "Zürich & Co");
        assert!(u.starts_with("https://de.wikipedia.org/w/api.php?action=query&format=json"), "{u}");
        assert!(u.contains("gsrsearch=Z%C3%BCrich%20%26%20Co&") && u.contains("maxlag=5"));
        // A hostile language code cannot change the host.
        assert!(search_url("../evil.com/", "x").starts_with("https://en.wikipedia.org/"));
        assert!(search_url("en.evil", "x").starts_with("https://en.wikipedia.org/"));
        assert!(geo_url("en", 1.0, 2.0, 50_000.0, 500).contains("ggsradius=10000&ggslimit=50"));
        assert!(geo_url("en", 1.0, 2.0, 1.0, 5).contains("ggsradius=10&"));
        assert!(extract_url("en", 7).contains("pageids=7&"));
        assert!(sparql_url("Q1741", "de").unwrap().contains("wd%3AQ1741"));
        assert_eq!(sparql_url("Q1741 . } #", "de"), None);
        assert_eq!(sparql_url("1741", "de"), None);
        assert_eq!(language("de_DE.UTF-8"), "de");
        assert_eq!(language(""), "en");
    }

    #[test]
    fn nearby_articles_are_sorted_by_distance() {
        let list = cands("geo_kals_en.json");
        let near = nearest(&list, (47.0, 12.69), 5);
        assert!(!near.is_empty() && near.len() <= 5);
        assert!(near.windows(2).all(|w| w[0].distance <= w[1].distance));
        assert!(near[0].distance < 10_000.0);
        assert!(ground_radius(100.0) == 1000.0 && ground_radius(1.0e9) == MAX_GEO_RADIUS);
        assert_eq!(distance_text(850.0), "850 m");
        assert_eq!(distance_text(12_340.0), "12.3 km");
        assert_eq!(distance_text(250_000.0), "250 km");
        assert_eq!(coords_text(-47.5, 12.25), "47.5000\u{b0} S  12.2500\u{b0} E");
    }

    #[test]
    fn text_is_made_to_fit_the_font() {
        let e = parse_extract(&fixture("extract_vienna_en.json")).unwrap();
        let t = display_text(&e.text);
        assert!(t.starts_with("Vienna (German: Wien; Austro-Bavarian: Wean) is the capital"), "{}", &t[..120]);
        assert!(t.chars().all(|c| c == '\n' || (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c)));
        assert_eq!(display_text("Dvo\u{159}\u{e1}k \u{2013} \u{201c}Pra\u{17e}sk\u{fd}\u{201d}"), "Dvor\u{e1}k - \"Prazsk\u{fd}\"");
        assert_eq!(display_text("a\r\n\n\n\nb ( ) c"), "a\n\nb c");
        assert_eq!(display_text("x (\u{1f600}\u{1f600})"), "x");
    }

    #[test]
    fn cjk_and_right_to_left_text_survive() {
        for file in ["extract_ja.json", "extract_ar.json"] {
            let e = parse_extract(&fixture(file)).unwrap();
            let t = display_text(&e.text);
            assert!(!t.is_empty());
            assert!(t.chars().all(|c| c == '\n' || (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c)), "{file}");
            for line in wrap_lines(&t, 20) {
                assert!(line.chars().count() <= 20);
            }
            // Wrapping the raw text works as well.
            for line in wrap_lines(&e.text, 7) {
                assert!(line.chars().count() <= 7);
            }
        }
    }

    #[test]
    fn peak_labels_lose_their_sign_and_height() {
        assert_eq!(label_name("\u{25b2} Wehrkogel 1126 m", LabelStyle::Peak), "Wehrkogel");
        assert_eq!(label_name("\u{25b2} Gro\u{df}er Priel", LabelStyle::Peak), "Gro\u{df}er Priel");
        assert_eq!(label_name("\u{25b2} Mont Blanc 4806 m", LabelStyle::Peak), "Mont Blanc");
        assert_eq!(label_name("  Vienna ", LabelStyle::City), "Vienna");
        // A height is cut from peaks only, and only as a number of its own.
        assert_eq!(label_name("Route 66 m", LabelStyle::City), "Route 66 m");
        assert_eq!(label_name("Fort 7m", LabelStyle::Peak), "Fort 7m");
        assert_eq!(label_name("\u{25b2}", LabelStyle::Peak), "");
    }

    #[test]
    fn long_words_are_broken_inside() {
        let lines = wrap_lines("short Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch end\n\nnext", 10);
        assert!(lines.iter().all(|l| l.chars().count() <= 10), "{lines:?}");
        assert_eq!(lines.join("").replace(' ', ""), "shortLlanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogochendnext");
        assert_eq!(wrap_lines("a\n\nb", 5), ["a", "", "b"]);
        assert_eq!(wrap_lines("", 5), [""]);
        assert_eq!(wrap_lines("x", 0), ["x"]);
        assert_eq!(shorten("abcdefghij", 6), "abc...");
        assert_eq!(shorten("abc", 6), "abc");
        assert_eq!(shorten("abc", 0), "");
    }
}
