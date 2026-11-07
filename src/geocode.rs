//! Looks up place names with Nominatim, on a thread of its own so that no frame waits for it.
//!
//! The public server's usage policy (https://operations.osmfoundation.org/policies/nominatim/)
//! allows one request per second at most, forbids autocomplete, wants an identifying user agent,
//! attribution, and caching of repeated queries. So a request goes out only when the user presses
//! Enter, at least `MIN_INTERVAL` after the previous one, and every answer is kept on disk.

use crate::fetcher::{FetchError, Fetcher};
use crate::search::{self, Failure, Place};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_SERVICE: &str = "https://nominatim.openstreetmap.org/search";
/// The policy says one request per second; this leaves a margin.
const MIN_INTERVAL: Duration = Duration::from_millis(1100);
const RESULT_LIMIT: usize = 8;

pub struct Reply {
    pub query: String,
    pub answer: Result<Vec<Place>, Failure>,
}

pub struct Geocoder {
    jobs: Sender<String>,
    replies: Mutex<Receiver<Reply>>,
}

impl Geocoder {
    /// `service` is the search endpoint, so that it can be switched without a new program.
    pub fn new(fetcher: Arc<Fetcher>, service: String, lang: String) -> Self {
        let (jobs, job_rx) = channel::<String>();
        let (reply_tx, replies) = channel();
        std::thread::spawn(move || {
            let mut gap = MinGap::new(MIN_INTERVAL);
            // The thread ends when the Geocoder is dropped and the channel closes.
            while let Ok(mut query) = job_rx.recv() {
                // Only the newest query matters if the user was quicker than the server.
                while let Ok(newer) = job_rx.try_recv() {
                    query = newer;
                }
                let answer = lookup(&fetcher, &service, &lang, &query, &mut gap);
                if reply_tx.send(Reply { query, answer }).is_err() {
                    break;
                }
            }
        });
        Self { jobs, replies: Mutex::new(replies) }
    }

    pub fn submit(&self, query: &str) {
        let _ = self.jobs.send(query.to_string());
    }

    pub fn poll(&self) -> Option<Reply> {
        self.replies.lock().ok()?.try_recv().ok()
    }
}

/// Keeps requests at least a given time apart.
struct MinGap {
    gap: Duration,
    last: Option<Instant>,
}

impl MinGap {
    fn new(gap: Duration) -> Self {
        Self { gap, last: None }
    }

    /// How long to wait before a request at `now`.
    fn wait(&self, now: Instant) -> Duration {
        self.last.map_or(Duration::ZERO, |l| (l + self.gap).saturating_duration_since(now))
    }

    fn sent(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

fn lookup(fetcher: &Fetcher, service: &str, lang: &str, query: &str, gap: &mut MinGap) -> Result<Vec<Place>, Failure> {
    let name = cache_name(service, lang, query);
    if let Some(places) = fetcher.cache().get(&name).and_then(|d| parse(&d).ok()) {
        return Ok(places);
    }
    if fetcher.is_offline() {
        return Err(Failure::Offline);
    }
    std::thread::sleep(gap.wait(Instant::now()));
    gap.sent(Instant::now());
    let data = fetcher.fetch_service(&request_url(service, lang, query)).map_err(|e| match e {
        FetchError::Missing | FetchError::Transient | FetchError::Paused(_) | FetchError::Busy(_) => Failure::Unreachable,
    })?;
    // Only answers that make sense are kept, so that a bad day is not remembered.
    let places = parse(&data)?;
    if let Err(e) = fetcher.cache().put(&name, &data) {
        eprintln!("cache: could not store a search: {e}");
    }
    Ok(places)
}

fn parse(data: &[u8]) -> Result<Vec<Place>, Failure> {
    search::parse_nominatim(std::str::from_utf8(data).map_err(|_| Failure::Garbled)?)
}

/// The same question in other spellings has one answer: case and spacing do not count.
fn normalize(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
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
    let mut url = format!("{service}?format=jsonv2&limit={RESULT_LIMIT}&q={}", percent_encode(query.trim()));
    if !lang.is_empty() {
        url.push_str("&accept-language=");
        url.push_str(&percent_encode(lang));
    }
    url
}

fn percent_encode(text: &str) -> String {
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
        assert_eq!(cache_name(s, "en", "Bad  Ischl"), cache_name(s, "en", " bad ischl "));
        assert_ne!(cache_name(s, "en", "bad ischl"), cache_name(s, "de", "bad ischl"));
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
        assert_eq!(gap.wait(t + Duration::from_millis(400)), Duration::from_millis(700));
        assert_eq!(gap.wait(t + Duration::from_secs(2)), Duration::ZERO);
    }

    fn fetcher(tag: &str, offline: bool) -> Arc<Fetcher> {
        let dir = std::env::temp_dir().join(format!("orbis-geocode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Arc::new(Fetcher::new(crate::cache::DiskCache::new(dir), crate::fetcher::Peers::default(), offline))
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
        f.cache().put(&cache_name(&service, "en", "somewhere"), body.as_bytes()).unwrap();
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
        f.cache().put(&cache_name(&service, "", "x"), b"<html>").unwrap();
        let g = Geocoder::new(f, service, String::new());
        g.submit("x");
        assert_eq!(wait_reply(&g).answer, Err(Failure::Offline));
    }
}
