//! Gets tiles with as little traffic as possible: from the disk cache, from a download already
//! under way, from a LAN cache server, and only then from the public server, if it may be asked.

use crate::cache::DiskCache;
use crate::geo::TileKey;
use crate::servers::{Failure, ServerReport, Servers};
use crate::sources::{self, TileSource, MAP_TILEJSON_URL};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USER_AGENT: &str = "orbis-globe/0.1";
const WEEK_SECONDS: u64 = 7 * 24 * 3600;
/// How long a cache server stays skipped after it could not be reached.
const PEER_DOWN: Duration = Duration::from_secs(60);
/// A discovered server not heard from for this long is forgotten.
const PEER_EXPIRY: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq, Clone)]
pub enum FetchError {
    /// The server has no such tile.
    Missing,
    /// The server may not be asked before this time.
    Paused(SystemTime),
    /// Something else went wrong that may pass.
    Transient,
    /// Nothing went wrong, but getting this tile now would mean waiting.
    Busy(Wait),
}

/// What a tile is waiting for.
#[derive(Debug, PartialEq, Clone)]
pub enum Wait {
    /// A public server already has as many requests from us as it should get at once.
    Server(String),
    /// Someone else is already getting this tile.
    Tile(String),
}

/// Limits how many threads do something at once.
pub struct Gate {
    free: Mutex<usize>,
    released: Condvar,
}

impl Gate {
    pub fn new(n: usize) -> Self {
        Self { free: Mutex::new(n), released: Condvar::new() }
    }

    pub fn enter(&self) -> GatePass<'_> {
        let mut free = self.free.lock().unwrap();
        while *free == 0 {
            free = self.released.wait(free).unwrap();
        }
        *free -= 1;
        GatePass(self)
    }

    /// Enters only if that needs no waiting.
    pub fn try_enter(&self) -> Option<GatePass<'_>> {
        let mut free = self.free.lock().unwrap();
        if *free == 0 {
            return None;
        }
        *free -= 1;
        Some(GatePass(self))
    }

    fn is_full(&self) -> bool {
        *self.free.lock().unwrap() == 0
    }
}

pub struct GatePass<'a>(&'a Gate);

impl Drop for GatePass<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap() += 1;
        self.0.released.notify_one();
    }
}

/// The tile URL template from a TileJSON document.
pub fn tile_template(json: &str) -> Option<String> {
    let after = &json[json.find("\"tiles\"")?..];
    let start = after.find("\"http")? + 1;
    let rest = &after[start..];
    let url = rest[..rest.find('"')?].replace("\\/", "/");
    let usable = url.starts_with("https://") && ["{z}", "{x}", "{y}"].iter().all(|p| url.contains(p));
    usable.then_some(url)
}

/// The host part of an http or https URL.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |p| p.1);
    rest.split(['/', '?']).next().unwrap_or(rest).to_string()
}

/// A download in progress that others can wait for.
#[derive(Default)]
struct Flight {
    result: Mutex<Option<Result<Vec<u8>, FetchError>>>,
    done: Condvar,
}

/// Publishes a result even if the download's thread were to panic, so nobody waits forever.
struct FlightGuard<'a> {
    fetcher: &'a Fetcher,
    name: &'a str,
    flight: Arc<Flight>,
    result: Option<Result<Vec<u8>, FetchError>>,
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        self.fetcher.flights.lock().unwrap().remove(self.name);
        let result = self.result.take().unwrap_or(Err(FetchError::Transient));
        *self.flight.result.lock().unwrap() = Some(result);
        self.flight.done.notify_all();
    }
}

/// Other cache servers on the LAN.
#[derive(Default)]
pub struct Peers {
    fixed: Vec<String>,
    /// Discovered servers: address, when last heard, and whether it downloads for its clients.
    found: Mutex<HashMap<String, (Instant, bool)>>,
    down: Mutex<HashMap<String, Instant>>,
    /// While servers may still answer a discovery request, nothing goes to the public servers.
    hold_until: Mutex<Option<Instant>>,
}

impl Peers {
    pub fn new(fixed: Vec<String>) -> Self {
        Self { fixed, ..Self::default() }
    }

