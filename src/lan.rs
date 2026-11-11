//! The LAN cache server and how machines find each other. Plain HTTP on purpose: other programs
//! can use the standard tile URLs. It answers private networks only, reads no paths from
//! requests and can reach only the known sources, so it is not an open proxy or file server.

use crate::fetcher::{FetchError, Fetcher};
use crate::geo::TileKey;
use crate::sources::{self, TileSource};
use crate::wtime::{Duration, Instant, SystemTime};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};

pub const TCP_PORT: u16 = 47813;
pub const DISCOVERY_PORT: u16 = 47814;
const ANNOUNCE_EVERY: Duration = Duration::from_secs(5);
const QUERY: &str = "orbis-cache/1 query";
/// How long a starting app waits for cache servers to answer before downloading anything.
const DISCOVERY_GRACE: Duration = Duration::from_millis(1500);
const THREADS: usize = 16;
const MAX_REQUEST_HEAD: usize = 8192;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Downloads what clients ask for, under the server rules, for the whole LAN.
    Dedicated,
    /// Offers only what this machine has already.
    SharedApp,
}

/// Whether an address belongs to a home or office network, or this machine.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

#[derive(Debug, PartialEq)]
enum Route {
    Tile(&'static TileSource, TileKey),
    Status,
    NotFound,
}

/// Sources that tests serve from a local server, so that requests can go through the real
/// route. Only test builds have it; a release build reaches the known sources alone.
#[cfg(test)]
pub(crate) static TEST_SOURCES: Mutex<Vec<&'static TileSource>> = Mutex::new(Vec::new());

fn source_by_id(id: &str) -> Option<&'static TileSource> {
    #[cfg(test)]
    if let Some(s) = TEST_SOURCES.lock().unwrap().iter().find(|s| s.id == id) {
        return Some(*s);
    }
    sources::by_id(id)
}

/// Parses a path into a known source and numbers. Nothing from it ever reaches a file path.
fn route(path: &str) -> Route {
    let path = path.split('?').next().unwrap_or(path);
    if path == "/status" {
        return Route::Status;
    }
    let parts: Vec<&str> = path.split('/').collect();
    let ["", "tiles", id, z, x, y] = parts[..] else {
        return Route::NotFound;
    };
    let Some(source) = source_by_id(id) else {
        return Route::NotFound;
    };
    let number = |s: &str| -> Option<u32> {
        // Plain digits only: no signs, and parse() alone would accept a leading plus.
        (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (Some(z), Some(x), Some(y)) = (number(z), number(x), number(y)) else {
        return Route::NotFound;
    };
    if z < source.min_z as u32
        || z > source.max_z as u32
        || x as u64 >= source.grid.columns(z as u8)
        || y as u64 >= source.grid.rows(z as u8)
    {
        return Route::NotFound;
    }
    Route::Tile(source, TileKey::new(z as u8, x, y))
}

fn respond(stream: &mut TcpStream, status: &str, headers: &str, body: &[u8], head_only: bool) {
    let head = format!(
        "HTTP/1.1 {status}\r\nServer: orbis-cache\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).is_ok() && !head_only {
        let _ = stream.write_all(body);
    }
}

fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut data = Vec::with_capacity(512);
    let mut chunk = [0u8; 1024];
    while !data.windows(4).any(|w| w == b"\r\n\r\n") {
        if data.len() >= MAX_REQUEST_HEAD {
            return None;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(n) => data.extend_from_slice(&chunk[..n]),
        }
    }
    String::from_utf8(data).ok()
}

fn handle(mut stream: TcpStream, peer: IpAddr, fetcher: &Fetcher, mode: Mode) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    if !is_private(peer) {
        respond(
            &mut stream,
            "403 Forbidden",
            "",
            b"private networks only\n",
            false,
        );
        return;
    }
    let Some(head) = read_head(&mut stream) else {
        return;
    };
    let mut request = head.lines().next().unwrap_or("").split(' ');
    let (method, path) = (request.next().unwrap_or(""), request.next().unwrap_or(""));
    let head_only = method == "HEAD";
    if method != "GET" && !head_only {
        respond(
            &mut stream,
            "405 Method Not Allowed",
            "Allow: GET, HEAD\r\n",
            b"",
            false,
        );
        return;
    }
    match route(path) {
        Route::NotFound => respond(&mut stream, "404 Not Found", "", b"", head_only),
        Route::Status => {
            let json = fetcher.status_json();
            let headers = "Content-Type: application/json\r\n";
            respond(&mut stream, "200 OK", headers, json.as_bytes(), head_only);
        }
        Route::Tile(source, key) => {
            let result = match mode {
                Mode::Dedicated => Some(fetcher.fetch(source, key)),
                Mode::SharedApp => fetcher.cached(source, key),
            };
            match result {
                Some(Ok(data)) => {
                    let headers = format!(
                        "Content-Type: {}\r\nCache-Control: public, max-age=86400\r\n",
                        source.content_type
                    );
                    respond(&mut stream, "200 OK", &headers, &data, head_only);
                }
                Some(Err(FetchError::Missing)) => {
                    respond(&mut stream, "404 Not Found", "", b"", head_only);
                }
                Some(Err(FetchError::Paused(until))) => {
                    let wait = until
                        .duration_since(SystemTime::now())
                        .map_or(1, |d| d.as_secs().max(1));
                    let headers = format!("Retry-After: {wait}\r\n");
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        &headers,
                        b"",
                        head_only,
                    );
                }
                Some(Err(FetchError::Transient | FetchError::Busy(_))) => {
                    respond(
                        &mut stream,
                        "503 Service Unavailable",
                        "Retry-After: 5\r\n",
                        b"",
                        head_only,
                    );
                }
                None => respond(&mut stream, "504 Gateway Timeout", "", b"", head_only),
            }
        }
    }
}

