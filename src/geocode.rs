//! Looks up place names with Nominatim, on a thread of its own so that no frame waits for it.
//!
//! The public server's usage policy (https://operations.osmfoundation.org/policies/nominatim/)
//! allows one request per second at most, forbids autocomplete, wants an identifying user agent,
//! attribution, and caching of repeated queries. So a request goes out only when the user presses
//! Enter, at least `MIN_INTERVAL` after the previous one, and every answer is kept on disk for `MAX_AGE`.

use crate::event_feeds::{pack, unix_now, unpack};
use crate::fetcher::{FetchError, Fetcher};
use crate::search::{self, Failure, Place};
use crate::worker::Worker;
use crate::wtime::{Duration, Instant};
use std::sync::Arc;

pub const DEFAULT_SERVICE: &str = "https://nominatim.openstreetmap.org/search";
/// The policy says one request per second; this leaves a margin.
const MIN_INTERVAL: Duration = Duration::from_millis(1100);
const RESULT_LIMIT: usize = 8;
/// Place names hardly change; a stored answer is used for 90 days, and past that when offline or unreachable.
const MAX_AGE: f64 = 90.0 * 86_400.0;

pub struct Reply {
    pub query: String,
    pub answer: Result<Vec<Place>, Failure>,
}

pub struct Geocoder {
    worker: Worker<String, Reply>,
}

impl Geocoder {
    /// `service` is the search endpoint, so that it can be switched without a new program.
    pub fn new(fetcher: Arc<Fetcher>, service: String, lang: String) -> Self {
        let mut gap = MinGap::new(MIN_INTERVAL);
        // Only the newest query matters if the user was quicker than the server.
        let worker = Worker::new(true, MIN_INTERVAL, move |query: String| {
            let answer = lookup(&fetcher, &service, &lang, &query, &mut gap);
            Reply { query, answer }
        });
        Self { worker }
    }

    pub fn submit(&self, query: &str) {
        self.worker.submit(query.to_string());
    }

    pub fn poll(&self) -> Option<Reply> {
        self.worker.poll()
    }
}

/// Keeps requests at least a given time apart.
pub(crate) struct MinGap {
    gap: Duration,
    last: Option<Instant>,
}

impl MinGap {
    pub(crate) fn new(gap: Duration) -> Self {
        Self { gap, last: None }
    }

    /// How long to wait before a request at `now`.
    pub(crate) fn wait(&self, now: Instant) -> Duration {
        self.last.map_or(Duration::ZERO, |l| {
            (l + self.gap).saturating_duration_since(now)
        })
    }