    pub fn hold_downloads_until(&self, until: Instant) {
        *self.hold_until.lock().unwrap() = Some(until);
    }

    fn wait_for_answers(&self) {
        let until = *self.hold_until.lock().unwrap();
        if let Some(wait) = until.and_then(|u| u.checked_duration_since(Instant::now())) {
            std::thread::sleep(wait);
        }
    }

    pub fn heard(&self, address: String, dedicated: bool, now: Instant) {
        self.found.lock().unwrap().insert(address, (now, dedicated));
    }

    /// Whom to ask, dedicated servers first, leaving out those recently unreachable.
    fn list(&self, now: Instant) -> Vec<String> {
        let down = self.down.lock().unwrap();
        let usable = |a: &String| down.get(a).is_none_or(|t| *t <= now);
        let mut found: Vec<(String, bool)> = {
            let mut map = self.found.lock().unwrap();
            map.retain(|_, v| now.duration_since(v.0) < PEER_EXPIRY);
            map.iter().map(|(a, v)| (a.clone(), v.1)).collect()
        };
        found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.fixed
            .iter()
            .cloned()
            .chain(found.into_iter().map(|f| f.0))
            .filter(|a| usable(a))
            .collect()
    }

    fn mark_down(&self, address: &str, now: Instant) {
        self.down.lock().unwrap().insert(address.to_string(), now + PEER_DOWN);
    }

