//! Looks up what Wikipedia and Wikidata say about a clicked place, on a thread of its own so that
//! no frame waits for it.
//!
//! Requests go out only for a click, one after the other, at least `MIN_GAP` apart, through the
//! fetcher (pauses, back-off, user agent, `--offline`), and every answer is kept on disk for a
//! week. A click makes at most five requests: a search (a second one in English when the interface
//! language has no match), a search around the label for small places, the facts, and the text.

use crate::event_feeds::{pack, unix_now, unpack};
use crate::fetcher::{FetchError, Fetcher};
use crate::geocode::MinGap;
use crate::vector::LabelStyle;
use crate::wiki::{self, Candidate, Choice, Facts, WikiError};
use crate::worker::Worker;
use std::sync::Arc;
use crate::wtime::{Duration, Instant};

/// Wikipedia asks for serial requests; this keeps them from following each other too closely.
const MIN_GAP: Duration = Duration::from_millis(300);
/// Articles change slowly; a stored answer is used for a week, and past that when offline or unreachable.
const MAX_AGE: f64 = 7.0 * 86_400.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Label { name: String, style: LabelStyle, lat: f64, lon: f64 },
    /// Open ground, seen from this height in metres.
    Ground { lat: f64, lon: f64, altitude: f64 },
    /// One article of the nearby list.
    Page { lang: String, page_id: u64, item: Option<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// Offline, and nothing stored.
    Offline,
    Unreachable,
    /// The service asked us to come back later.
    Busy,
    Garbled,
}