    pub(crate) fn sent(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

fn lookup(
    fetcher: &Fetcher,
    service: &str,
    lang: &str,
    query: &str,
    gap: &mut MinGap,
) -> Result<Vec<Place>, Failure> {
    let name = cache_name(service, lang, query);
    let stored = fetcher.cache().get(&name);
    let parsed = stored.as_deref().and_then(read_stored);
    if let Some((stamp, places)) = &parsed {
        if fetcher.is_offline() || unix_now() - stamp < MAX_AGE {
            return Ok(places.clone());
        }
    }
    if fetcher.is_offline() {
        return Err(Failure::Offline);
    }
    crate::wtime::pause(gap.wait(Instant::now()));
    gap.sent(Instant::now());
    // An old answer still beats none when the service cannot be reached.
    let stale = |f: Failure| parsed.clone().map(|(_, p)| p).ok_or(f);
    let data = match fetcher.fetch_service(&request_url(service, lang, query)) {
        Ok(data) => data,
        Err(
            FetchError::Missing
            | FetchError::Transient
            | FetchError::Paused(_)
            | FetchError::Busy(_),
        ) => return stale(Failure::Unreachable),
    };
    // Only answers that make sense are kept, so that a bad day is not remembered.
    let places = match parse(&data) {
        Ok(p) => p,
        Err(e) => return stale(e),
    };
    if let Err(e) = fetcher.cache().put(&name, &pack(unix_now(), &data)) {
        eprintln!("cache: could not store a search: {e}");
    }
    Ok(places)
}

/// A stored answer and when it was fetched. Answers stored before they carried a time count as old.
fn read_stored(data: &[u8]) -> Option<(f64, Vec<Place>)> {
    match unpack(data) {
        Some((stamp, body)) => parse(body.as_bytes()).ok().map(|p| (stamp, p)),
        None => parse(data).ok().map(|p| (0.0, p)),
    }
}

fn parse(data: &[u8]) -> Result<Vec<Place>, Failure> {
    search::parse_nominatim(std::str::from_utf8(data).map_err(|_| Failure::Garbled)?)
}

/// The same question in other spellings has one answer: case and spacing do not count.
fn normalize(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn cache_name(service: &str, lang: &str, query: &str) -> String {
    // FNV-1a, which unlike the standard hasher is the same in every version.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in format!("{service}\n{lang}\n{}", normalize(query)).bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("nominatim/{h:016x}.json")
}

fn request_url(service: &str, lang: &str, query: &str) -> String {
    let mut url = format!(
        "{service}?format=jsonv2&limit={RESULT_LIMIT}&q={}",
        percent_encode(query.trim())
    );
    if !lang.is_empty() {
        url.push_str("&accept-language=");
        url.push_str(&percent_encode(lang));
    }
    url
}

pub(crate) fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_encoded() {
        let u = request_url("https://s.example/search", "de", " Zürich & Co ");
        assert_eq!(u, "https://s.example/search?format=jsonv2&limit=8&q=Z%C3%BCrich%20%26%20Co&accept-language=de");
        assert!(!request_url("https://s.example/search", "", "x").contains("accept-language"));
    }

    #[test]
    fn cache_names_ignore_case_and_spacing_only() {
        let s = DEFAULT_SERVICE;
        assert_eq!(
            cache_name(s, "en", "Bad  Ischl"),
            cache_name(s, "en", " bad ischl ")
        );
        assert_ne!(
            cache_name(s, "en", "bad ischl"),
            cache_name(s, "de", "bad ischl")
        );
        assert_ne!(cache_name(s, "en", "bad ischl"), cache_name(s, "en", "bad"));
        assert!(cache_name(s, "en", "x").starts_with("nominatim/"));
    }

    #[test]
    fn requests_are_spaced() {
        let t = Instant::now();
        let mut gap = MinGap::new(Duration::from_millis(1100));
        assert_eq!(gap.wait(t), Duration::ZERO);
        gap.sent(t);
        assert_eq!(gap.wait(t), Duration::from_millis(1100));
        assert_eq!(
            gap.wait(t + Duration::from_millis(400)),
            Duration::from_millis(700)
        );
        assert_eq!(gap.wait(t + Duration::from_secs(2)), Duration::ZERO);
    }

    fn fetcher(tag: &str, offline: bool) -> Arc<Fetcher> {
        let dir = std::env::temp_dir().join(format!("orbis-geocode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Arc::new(Fetcher::new(
            crate::cache::DiskCache::new(dir),
            crate::fetcher::Peers::default(),
            offline,
        ))
    }

    fn wait_reply(g: &Geocoder) -> Reply {
        for _ in 0..500 {
            if let Some(r) = g.poll() {
                return r;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no reply");
    }

    #[test]
    fn offline_answers_from_the_cache_only() {
        let f = fetcher("offline", true);
        let service = "http://127.0.0.1:9/search".to_string();
        let g = Geocoder::new(f.clone(), service.clone(), "en".into());
        g.submit("Nowhere");
        assert_eq!(wait_reply(&g).answer, Err(Failure::Offline));
        let body = r#"[{"lat":"1","lon":"2","display_name":"Somewhere"}]"#;
        f.cache()
            .put(&cache_name(&service, "en", "somewhere"), body.as_bytes())
            .unwrap();
        g.submit("Somewhere");
        let r = wait_reply(&g);
        assert_eq!(r.query, "Somewhere");
        assert_eq!(r.answer.unwrap()[0].name, "Somewhere");
        assert_eq!(f.traffic().downloads, 0);
    }

    #[test]
    fn garbled_cache_entries_are_not_trusted() {
        let f = fetcher("garbled", true);
        let service = "http://127.0.0.1:9/search".to_string();
        f.cache()
            .put(&cache_name(&service, "", "x"), b"<html>")
            .unwrap();
        let g = Geocoder::new(f, service, String::new());
        g.submit("x");
        assert_eq!(wait_reply(&g).answer, Err(Failure::Offline));
    }

    #[test]
    fn stored_answers_expire_but_still_help_when_unreachable() {
        let f = fetcher("expiry", false);
        let service = "http://127.0.0.1:9/search".to_string();
        let body = br#"[{"lat":"1","lon":"2","display_name":"Somewhere"}]"#;
        let fresh = unix_now() - 10.0 * 86_400.0;
        f.cache()
            .put(&cache_name(&service, "en", "fresh"), &pack(fresh, body))
            .unwrap();
        f.cache()
            .put(
                &cache_name(&service, "en", "old"),
                &pack(unix_now() - MAX_AGE - 1.0, body),
            )
            .unwrap();
        assert_eq!(read_stored(&pack(5.0, body)).unwrap().0, 5.0);
        assert_eq!(
            read_stored(body).unwrap().0,
            0.0,
            "answers without a time count as old"
        );
        let g = Geocoder::new(f.clone(), service, "en".into());
        g.submit("fresh");
        assert_eq!(wait_reply(&g).answer.unwrap()[0].name, "Somewhere");
        assert_eq!(f.traffic().downloads, 0);
        // The old one is asked for again; the service is unreachable, so the old answer is used.
        g.submit("old");
        assert_eq!(wait_reply(&g).answer.unwrap()[0].name, "Somewhere");
    }
}