    #[cfg(test)]
    pub fn list_for_test(&self, now: Instant) -> Vec<String> {
        self.list(now)
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Traffic {
    pub disk_hits: u64,
    pub lan_hits: u64,
    pub downloads: u64,
    pub downloaded_bytes: u64,
    /// Requests that waited for a download already under way rather than starting another.
    pub shared_downloads: u64,
}

pub struct Fetcher {
    cache: DiskCache,
    agent: ureq::Agent,
    lan_agent: ureq::Agent,
    pub servers: Servers,
    pub peers: Peers,
    offline: bool,
    map_template: Mutex<Option<String>>,
    flights: Mutex<HashMap<String, Arc<Flight>>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    disk_hits: AtomicU64,
    lan_hits: AtomicU64,
    downloads: AtomicU64,
    downloaded_bytes: AtomicU64,
    shared_downloads: AtomicU64,
}

fn agent(global: Duration, connect: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(global))
        .timeout_connect(Some(connect))
        .user_agent(USER_AGENT)
        // Status codes are decisions for us to make, not errors.
        .http_status_as_error(false)
        .build()
        .into()
}

fn retry_after(response: &ureq::http::Response<ureq::Body>) -> Option<Duration> {
    let value = response.headers().get("retry-after")?.to_str().ok()?;
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

impl Fetcher {
    pub fn new(cache: DiskCache, peers: Peers, offline: bool) -> Self {
        let servers = Servers::new(Some(cache.state_path("servers.txt")));
        Self {
            cache,
            agent: agent(Duration::from_secs(30), Duration::from_secs(10)),
            lan_agent: agent(Duration::from_secs(35), Duration::from_secs(1)),
            servers,
            peers,
            offline,
            map_template: Mutex::new(None),
            flights: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
            disk_hits: AtomicU64::new(0),
            lan_hits: AtomicU64::new(0),
            downloads: AtomicU64::new(0),
            downloaded_bytes: AtomicU64::new(0),
            shared_downloads: AtomicU64::new(0),
        }
    }

    pub fn traffic(&self) -> Traffic {
        Traffic {
            disk_hits: self.disk_hits.load(Relaxed),
            lan_hits: self.lan_hits.load(Relaxed),
            downloads: self.downloads.load(Relaxed),
            downloaded_bytes: self.downloaded_bytes.load(Relaxed),
            shared_downloads: self.shared_downloads.load(Relaxed),
        }
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }

    /// One request to a web service that is not a tile server (the caller keeps its own rate
    /// limit), under the same pauses and back-off as tiles. Nothing is cached here.
    pub fn fetch_service(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        if self.offline {
            return Err(FetchError::Transient);
        }
        self.download(url, 1, true)
    }

    pub fn server_report(&self) -> Vec<ServerReport> {
        self.servers.report(SystemTime::now())
    }

    pub fn cache(&self) -> &DiskCache {
        &self.cache
    }

    #[cfg(test)]
    pub fn peers_for_test(&self) -> Vec<String> {
        self.peers.list(Instant::now())
    }

    /// What the disk cache knows about a tile: its bytes, `Missing`, or `None` if nothing.
    pub fn cached(&self, source: &TileSource, k: TileKey) -> Option<Result<Vec<u8>, FetchError>> {
        let name = source.cache_name(k);
        if let Some(data) = self.cache.get(&name) {
            self.disk_hits.fetch_add(1, Relaxed);
            return Some(Ok(data));
        }
        self.cache.is_known_missing(&name).then_some(Err(FetchError::Missing))
    }

    /// Whether a coarser image of this area already turned out empty. A finer one holds nothing
    /// either, so asking the service for each of them would only cost it requests.
    pub fn empty_above(&self, source: &TileSource, k: TileKey) -> bool {
        (source.min_z.saturating_sub(1)..k.z).any(|z| self.cache.is_known_missing(&source.cache_name(k.ancestor(z))))
    }

    /// Gets a tile by the cheapest route available, waiting for its turn if need be.
    pub fn fetch(&self, source: &'static TileSource, k: TileKey) -> Result<Vec<u8>, FetchError> {
        loop {
            // Whoever this was waiting for could only say the server is busy, so ask for itself.
            match self.fetch_with(source, k, true) {
                Err(FetchError::Busy(_)) => {}
                result => return result,
            }
        }
    }

    /// Like `fetch`, but says what it would have to wait for instead of waiting. This lets a
    /// thread do other work meanwhile, so a slow server holds up only its own tiles.
    pub fn try_fetch(&self, source: &'static TileSource, k: TileKey) -> Result<Vec<u8>, FetchError> {
        self.fetch_with(source, k, false)
    }

    /// Whether what a tile was waiting for has passed, so it is worth trying again.
    pub fn is_clear(&self, wait: &Wait) -> bool {
        match wait {
            Wait::Server(host) => self.gates.lock().unwrap().get(host).is_none_or(|g| !g.is_full()),
            Wait::Tile(name) => !self.flights.lock().unwrap().contains_key(name),
        }
    }

    fn fetch_with(&self, source: &'static TileSource, k: TileKey, may_wait: bool) -> Result<Vec<u8>, FetchError> {
        if let Some(known) = self.cached(source, k) {
            return known;
        }
        let name = source.cache_name(k);
        let (flight, leader) = {
            let mut flights = self.flights.lock().unwrap();
            match flights.get(&name) {
                Some(_) if !may_wait => return Err(FetchError::Busy(Wait::Tile(name))),
                Some(f) => (f.clone(), false),
                None => {
                    let f = Arc::new(Flight::default());
                    flights.insert(name.clone(), f.clone());
                    (f, true)
                }
            }
        };
        if !leader {
            self.shared_downloads.fetch_add(1, Relaxed);
            let mut slot = flight.result.lock().unwrap();
            loop {
                if let Some(result) = &*slot {
                    return result.clone();
                }
                slot = flight.done.wait(slot).unwrap();
            }
        }
        let mut guard = FlightGuard { fetcher: self, name: &name, flight, result: None };
        let result = self.fetch_remote(source, k, &name, may_wait);
        guard.result = Some(result.clone());
        result
    }

    /// Forgets a stored tile that turned out to be damaged.
    pub fn discard(&self, source: &TileSource, k: TileKey) {
        self.cache.remove(&source.cache_name(k));
    }

    /// Records that a tile holds no data, such as a blank image outside a service's coverage.
    pub fn mark_missing(&self, source: &TileSource, k: TileKey) {
        let name = source.cache_name(k);
        self.cache.remove(&name);
        self.cache.mark_missing(&name);
    }

    fn fetch_remote(&self, source: &'static TileSource, k: TileKey, name: &str, may_wait: bool) -> Result<Vec<u8>, FetchError> {
        if self.offline {
            return Err(FetchError::Transient);
        }
        // Cache servers know only the built-in sources, and must not learn what a user views.
        let asked = if crate::service_lib::is_service(source) { None } else { self.ask_peers(source, k) };
        match asked {
            Some(Ok(data)) => {
                self.store(name, &data);
                self.lan_hits.fetch_add(1, Relaxed);
                return Ok(data);
            }
            Some(Err(FetchError::Missing)) => {
                self.cache.mark_missing(name);
                return Err(FetchError::Missing);
            }
            Some(Err(e)) => return Err(e),
            None => {}
        }
        let template = if source.url.is_empty() {
            self.map_url_template()?
        } else {
            source.url.to_string()
        };
        let url = TileSource::url_from(&template, k);
        match self.download(&url, source.concurrency, may_wait) {
            Ok(data) => {
                self.store(name, &data);
                Ok(data)
            }
            Err(FetchError::Missing) => {
                self.cache.mark_missing(name);
                Err(FetchError::Missing)
            }
            Err(e) => Err(e),
        }
    }

    fn store(&self, name: &str, data: &[u8]) {
        if let Err(e) = self.cache.put(name, data) {
            eprintln!("cache: could not store {name}: {e}");
        }
    }

    /// `None` means no cache server has an answer, so the public server must be asked.
    fn ask_peers(&self, source: &TileSource, k: TileKey) -> Option<Result<Vec<u8>, FetchError>> {
        self.peers.wait_for_answers();
        let now = Instant::now();
        for peer in self.peers.list(now) {
            let url = format!("http://{peer}/tiles/{}/{}/{}/{}", source.id, k.z, k.x, k.y);
            let mut response = match self.lan_agent.get(&url).call() {
                Ok(r) => r,
                Err(_) => {
                    eprintln!("cache server {peer}: unreachable, skipping it for a minute");
                    self.peers.mark_down(&peer, now);
                    continue;
                }
            };
            match response.status().as_u16() {
                200 => match response.body_mut().read_to_vec() {
                    Ok(data) => return Some(Ok(data)),
                    Err(_) => self.peers.mark_down(&peer, now),
                },
                404 => return Some(Err(FetchError::Missing)),
                // A dedicated server tracks the public server for the whole LAN.
                503 => {
                    let wait = retry_after(&response).unwrap_or(Duration::from_secs(5));
                    return Some(Err(FetchError::Paused(SystemTime::now() + wait)));
                }
                // This one simply does not have it.
                _ => {}
            }
        }
        None
    }

    pub(crate) fn gate(&self, host: &str, concurrency: usize) -> Arc<Gate> {
        let mut gates = self.gates.lock().unwrap();
        gates.entry(host.to_string()).or_insert_with(|| Arc::new(Gate::new(concurrency))).clone()
    }

    /// One request to a public server, under the rules for that server.
    fn download(&self, url: &str, concurrency: usize, may_wait: bool) -> Result<Vec<u8>, FetchError> {
        let host = host_of(url);
        let gate = self.gate(&host, concurrency);
        let _pass = if may_wait {
            gate.enter()
        } else {
            gate.try_enter().ok_or_else(|| FetchError::Busy(Wait::Server(host.clone())))?
        };
        // Asked only now: the server may have been timed out while this request waited.
        self.servers.check(&host, SystemTime::now()).map_err(FetchError::Paused)?;
        let fail = |failure: Failure| match self.servers.failed(&host, failure, SystemTime::now()) {
            Some(until) => FetchError::Paused(until),
            None => FetchError::Transient,
        };
        let mut response = match self.agent.get(url).call() {
            Ok(r) => r,
            Err(ureq::Error::Timeout(_) | ureq::Error::BodyStalled) => {
                eprintln!("{host}: request timed out");
                return Err(fail(Failure::ServerError { retry_after: None }));
            }
            Err(e) => {
                eprintln!("{host}: {e}");
                return Err(fail(Failure::Unreachable));
            }
        };
        let status = response.status().as_u16();
        match status {
            200..=299 => match response.body_mut().read_to_vec() {
                Ok(data) => {
                    self.servers.succeeded(&host, data.len() as u64);
                    self.downloads.fetch_add(1, Relaxed);
                    self.downloaded_bytes.fetch_add(data.len() as u64, Relaxed);
                    Ok(data)
                }
                Err(ureq::Error::Timeout(_) | ureq::Error::BodyStalled) => {
                    Err(fail(Failure::ServerError { retry_after: None }))
                }
                Err(_) => Err(fail(Failure::Unreachable)),
            },
            404 | 410 => {
                self.servers.succeeded(&host, 0);
                Err(FetchError::Missing)
            }
            429 => {
                eprintln!("{host}: told to slow down (HTTP 429)");
                Err(fail(Failure::TooManyRequests { retry_after: retry_after(&response) }))
            }
            500..=599 | 403 => {
                eprintln!("{host}: HTTP {status}");
                Err(fail(Failure::ServerError { retry_after: retry_after(&response) }))
            }
            _ => {
                eprintln!("{host}: unexpected HTTP {status}");
                self.servers.succeeded(&host, 0);
                Err(FetchError::Transient)
            }
        }
    }

    /// The map service publishes data under a versioned URL. It is looked up at most weekly;
    /// if that fails, an earlier answer still gets tiles while they remain online.
    fn map_url_template(&self) -> Result<String, FetchError> {
        let mut known = self.map_template.lock().unwrap();
        if let Some(t) = &*known {
            return Ok(t.clone());
        }
        let name = |week: u64| format!("openfreemap/tilejson-{week}.json");
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let week = seconds / WEEK_SECONDS;
        let parse = |bytes: &[u8]| tile_template(&String::from_utf8_lossy(bytes));
        let mut trouble = FetchError::Transient;
        let fresh = match self.cache.get(&name(week)) {
            Some(bytes) => parse(&bytes),
            None => match self.download(MAP_TILEJSON_URL, 1, true) {
                Ok(bytes) => {
                    self.store(&name(week), &bytes);
                    parse(&bytes)
                }
                Err(e) => {
                    trouble = e;
                    None
                }
            },
        };
        let template = fresh
            .or_else(|| {
                (1..=8).find_map(|back| parse(&self.cache.get(&name(week.saturating_sub(back)))?))
            })
            .ok_or(trouble)?;
        *known = Some(template.clone());
        Ok(template)
    }

    /// The server's state as JSON, for the cache server's status page.
    pub fn status_json(&self) -> String {
        let now = SystemTime::now();
        let t = self.traffic();
        let (entries, bytes) = self.cache.usage();
        let mut servers = Vec::new();
        for r in self.server_report() {
            let paused = r
                .paused_until
                .and_then(|u| u.duration_since(now).ok())
                .map_or(0, |d| d.as_secs());
            servers.push(format!(
                "{{\"host\":\"{}\",\"requests\":{},\"bytes\":{},\"errors\":{},\"paused_seconds\":{}}}",
                r.host.replace(['"', '\\'], ""), r.requests, r.bytes, r.errors, paused,
            ));
        }
        let sources: Vec<String> = sources::all().map(|s| format!("\"{}\"", s.id)).collect();
        format!(
            "{{\"cache\":{{\"entries\":{entries},\"bytes\":{bytes}}},\"traffic\":{{\"disk_hits\":{},\"lan_hits\":{},\"downloads\":{},\"downloaded_bytes\":{},\"shared_downloads\":{}}},\"servers\":[{}],\"sources\":[{}]}}\n",
            t.disk_hits,
            t.lan_hits,
            t.downloads,
            t.downloaded_bytes,
            t.shared_downloads,
            servers.join(","),
            sources.join(","),
        )
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::sources::Kind;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    /// A real HTTP server for tests, which counts requests and answers as told: a list of
    /// status codes with extra headers, of which the last one repeats.
    pub struct TestServer {
        pub port: u16,
        pub requests: Arc<AtomicUsize>,
    }

    impl TestServer {
        pub fn start(script: Vec<(u16, &'static str)>, delay: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let requests = Arc::new(AtomicUsize::new(0));
            let script = Arc::new(Mutex::new(script));
            let (counter, plan) = (requests.clone(), script);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (counter, plan) = (counter.clone(), plan.clone());
                    std::thread::spawn(move || {
                        let mut stream = stream;
                        let mut buffer = [0u8; 2048];
                        if stream.read(&mut buffer).unwrap_or(0) == 0 {
                            return;
                        }
                        counter.fetch_add(1, Relaxed);
                        let (status, headers) = {
                            let mut plan = plan.lock().unwrap();
                            if plan.len() > 1 { plan.remove(0) } else { plan[0] }
                        };
                        std::thread::sleep(delay);
                        let body = b"tile-bytes";
                        let head = format!(
                            "HTTP/1.1 {status} X\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body);
                    });
                }
            });
            Self { port, requests }
        }

        pub fn count(&self) -> usize {
            self.requests.load(Relaxed)
        }

        /// A source that downloads from this server.
        pub fn source(&self) -> &'static TileSource {
            let url = format!("http://127.0.0.1:{}/{{z}}/{{x}}/{{y}}", self.port);
            let cache = format!("test-{}/{{z}}/{{x}}/{{y}}.bin", self.port);
            Box::leak(Box::new(TileSource {
                id: "test",
                kind: Kind::Imagery,
                url: Box::leak(url.into_boxed_str()),
                cache: Box::leak(cache.into_boxed_str()),
                content_type: "application/octet-stream",
                min_z: 0,
                max_z: 19,
                bounds: &[],
                credit: "",
                concurrency: 4,
                pixels: 256,
                grid: crate::geo::Grid::Mercator,
            }))
        }
    }

    pub fn temp_cache(tag: &str) -> DiskCache {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Relaxed);
        let root = std::env::temp_dir().join(format!("orbis-fetch-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        DiskCache::new(root)
    }

    fn fetcher(tag: &str) -> Fetcher {
        Fetcher::new(temp_cache(tag), Peers::default(), false)
    }

    const K: TileKey = TileKey::new(3, 2, 1);

    #[test]
    fn a_tile_is_downloaded_only_once() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let f = fetcher("once");
        let source = server.source();
        assert_eq!(f.fetch(source, K).unwrap(), b"tile-bytes");
        assert_eq!(f.fetch(source, K).unwrap(), b"tile-bytes");
        assert_eq!(server.count(), 1);
        let t = f.traffic();
        assert_eq!((t.downloads, t.disk_hits, t.downloaded_bytes), (1, 1, 10));
    }

    #[test]
    fn a_later_session_uses_the_same_cache() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let cache = temp_cache("restart");
        let source = server.source();
        Fetcher::new(cache.clone(), Peers::default(), false).fetch(source, K).unwrap();
        Fetcher::new(cache, Peers::default(), false).fetch(source, K).unwrap();
        assert_eq!(server.count(), 1);
    }