impl Failure {
    pub fn message(&self) -> &'static str {
        match self {
            Failure::Offline => "Offline: nothing is stored for this place.",
            Failure::Unreachable => "Wikipedia could not be reached.",
            Failure::Busy => "Wikipedia or Wikidata asked to wait. Try again in a few minutes.",
            Failure::Garbled => "Wikipedia gave an answer that could not be read.",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Article {
    pub lang: String,
    pub title: String,
    pub description: String,
    pub facts: Vec<(String, String)>,
    pub text: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NearItem {
    pub lang: String,
    pub page_id: u64,
    pub title: String,
    pub item: Option<String>,
    pub distance: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Article(Article),
    Nearby { items: Vec<NearItem>, radius: f64 },
    NoArticle,
    Failed(Failure),
}

#[derive(Debug)]
pub struct Reply {
    pub id: u64,
    pub outcome: Outcome,
    /// Requests that went to the network for this click (stored answers are free).
    pub requests: usize,
}

pub struct Lookup {
    worker: Worker<(u64, Job), Reply>,
}

impl Lookup {
    pub fn new(fetcher: Arc<Fetcher>, lang: &str) -> Self {
        let lang = wiki::language(lang);
        let mut gap = MinGap::new(MIN_GAP);
        // Only the newest click matters if the user was quicker than the network.
        let worker = Worker::new(true, MIN_GAP, move |(id, job): (u64, Job)| {
            let mut net = Net { fetcher: &fetcher, gap: std::mem::replace(&mut gap, MinGap::new(MIN_GAP)), sent: 0 };
            let outcome = run(&mut net, &lang, &job).unwrap_or_else(Outcome::Failed);
            gap = net.gap;
            Reply { id, outcome, requests: net.sent }
        });
        Self { worker }
    }

    pub fn submit(&self, id: u64, job: Job) {
        self.worker.submit((id, job));
    }

    pub fn poll(&self) -> Option<Reply> {
        self.worker.poll()
    }
}

struct Net<'a> {
    fetcher: &'a Fetcher,
    gap: MinGap,
    sent: usize,
}

pub(crate) fn cache_name(url: &str) -> String {
    // FNV-1a, which unlike the standard hasher is the same in every version.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("wikipedia/{h:016x}.json")
}

fn failure(e: WikiError) -> Failure {
    match e {
        WikiError::Busy => Failure::Busy,
        _ => Failure::Garbled,
    }
}

impl Net<'_> {
    /// The answer to a request: from disk if it is fresh (or if offline, or the service cannot be
    /// reached and a stale one exists), else from the network. Only answers that parse are kept.
    fn get<T>(&mut self, url: &str, parse: impl Fn(&str) -> Result<T, WikiError>) -> Result<T, Failure> {
        let name = cache_name(url);
        let stored = self.fetcher.cache().get(&name);
        let parsed = stored.as_deref().and_then(unpack).and_then(|(t, b)| parse(b).ok().map(|v| (t, v)));
        let offline = self.fetcher.is_offline();
        let fresh = parsed.as_ref().is_some_and(|(t, _)| unix_now() - t < MAX_AGE);
        if fresh || (offline && parsed.is_some()) {
            return parsed.map(|p| p.1).ok_or(Failure::Offline);
        }
        if offline {
            return Err(Failure::Offline);
        }
        crate::wtime::pause(self.gap.wait(Instant::now()));
        self.gap.sent(Instant::now());
        self.sent += 1;
        let stale = |parsed: Option<(f64, T)>, f: Failure| parsed.map(|p| p.1).ok_or(f);
        let data = match self.fetcher.fetch_service(url) {
            Ok(d) => d,
            // A server that has been timed out or that asked us to slow down is not "unreachable".
            Err(FetchError::Paused(_) | FetchError::Busy(_)) => return stale(parsed, Failure::Busy),
            Err(_) => return stale(parsed, Failure::Unreachable),
        };
        let Ok(body) = std::str::from_utf8(&data) else { return stale(parsed, Failure::Garbled) };
        match parse(body) {
            Ok(v) => {
                if let Err(e) = self.fetcher.cache().put(&name, &pack(unix_now(), &data)) {
                    eprintln!("cache: could not store a Wikipedia answer: {e}");
                }
                Ok(v)
            }
            Err(e) => stale(parsed, failure(e)),
        }
    }

    fn facts(&mut self, item: &str, lang: &str) -> Result<Facts, Failure> {
        let url = wiki::sparql_url(item, lang).ok_or(Failure::Garbled)?;
        self.get(&url, wiki::parse_facts)
    }
}

fn run(net: &mut Net, lang: &str, job: &Job) -> Result<Outcome, Failure> {
    match job {
        Job::Label { name, style, lat, lon } => label(net, lang, name, *style, (*lat, *lon)),
        Job::Ground { lat, lon, altitude } => ground(net, lang, (*lat, *lon), *altitude),
        Job::Page { lang, page_id, item } => {
            let article = article(net, lang, *page_id, "", item.as_deref(), None)?;
            Ok(Outcome::Article(article))
        }
    }
}

struct Found {
    lang: String,
    cand: Candidate,
    facts: Option<Facts>,
}

/// Checks one list of candidates; for countries and seas Wikidata must also agree on the kind.
fn accept(net: &mut Net, lang: &str, name: &str, style: LabelStyle, at: (f64, f64), list: &[Candidate]) -> Result<Option<Found>, Failure> {
    let (i, must) = match wiki::choose(name, style, at, list) {
        Choice::None => return Ok(None),
        Choice::Match(i) => (i, wiki::needs_class(style)),
        Choice::NeedClass(i) => (i, true),
    };
    let cand = list[i].clone();
    let mut facts = None;
    if must {
        let Some(item) = cand.item.as_deref() else { return Ok(None) };
        let f = net.facts(item, lang)?;
        if !wiki::class_ok(style, &f.classes) {
            return Ok(None);
        }
        facts = Some(f);
    }
    Ok(Some(Found { lang: lang.to_string(), cand, facts }))
}

fn label(net: &mut Net, lang: &str, name: &str, style: LabelStyle, at: (f64, f64)) -> Result<Outcome, Failure> {
    let Some(radius) = wiki::radius_m(style).filter(|_| !name.trim().is_empty()) else { return Ok(Outcome::NoArticle) };
    let mut langs = vec![lang.to_string()];
    if lang != "en" {
        langs.push("en".to_string());
    }
    let mut found = None;
    for l in &langs {
        let list = net.get(&wiki::search_url(l, name), wiki::parse_candidates)?;
        found = accept(net, l, name, style, at, &list)?;
        if found.is_some() {
            break;
        }
    }
    // The text search ranks by words, not by place, so a small place with a common name can be
    // missing from it; the articles around the label's position are looked through too.
    if found.is_none() && wiki::small(style) {
        let list = net.get(&wiki::geo_url(lang, at.0, at.1, radius.min(wiki::MAX_GEO_RADIUS), 50), wiki::parse_candidates)?;
        found = accept(net, lang, name, style, at, &list)?;
    }
    let Some(f) = found else { return Ok(Outcome::NoArticle) };
    let article = article(net, &f.lang, f.cand.page_id, &f.cand.description, f.cand.item.as_deref(), f.facts)?;
    Ok(Outcome::Article(article))
}

fn article(net: &mut Net, lang: &str, page_id: u64, description: &str, item: Option<&str>, facts: Option<Facts>) -> Result<Article, Failure> {
    let e = net.get(&wiki::extract_url(lang, page_id), wiki::parse_extract)?;
    // The facts are a bonus: without them the article is still shown.
    let facts = facts.or_else(|| item.and_then(|q| net.facts(q, lang).ok()));
    let rows = facts.as_ref().map(|f| wiki::fact_lines(f, lang)).unwrap_or_default();
    Ok(Article {
        lang: lang.to_string(),
        title: wiki::display_text(&e.title),
        description: wiki::display_text(description),
        facts: rows.into_iter().map(|(k, v)| (k.to_string(), wiki::display_text(&v))).collect(),
        text: wiki::display_text(&e.text),
        url: e.url,
    })
}

fn ground(net: &mut Net, lang: &str, at: (f64, f64), altitude: f64) -> Result<Outcome, Failure> {
    let radius = wiki::ground_radius(altitude);
    let mut langs = vec![lang.to_string()];
    if lang != "en" {
        langs.push("en".to_string());
    }
    for l in langs {
        let list = net.get(&wiki::geo_url(&l, at.0, at.1, radius, 20), wiki::parse_candidates)?;
        let items: Vec<NearItem> = wiki::nearest(&list, at, 5)
            .into_iter()
            .map(|n| NearItem { lang: l.clone(), page_id: n.cand.page_id, title: wiki::display_text(&n.cand.title), item: n.cand.item, distance: n.distance })
            .collect();
        if !items.is_empty() {
            return Ok(Outcome::Nearby { items, radius });
        }
    }
    Ok(Outcome::Nearby { items: Vec::new(), radius })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::Peers;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/wiki/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// An offline fetcher whose cache holds the given (address, saved answer) pairs.
    fn offline_with(tag: &str, stored: &[(String, String)]) -> Arc<Fetcher> {
        let dir = std::env::temp_dir().join(format!("orbis-wiki-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = Arc::new(Fetcher::new(crate::cache::DiskCache::new(dir), Peers::default(), true));
        for (url, body) in stored {
            f.cache().put(&cache_name(url), &pack(unix_now(), body.as_bytes())).unwrap();
        }
        f
    }

    fn wait(l: &Lookup) -> Reply {
        for _ in 0..500 {
            if let Some(r) = l.poll() {
                return r;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no reply");
    }

    fn vienna() -> Vec<(String, String)> {
        vec![
            (wiki::search_url("en", "Vienna"), fixture("search_vienna_en.json")),
            (wiki::extract_url("en", 55866), fixture("extract_vienna_en.json")),
            (wiki::sparql_url("Q1741", "en").unwrap(), fixture("sparql_q1741_en.json")),
        ]
    }

    #[test]
    fn a_city_label_gives_the_article_with_facts_from_stored_answers() {
        let f = offline_with("city", &vienna());
        let l = Lookup::new(f.clone(), "en");
        l.submit(7, Job::Label { name: "Vienna".into(), style: LabelStyle::City, lat: 48.208, lon: 16.372 });
        let r = wait(&l);
        assert_eq!(r.id, 7);
        assert_eq!(r.requests, 0);
        let Outcome::Article(a) = r.outcome else { panic!("{:?}", r.outcome) };
        assert_eq!(a.title, "Vienna");
        assert_eq!(a.description, "Capital and largest city of Austria");
        assert_eq!(a.url, "https://en.wikipedia.org/wiki/Vienna");
        assert!(a.facts.iter().any(|(k, v)| k == "Population" && v.starts_with("2,028,289")), "{:?}", a.facts);
        assert!(a.text.starts_with("Vienna (German: Wien"));
        assert_eq!(f.traffic().downloads, 0);
    }

    #[test]
    fn the_wrong_place_gets_no_article() {
        let mut stored = vienna();
        // Nothing lies around that spot.
        stored.push((wiki::geo_url("en", -33.0, 150.0, 10_000.0, 50), "{\"batchcomplete\":true}".to_string()));
        let f = offline_with("wrong", &stored);
        let l = Lookup::new(f, "en");
        // The same name, but on the other side of the world.
        l.submit(1, Job::Label { name: "Vienna".into(), style: LabelStyle::City, lat: -33.0, lon: 150.0 });
        assert_eq!(wait(&l).outcome, Outcome::NoArticle);
        l.submit(2, Job::Label { name: "Vienna".into(), style: LabelStyle::Road, lat: 48.208, lon: 16.372 });
        assert_eq!(wait(&l).outcome, Outcome::NoArticle);
    }

    #[test]
    fn a_state_is_not_taken_for_a_country_by_its_class() {
        let mut stored = vec![
            (wiki::search_url("en", "Georgia"), fixture("search_georgia_en.json")),
            (wiki::sparql_url("Q1428", "en").unwrap(), fixture("sparql_q1428_en.json")),
        ];
        let l = Lookup::new(offline_with("state", &stored), "en");
        l.submit(1, Job::Label { name: "Georgia".into(), style: LabelStyle::Country, lat: 33.0, lon: -83.0 });
        assert_eq!(wait(&l).outcome, Outcome::NoArticle);
        // A sea with the stored class passes the check, and then needs its text.
        stored.push((wiki::search_url("en", "Mediterranean Sea"), fixture("search_mediterranean_en.json")));
        stored.push((wiki::sparql_url("Q4918", "en").unwrap(), fixture("sparql_q4918_en.json")));
        let id = parse_id(&fixture("search_mediterranean_en.json"), "Mediterranean Sea");
        stored.push((wiki::extract_url("en", id), fixture("extract_vienna_en.json").replace("\"Vienna\"", "\"Mediterranean Sea\"")));
        let l = Lookup::new(offline_with("sea", &stored), "en");
        l.submit(2, Job::Label { name: "Mediterranean Sea".into(), style: LabelStyle::Sea, lat: 35.0, lon: 18.0 });
        let Outcome::Article(a) = wait(&l).outcome else { panic!("sea") };
        assert_eq!(a.title, "Mediterranean Sea");
        assert!(a.facts.iter().any(|(k, v)| k == "Type" && v.contains("adjacent sea")));
    }

    fn parse_id(body: &str, title: &str) -> u64 {
        wiki::parse_candidates(body).unwrap().into_iter().find(|c| c.title == title).unwrap().page_id
    }

    #[test]
    fn open_ground_lists_the_nearest_articles() {
        let url = wiki::geo_url("en", 47.0, 12.69, wiki::ground_radius(20_000.0), 20);
        let l = Lookup::new(offline_with("ground", &[(url, fixture("geo_kals_en.json"))]), "en");
        l.submit(3, Job::Ground { lat: 47.0, lon: 12.69, altitude: 20_000.0 });
        let Outcome::Nearby { items, radius } = wait(&l).outcome else { panic!("nearby") };
        assert!(radius <= wiki::MAX_GEO_RADIUS);
        assert!(!items.is_empty() && items.len() <= 5);
        assert!(items.windows(2).all(|w| w[0].distance <= w[1].distance));
    }

    #[test]
    fn offline_with_nothing_stored_says_so_and_sends_nothing() {
        let f = offline_with("empty", &[]);
        let l = Lookup::new(f.clone(), "de");
        l.submit(1, Job::Label { name: "Wien".into(), style: LabelStyle::City, lat: 48.2, lon: 16.4 });
        assert_eq!(wait(&l).outcome, Outcome::Failed(Failure::Offline));
        l.submit(2, Job::Ground { lat: 1.0, lon: 2.0, altitude: 1000.0 });
        assert_eq!(wait(&l).outcome, Outcome::Failed(Failure::Offline));
        assert_eq!(f.traffic().downloads, 0);
    }

    #[test]
    fn damaged_stored_answers_are_not_trusted() {
        let url = wiki::search_url("en", "Vienna");
        let f = offline_with("damaged", &[(url, "<html>not json".to_string())]);
        let l = Lookup::new(f, "en");
        l.submit(1, Job::Label { name: "Vienna".into(), style: LabelStyle::City, lat: 48.2, lon: 16.4 });
        assert_eq!(wait(&l).outcome, Outcome::Failed(Failure::Offline));
    }

    #[test]
    fn only_the_newest_queued_job_is_answered() {
        let l = Lookup::new(offline_with("newest", &vienna()), "en");
        for id in 1..=20 {
            l.submit(id, Job::Ground { lat: 0.0, lon: 0.0, altitude: 1.0 });
        }
        let mut last = 0;
        std::thread::sleep(Duration::from_millis(200));
        while let Some(r) = l.poll() {
            last = r.id;
        }
        assert_eq!(last, 20);
    }
}