/// Serves on an open listener with a fixed number of threads, so that a flood of connections
/// waits instead of using up the machine.
pub fn serve(listener: TcpListener, fetcher: Arc<Fetcher>, mode: Mode) {
    let (tx, rx) = sync_channel::<(TcpStream, IpAddr)>(THREADS * 4);
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..THREADS {
        let (rx, fetcher) = (rx.clone(), fetcher.clone());
        std::thread::spawn(move || loop {
            let next = rx.lock().unwrap().recv();
            let Ok((stream, peer)) = next else { return };
            handle(stream, peer, &fetcher, mode);
        });
    }
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let Ok(peer) = stream.peer_addr() else {
                        continue;
                    };
                    if tx.send((stream, peer.ip())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => {
                    eprintln!("cache server: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
}

fn beacon_text(id: u64, port: u16, mode: Mode) -> String {
    let role = if mode == Mode::Dedicated {
        "dedicated"
    } else {
        "shared"
    };
    format!("orbis-cache/1 {id} {port} {role}")
}

/// Reads a discovery message into (instance, port, whether it is a dedicated server).
fn parse_beacon(text: &str) -> Option<(u64, u16, bool)> {
    let mut parts = text.split(' ');
    if parts.next()? != "orbis-cache/1" {
        return None;
    }
    let id = parts.next()?.parse().ok()?;
    let port: u16 = parts.next()?.parse().ok()?;
    let dedicated = match parts.next()? {
        "dedicated" => true,
        "shared" => false,
        _ => return None,
    };
    (parts.next().is_none() && port != 0).then_some((id, port, dedicated))
}

/// A number that tells this running program apart from others.
pub fn instance_id() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos() as u64);
    (std::process::id() as u64) << 32 | nanos
}

#[cfg(target_arch = "wasm32")]
fn discovery_socket() -> std::io::Result<UdpSocket> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(not(target_arch = "wasm32"))]
fn discovery_socket() -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Several programs on one machine can all listen for announcements.
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT));
    socket.bind(&address.into())?;
    Ok(socket.into())
}

/// Announces this server on the LAN every few seconds.
pub fn announce(id: u64, port: u16, mode: Mode) {
    std::thread::spawn(move || {
        let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
            return;
        };
        if socket.set_broadcast(true).is_err() {
            return;
        }
        let text = beacon_text(id, port, mode);
        let target = (Ipv4Addr::BROADCAST, DISCOVERY_PORT);
        loop {
            // Failing here just means no network at the moment.
            let _ = socket.send_to(text.as_bytes(), target);
            std::thread::sleep(ANNOUNCE_EVERY);
        }
    });
}