    #[test]
    fn simultaneous_requests_become_one_download() {
        let server = TestServer::start(vec![(200, "")], Duration::from_millis(150));
        let f = Arc::new(fetcher("flight"));
        let source = server.source();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let f = f.clone();
                std::thread::spawn(move || f.fetch(source, K).unwrap())
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), b"tile-bytes");
        }
        assert_eq!(server.count(), 1);
        assert!(f.traffic().shared_downloads > 0);
    }

    #[test]
    fn what_a_server_does_not_have_is_remembered() {
        let server = TestServer::start(vec![(404, "")], Duration::ZERO);
        let cache = temp_cache("missing");
        let source = server.source();
        let f = Fetcher::new(cache.clone(), Peers::default(), false);
        assert_eq!(f.fetch(source, K), Err(FetchError::Missing));
        assert_eq!(f.fetch(source, K), Err(FetchError::Missing));
        // Also in a later session.
        let f = Fetcher::new(cache, Peers::default(), false);
        assert_eq!(f.fetch(source, K), Err(FetchError::Missing));
        assert_eq!(server.count(), 1);
    }

    #[test]
    fn server_errors_stop_all_requests_to_that_server() {
        let server = TestServer::start(vec![(500, "")], Duration::ZERO);
        let other = TestServer::start(vec![(200, "")], Duration::ZERO);
        let f = fetcher("pause");
        let source = server.source();
        // Each tile is different, as when browsing.
        let tile = |x| TileKey::new(10, x, 5);
        assert_eq!(f.fetch(source, tile(1)), Err(FetchError::Transient));
        assert_eq!(f.fetch(source, tile(2)), Err(FetchError::Transient));
        let third = f.fetch(source, tile(3));
        let Err(FetchError::Paused(until)) = third else { panic!("{third:?}") };
        let wait = until.duration_since(SystemTime::now()).unwrap().as_secs();
        assert!((890..=900).contains(&wait), "paused for {wait} s");
        assert_eq!(server.count(), 3);
        for x in 4..20 {
            assert!(matches!(f.fetch(source, tile(x)), Err(FetchError::Paused(_))));
        }
        assert_eq!(server.count(), 3, "nothing more reached the server");
        // Other servers carry on.
        assert!(f.fetch(other.source(), tile(1)).is_ok());
    }

    #[test]
    fn a_timeout_survives_a_restart_of_the_app() {
        let server = TestServer::start(vec![(429, "Retry-After: 3600\r\n")], Duration::ZERO);
        let cache = temp_cache("persist");
        let source = server.source();
        let f = Fetcher::new(cache.clone(), Peers::default(), false);
        assert!(matches!(f.fetch(source, K), Err(FetchError::Paused(_))));
        let f = Fetcher::new(cache, Peers::default(), false);
        let again = f.fetch(source, TileKey::new(3, 1, 1));
        let Err(FetchError::Paused(until)) = again else { panic!("{again:?}") };
        let wait = until.duration_since(SystemTime::now()).unwrap().as_secs();
        assert!(wait > 3500, "Retry-After is respected: {wait} s");
        assert_eq!(server.count(), 1);
    }

    #[test]
    fn a_failure_is_not_cached_and_the_tile_comes_later() {
        let server = TestServer::start(vec![(500, ""), (200, "")], Duration::ZERO);
        let f = fetcher("recover");
        let source = server.source();
        assert_eq!(f.fetch(source, K), Err(FetchError::Transient));
        assert_eq!(f.fetch(source, K).unwrap(), b"tile-bytes");
    }

    #[test]
    fn an_unreachable_server_gets_only_a_short_backoff() {
        let f = fetcher("unreachable");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = format!("http://127.0.0.1:{port}/{{z}}/{{x}}/{{y}}");
        let source: &'static TileSource = Box::leak(Box::new(TileSource {
            url: Box::leak(url.into_boxed_str()),
            cache: "test-unreachable/{z}/{x}/{y}.bin",
            ..clone_source(&sources::TERRAIN)
        }));
        let result = f.fetch(source, K);
        let Err(FetchError::Paused(until)) = result else { panic!("{result:?}") };
        assert!(until.duration_since(SystemTime::now()).unwrap() <= Duration::from_secs(2));
    }

    fn clone_source(s: &TileSource) -> TileSource {
        TileSource { kind: s.kind, content_type: s.content_type, credit: s.credit, bounds: s.bounds, ..*s }
    }

    #[test]
    fn nothing_is_downloaded_while_cache_servers_may_still_answer() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let f = fetcher("grace");
        f.peers.hold_downloads_until(Instant::now() + Duration::from_millis(150));
        let started = Instant::now();
        f.fetch(server.source(), K).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150));
    }

    #[test]
    fn requests_per_server_stay_within_its_limit() {
        let server = TestServer::start(vec![(200, "")], Duration::from_millis(100));
        let f = Arc::new(fetcher("limit"));
        let source = server.source();
        let started = Instant::now();
        let threads: Vec<_> = (0..8u32)
            .map(|x| {
                let f = f.clone();
                std::thread::spawn(move || f.fetch(source, TileKey::new(10, x, 7)).unwrap())
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(started.elapsed() >= Duration::from_millis(200), "8 requests at 4 at a time");
        assert_eq!(server.count(), 8);
    }

    #[test]
    fn offline_mode_sends_nothing() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let f = Fetcher::new(temp_cache("offline"), Peers::default(), true);
        assert_eq!(f.fetch(server.source(), K), Err(FetchError::Transient));
        assert_eq!(server.count(), 0);
    }

    #[test]
    fn a_damaged_tile_can_be_discarded_and_fetched_again() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let f = fetcher("discard");
        let source = server.source();
        f.fetch(source, K).unwrap();
        f.discard(source, K);
        f.fetch(source, K).unwrap();
        assert_eq!(server.count(), 2);
    }

    #[test]
    fn hosts_and_templates_parse() {
        assert_eq!(host_of("https://a.example.org/x/y?z=1"), "a.example.org");
        assert_eq!(host_of("http://127.0.0.1:80/x"), "127.0.0.1:80");
        assert_eq!(host_of("https://a.example.org?x=1"), "a.example.org");
        let json = r#"{"tilejson":"3.0.0","tiles":["https://t.example/v1/{z}/{x}/{y}.pbf"],"x":"http://no"}"#;
        assert_eq!(tile_template(json).unwrap(), "https://t.example/v1/{z}/{x}/{y}.pbf");
        let escaped = r#"{"tiles":["https:\/\/t.example\/{z}\/{x}\/{y}.pbf"]}"#;
        assert_eq!(tile_template(escaped).unwrap(), "https://t.example/{z}/{x}/{y}.pbf");
        assert_eq!(tile_template(r#"{"tiles":["http://t.example/{z}/{x}/{y}"]}"#), None);
        assert_eq!(tile_template(r#"{"tiles":["https://t.example/fixed.pbf"]}"#), None);
        assert_eq!(tile_template("not json"), None);
    }

    #[test]
    fn gate_limits_how_many_enter() {
        let gate = Arc::new(Gate::new(2));
        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (gate, inside, most) = (gate.clone(), inside.clone(), most.clone());
                std::thread::spawn(move || {
                    let _pass = gate.enter();
                    let now = inside.fetch_add(1, Relaxed) + 1;
                    most.fetch_max(now, Relaxed);
                    std::thread::sleep(Duration::from_millis(10));
                    inside.fetch_sub(1, Relaxed);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(most.load(Relaxed), 2);
    }

    #[test]
    fn status_reports_traffic_as_json() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let f = fetcher("status");
        f.fetch(server.source(), K).unwrap();
        let json = f.status_json();
        assert!(json.contains("\"downloads\":1"), "{json}");
        assert!(json.contains("127.0.0.1"), "{json}");
        assert!(json.contains("\"eox\""), "{json}");
    }

    #[test]
    fn a_full_server_is_reported_instead_of_waited_for() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let source = server.source();
        let f = fetcher("busy-server");
        let host = host_of(source.url);
        let gate = f.gate(&host, 1);
        let pass = gate.enter();

        let wait = Wait::Server(host);
        assert_eq!(f.try_fetch(source, K), Err(FetchError::Busy(wait.clone())));
        assert!(!f.is_clear(&wait));
        assert!(f.is_clear(&Wait::Server("other.example".into())));

        drop(pass);
        assert!(f.is_clear(&wait));
        assert_eq!(f.try_fetch(source, K).unwrap(), b"tile-bytes");
    }

    #[test]
    fn a_tile_being_fetched_is_reported_instead_of_waited_for() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let source = server.source();
        let f = fetcher("busy-tile");
        let name = source.cache_name(K);
        f.flights.lock().unwrap().insert(name.clone(), Arc::default());

        let wait = Wait::Tile(name.clone());
        assert_eq!(f.try_fetch(source, K), Err(FetchError::Busy(wait.clone())));
        assert!(!f.is_clear(&wait));
        f.flights.lock().unwrap().remove(&name);
        assert!(f.is_clear(&wait));
    }

    #[test]
    fn someone_waiting_gets_the_tile_even_if_the_one_ahead_found_the_server_busy() {
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let source = server.source();
        let f = Arc::new(fetcher("busy-follower"));
        let name = source.cache_name(K);
        let flight: Arc<Flight> = Arc::default();
        f.flights.lock().unwrap().insert(name.clone(), flight.clone());

        let waiting = {
            let f = f.clone();
            std::thread::spawn(move || f.fetch(source, K))
        };
        std::thread::sleep(Duration::from_millis(50));
        f.flights.lock().unwrap().remove(&name);
        *flight.result.lock().unwrap() = Some(Err(FetchError::Busy(Wait::Server("x".into()))));
        flight.done.notify_all();
        assert_eq!(waiting.join().unwrap().unwrap(), b"tile-bytes");
    }

    #[test]
    fn a_gate_can_be_tried_without_waiting() {
        let gate = Gate::new(1);
        let pass = gate.try_enter();
        assert!(pass.is_some() && gate.is_full());
        assert!(gate.try_enter().is_none());
        drop(pass);
        assert!(gate.try_enter().is_some());
    }
}