/// Takes part in discovery: `serving` answers those looking for a server, and `looking` asks
/// at once who is there and keeps its peers up to date from what it hears.
pub fn discover(id: u64, serving: Option<(u16, Mode)>, looking: Option<Arc<Fetcher>>) {
    let socket = match discovery_socket() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cache server discovery unavailable: {e}");
            return;
        }
    };
    if let Some(fetcher) = &looking {
        // Answers take a moment, and downloading meanwhile would waste the point of sharing.
        fetcher
            .peers
            .hold_downloads_until(Instant::now() + DISCOVERY_GRACE);
        let _ = socket.send_to(QUERY.as_bytes(), (Ipv4Addr::BROADCAST, DISCOVERY_PORT));
    }
    std::thread::spawn(move || {
        let mut buffer = [0u8; 128];
        loop {
            let Ok((n, from)) = socket.recv_from(&mut buffer) else {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            };
            if !is_private(from.ip()) {
                continue;
            }
            let Some(text) = std::str::from_utf8(&buffer[..n]).ok() else {
                continue;
            };
            if text == QUERY {
                if let Some((port, mode)) = serving {
                    let _ = socket.send_to(beacon_text(id, port, mode).as_bytes(), from);
                }
                continue;
            }
            let Some((sender, port, dedicated)) = parse_beacon(text) else {
                continue;
            };
            if let Some(fetcher) = looking.as_ref().filter(|_| sender != id) {
                fetcher
                    .peers
                    .heard(format!("{}:{port}", from.ip()), dedicated, Instant::now());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::tests::{temp_cache, TestServer};
    use crate::fetcher::Peers;

    const K: TileKey = TileKey::new(3, 2, 1);

    fn start(mode: Mode, peers: Peers, tag: &str) -> (Arc<Fetcher>, u16) {
        let fetcher = Arc::new(Fetcher::new(temp_cache(tag), peers, false));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        serve(listener, fetcher.clone(), mode);
        (fetcher, port)
    }

    /// Sends a raw request and returns the status line and the whole response.
    fn raw(port: u16, request: &str) -> (String, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        (out.lines().next().unwrap_or("").to_string(), out)
    }

    fn get(port: u16, path: &str) -> (String, String) {
        raw(port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"))
    }

    #[test]
    fn paths_become_known_sources_and_numbers_only() {
        let Route::Tile(s, k) = route("/tiles/eox/3/2/1") else {
            panic!()
        };
        assert_eq!((s.id, k), ("eox", TileKey::new(3, 2, 1)));
        assert_eq!(route("/status"), Route::Status);
        assert!(matches!(route("/tiles/eox/3/2/1?x=y"), Route::Tile(..)));
        for bad in [
            "/",
            "/tiles",
            "/tiles/eox/3/2",
            "/tiles/eox/3/2/1/extra",
            "/tiles/../../etc/passwd/1/1",
            "/tiles/eox/../../1/1",
            "/tiles/nope/1/0/0",
            "/tiles/eox/-1/0/0",
            "/tiles/eox/+3/2/1",
            "/tiles/eox/3/8/0",
            "/tiles/eox/3/0/8",
            "/tiles/eox/15/0/0",
            "/tiles/eox/99999999999/0/0",
            "/tiles/eox/3/2/1.jpg",
            "/tiles/basemap-at/3/2/1",
        ] {
            assert_eq!(route(bad), Route::NotFound, "{bad}");
        }
    }

    #[test]
    fn only_private_networks_are_answered() {
        for ip in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.5",
            "127.0.0.1",
            "169.254.1.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.0.1",
        ] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn a_dedicated_server_makes_one_download_for_the_whole_lan() {
        // The public server is a local test server, made reachable through the daemon's route.
        let server = TestServer::start(vec![(200, "")], Duration::ZERO);
        let source = server.source();
        TEST_SOURCES.lock().unwrap().push(source);
        let (daemon, port) = start(Mode::Dedicated, Peers::default(), "daemon");
        // Two machines on the LAN, each with an empty cache of its own, ask the daemon.
        for tag in ["lan-a", "lan-b"] {
            let client = Fetcher::new(
                temp_cache(tag),
                Peers::new(vec![format!("127.0.0.1:{port}")]),
                false,
            );
            assert_eq!(client.fetch(source, K).unwrap(), b"tile-bytes", "{tag}");
            let t = client.traffic();
            assert_eq!(
                (t.lan_hits, t.downloads),
                (1, 0),
                "{tag} got it from the LAN, not the server"
            );
        }
        // The daemon downloaded it once and answered the second machine from its cache.
        assert_eq!(server.count(), 1);
        assert_eq!(daemon.traffic().downloads, 1);
        assert_eq!(
            daemon.cache().get(&source.cache_name(K)).as_deref(),
            Some(&b"tile-bytes"[..])
        );
    }

    /// Puts a tile into a machine's cache under a real source.
    fn seed(fetcher: &Fetcher, source: &TileSource, k: TileKey, bytes: &[u8]) {
        fetcher.cache().put(&source.cache_name(k), bytes).unwrap();
    }

    #[test]
    fn clients_get_tiles_from_the_lan_instead_of_downloading() {
        let (daemon, port) = start(Mode::Dedicated, Peers::default(), "lan-server");
        seed(&daemon, &sources::TERRAIN, K, b"from-lan");
        let client = Fetcher::new(
            temp_cache("lan-client"),
            Peers::new(vec![format!("127.0.0.1:{port}")]),
            false,
        );
        assert_eq!(client.fetch(&sources::TERRAIN, K).unwrap(), b"from-lan");
        let t = client.traffic();
        assert_eq!((t.lan_hits, t.downloads), (1, 0));
        // It is now in the client's own cache too.
        assert_eq!(client.fetch(&sources::TERRAIN, K).unwrap(), b"from-lan");
        assert_eq!(client.traffic().disk_hits, 1);
    }

    #[test]
    fn a_shared_app_offers_only_what_it_has_and_downloads_nothing_for_others() {
        let (app, port) = start(Mode::SharedApp, Peers::default(), "shared");
        seed(&app, &sources::EOX, K, b"cached");
        assert!(get(port, "/tiles/eox/3/2/1").0.contains("200"));
        let (status, _) = get(port, "/tiles/eox/4/2/1");
        assert!(status.contains("504"), "{status}");
        assert_eq!(app.traffic().downloads, 0);
    }

    #[test]
    fn a_shared_apps_gap_falls_through_to_the_next_server() {
        let (empty, empty_port) = start(Mode::SharedApp, Peers::default(), "gap-a");
        let (full, full_port) = start(Mode::SharedApp, Peers::default(), "gap-b");
        seed(&full, &sources::TERRAIN, K, b"second");
        let _ = empty;
        let peers = Peers::new(vec![
            format!("127.0.0.1:{empty_port}"),
            format!("127.0.0.1:{full_port}"),
        ]);
        let client = Fetcher::new(temp_cache("gap-client"), peers, false);
        assert_eq!(client.fetch(&sources::TERRAIN, K).unwrap(), b"second");
    }

    #[test]
    fn what_a_server_knows_is_missing_is_passed_on_and_remembered() {
        let (daemon, port) = start(Mode::Dedicated, Peers::default(), "missing-server");
        daemon.cache().mark_missing(&sources::TERRAIN.cache_name(K));
        let cache = temp_cache("missing-client");
        let client = Fetcher::new(
            cache.clone(),
            Peers::new(vec![format!("127.0.0.1:{port}")]),
            false,
        );
        assert_eq!(client.fetch(&sources::TERRAIN, K), Err(FetchError::Missing));
        assert!(cache.is_known_missing(&sources::TERRAIN.cache_name(K)));
    }

    #[test]
    fn a_public_server_timeout_is_respected_across_the_lan() {
        let (daemon, port) = start(Mode::Dedicated, Peers::default(), "pause-server");
        let host = crate::fetcher::host_of(sources::TERRAIN.url);
        let f = crate::servers::Failure::TooManyRequests { retry_after: None };
        daemon.servers.failed(&host, f, SystemTime::now());
        let (status, whole) = get(port, "/tiles/mapterhorn/3/2/1");
        assert!(status.contains("503"), "{status}");
        assert!(
            whole.to_ascii_lowercase().contains("retry-after: 8")
                || whole.contains("Retry-After: 89"),
            "{whole}"
        );
        let client = Fetcher::new(
            temp_cache("pause-client"),
            Peers::new(vec![format!("127.0.0.1:{port}")]),
            false,
        );
        let result = client.fetch(&sources::TERRAIN, K);
        let Err(FetchError::Paused(until)) = result else {
            panic!("{result:?}")
        };
        assert!(until.duration_since(SystemTime::now()).unwrap() > Duration::from_secs(800));
    }

    #[test]
    fn an_unreachable_cache_server_is_skipped_for_a_while() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = listener.local_addr().unwrap().port();
        drop(listener);
        let (alive, port) = start(Mode::SharedApp, Peers::default(), "skip-alive");
        seed(&alive, &sources::TERRAIN, K, b"alive");
        let peers = Peers::new(vec![
            format!("127.0.0.1:{dead}"),
            format!("127.0.0.1:{port}"),
        ]);
        let client = Fetcher::new(temp_cache("skip-client"), peers, false);
        assert_eq!(client.fetch(&sources::TERRAIN, K).unwrap(), b"alive");
        let list = client.peers_for_test();
        assert_eq!(
            list,
            [format!("127.0.0.1:{port}")],
            "the dead one is left out"
        );
    }

    #[test]
    fn only_get_and_head_are_accepted() {
        let (app, port) = start(Mode::SharedApp, Peers::default(), "methods");
        seed(&app, &sources::EOX, K, b"cached");
        let (status, _) = raw(port, "POST /tiles/eox/3/2/1 HTTP/1.1\r\n\r\n");
        assert!(status.contains("405"), "{status}");
        let (status, whole) = raw(port, "HEAD /tiles/eox/3/2/1 HTTP/1.1\r\n\r\n");
        assert!(
            status.contains("200") && !whole.contains("cached"),
            "{whole}"
        );
    }

    #[test]
    fn oversized_or_garbage_requests_are_dropped_harmlessly() {
        let (_app, port) = start(Mode::SharedApp, Peers::default(), "garbage");
        let huge = format!("GET /{} HTTP/1.1\r\n", "a".repeat(20_000));
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = s.write_all(huge.as_bytes());
        drop(s);
        let (status, _) = raw(port, "\u{1}\u{2}garbage\r\n\r\n");
        assert!(status.contains("405"), "{status}");
        // Still answering afterwards.
        assert!(get(port, "/status").0.contains("200"));
    }

    #[test]
    fn status_page_answers_with_json() {
        let (_app, port) = start(Mode::SharedApp, Peers::default(), "statuspage");
        let (status, whole) = get(port, "/status");
        assert!(
            status.contains("200") && whole.contains("\"traffic\""),
            "{whole}"
        );
    }

    #[test]
    fn beacons_round_trip_and_garbage_is_ignored() {
        let text = beacon_text(42, 47813, Mode::Dedicated);
        assert_eq!(parse_beacon(&text), Some((42, 47813, true)));
        assert_eq!(
            parse_beacon(&beacon_text(7, 9, Mode::SharedApp)),
            Some((7, 9, false))
        );
        for bad in [
            "",
            "hello",
            "orbis-cache/1",
            QUERY,
            "orbis-cache/1 x 1 shared",
            "orbis-cache/1 1 0 shared",
            "orbis-cache/1 1 2 root",
            "orbis-cache/1 1 2 shared more",
            "orbis-cache/2 1 2 shared",
        ] {
            assert_eq!(parse_beacon(bad), None, "{bad}");
        }
    }

    #[test]
    fn dedicated_servers_are_asked_first_and_silent_ones_forgotten() {
        let peers = Peers::default();
        let now = Instant::now();
        peers.heard("10.0.0.9:1".into(), false, now);
        peers.heard("10.0.0.8:1".into(), true, now);
        assert_eq!(peers.list_for_test(now), ["10.0.0.8:1", "10.0.0.9:1"]);
        assert!(peers
            .list_for_test(now + Duration::from_secs(30))
            .is_empty());
    }
}
