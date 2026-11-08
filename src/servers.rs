//! Tracks each public server's health, so that one under load is left alone. Time is passed
//! in, which keeps the rules testable without waiting for fifteen minutes.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use crate::wtime::{Duration, SystemTime, UNIX_EPOCH};

/// How many server errors within the window take a server out.
pub const STRIKES: usize = 3;
const STRIKE_WINDOW: Duration = Duration::from_secs(60);
pub const PAUSE: Duration = Duration::from_secs(15 * 60);
/// A server's own `Retry-After` is respected up to this, so a bad value cannot block it for good.
const LONGEST_PAUSE: Duration = Duration::from_secs(24 * 3600);
/// How soon to ask again while another request is testing whether a server has recovered.
const TRIAL_WAIT: Duration = Duration::from_secs(5);
const FIRST_BACKOFF: Duration = Duration::from_secs(2);
const LONGEST_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Failure {
    /// A 5xx answer or a request that timed out: signs of a server under load.
    ServerError { retry_after: Option<Duration> },
    /// The server told us to slow down.
    TooManyRequests { retry_after: Option<Duration> },
    /// We could not reach it, which may well be our own network.
    Unreachable,
}

#[derive(Default)]
struct Server {
    strikes: VecDeque<SystemTime>,
    paused_until: Option<SystemTime>,
    /// The pause has ended, but the server has not yet proven that it recovered.
    on_trial: bool,
    trial_running: bool,
    backoff_until: Option<SystemTime>,
    backoff: Option<Duration>,
    requests: u64,
    bytes: u64,
    errors: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerReport {
    pub host: String,
    pub requests: u64,
    pub bytes: u64,
    pub errors: u64,
    pub paused_until: Option<SystemTime>,
}

pub struct Servers {
    state: Mutex<HashMap<String, Server>>,
    /// Where pauses are stored, so that they outlast a restart.
    path: Option<PathBuf>,
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Servers {
    pub fn new(path: Option<PathBuf>) -> Self {
        let mut state: HashMap<String, Server> = HashMap::new();
        let now = SystemTime::now();
        if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                let Some((host, seconds)) = line.split_once(' ') else { continue };
                let Ok(seconds) = seconds.parse::<u64>() else { continue };
                let until = UNIX_EPOCH + Duration::from_secs(seconds);
                // A wrong or edited file must not block a server for longer than the rules allow.
                if until > now && until <= now + LONGEST_PAUSE {
                    state.insert(
                        host.to_string(),
                        Server { paused_until: Some(until), on_trial: true, ..Server::default() },
                    );
                }
            }
        }
        Self { state: Mutex::new(state), path }
    }

    fn save(&self, state: &HashMap<String, Server>) {
        let Some(path) = &self.path else { return };
        let mut text = String::new();
        for (host, server) in state {
            if let Some(until) = server.paused_until {
                text.push_str(&format!("{host} {}\n", unix(until)));
            }
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }

    /// Asks whether a request may go out now. `Err` holds when to ask again.
    pub fn check(&self, host: &str, now: SystemTime) -> Result<(), SystemTime> {
        let mut state = self.state.lock().unwrap();
        let server = state.entry(host.to_string()).or_default();
        if let Some(until) = server.paused_until {
            if until > now {
                return Err(until);
            }
            server.paused_until = None;
            server.on_trial = true;
            server.trial_running = false;
            self.save(&state);
            let server = state.get_mut(host).unwrap();
            return Self::after_pause(server, now);
        }
        Self::after_pause(server, now)
    }

    fn after_pause(server: &mut Server, now: SystemTime) -> Result<(), SystemTime> {
        if let Some(until) = server.backoff_until.filter(|u| *u > now) {
            return Err(until);
        }
        if server.on_trial {
            if server.trial_running {
                return Err(now + TRIAL_WAIT);
            }
            server.trial_running = true;
        }
        Ok(())
    }

    /// Notes a request that got an answer the server meant to give, such as a tile or a 404.
    pub fn succeeded(&self, host: &str, bytes: u64) {
        let mut state = self.state.lock().unwrap();
        let server = state.entry(host.to_string()).or_default();
        server.requests += 1;
        server.bytes += bytes;
        server.on_trial = false;
        server.trial_running = false;
        server.backoff = None;
        server.backoff_until = None;
    }

    /// Notes a failure. Returns when the server may be asked again, if that is later than now.
    pub fn failed(&self, host: &str, failure: Failure, now: SystemTime) -> Option<SystemTime> {
        let mut state = self.state.lock().unwrap();
        let server = state.entry(host.to_string()).or_default();
        server.requests += 1;
        server.errors += 1;
        server.trial_running = false;
        let pause_for = match failure {
            Failure::Unreachable => {
                let wait = server.backoff.map_or(FIRST_BACKOFF, |b| (b * 2).min(LONGEST_BACKOFF));
                server.backoff = Some(wait);
                server.backoff_until = Some(now + wait);
                return server.backoff_until;
            }
            Failure::TooManyRequests { retry_after } => Some(retry_after),
            Failure::ServerError { retry_after } => {
                server.strikes.push_back(now);
                while server
                    .strikes
                    .front()
                    .is_some_and(|t| now.duration_since(*t).unwrap_or_default() > STRIKE_WINDOW)
                {
                    server.strikes.pop_front();
                }
                (server.on_trial || server.strikes.len() >= STRIKES).then_some(retry_after)
            }
        };
        let retry_after = pause_for?;
        let length = retry_after.unwrap_or(PAUSE).clamp(PAUSE, LONGEST_PAUSE);
        let until = now + length;
        server.paused_until = Some(until);
        server.on_trial = true;
        server.strikes.clear();
        self.save(&state);
        eprintln!("{host}: leaving this server alone for {} minutes", length.as_secs() / 60);
        Some(until)
    }

    /// When the server's timeout ends, if it has one. Short back-offs do not count.
    #[cfg(test)]
    pub fn paused_until(&self, host: &str, now: SystemTime) -> Option<SystemTime> {
        let state = self.state.lock().unwrap();
        state.get(host)?.paused_until.filter(|u| *u > now)
    }

    pub fn report(&self, now: SystemTime) -> Vec<ServerReport> {
        let state = self.state.lock().unwrap();
        let mut out: Vec<_> = state
            .iter()
            .map(|(host, s)| ServerReport {
                host: host.clone(),
                requests: s.requests,
                bytes: s.bytes,
                errors: s.errors,
                paused_until: s.paused_until.filter(|u| *u > now),
            })
            .collect();
        out.sort_by(|a, b| a.host.cmp(&b.host));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ERROR: Failure = Failure::ServerError { retry_after: None };
    const HOST: &str = "tiles.example.org";

    fn start() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn servers() -> Servers {
        Servers::new(None)
    }

    #[test]
    fn a_few_server_errors_in_a_row_time_the_server_out_for_15_minutes() {
        let s = servers();
        let t = start();
        assert!(s.check(HOST, t).is_ok());
        assert_eq!(s.failed(HOST, ERROR, t), None, "one error can be a single broken tile");
        assert!(s.check(HOST, t).is_ok());
        assert_eq!(s.failed(HOST, ERROR, t + secs(1)), None);
        let until = s.failed(HOST, ERROR, t + secs(2)).unwrap();
        assert_eq!(until, t + secs(2) + PAUSE);
        assert_eq!(s.check(HOST, t + secs(3)), Err(until));
        assert_eq!(s.check(HOST, t + secs(2) + PAUSE - secs(1)), Err(until));
        assert_eq!(PAUSE, secs(900));
    }

    #[test]
    fn errors_spread_over_time_do_not_add_up() {
        let s = servers();
        let t = start();
        for i in 0..10 {
            assert_eq!(s.failed(HOST, ERROR, t + secs(i * 61)), None);
            assert!(s.check(HOST, t + secs(i * 61)).is_ok());
        }
    }

    #[test]
    fn being_told_to_slow_down_counts_at_once() {
        let s = servers();
        let t = start();
        let until = s.failed(HOST, Failure::TooManyRequests { retry_after: None }, t).unwrap();
        assert_eq!(until, t + PAUSE);
    }

    #[test]
    fn retry_after_can_lengthen_a_timeout_but_not_shorten_it_or_make_it_endless() {
        let t = start();
        let pause_for = |seconds| {
            let f = Failure::TooManyRequests { retry_after: Some(secs(seconds)) };
            servers().failed(HOST, f, t).unwrap()
        };
        assert_eq!(pause_for(3600), t + secs(3600));
        assert_eq!(pause_for(5), t + PAUSE);
        assert_eq!(pause_for(10_000_000), t + secs(24 * 3600));
    }

    #[test]
    fn after_a_timeout_one_trial_at_a_time_tests_the_server() {
        let s = servers();
        let t = start();
        let f = Failure::TooManyRequests { retry_after: None };
        let until = s.failed(HOST, f, t).unwrap();
        let later = until + secs(1);
        assert!(s.check(HOST, later).is_ok(), "the first request is the trial");
        let blocked = s.check(HOST, later);
        assert_eq!(blocked, Err(later + TRIAL_WAIT), "others wait for its result");
        s.succeeded(HOST, 100);
        assert!(s.check(HOST, later).is_ok());
        assert!(s.check(HOST, later).is_ok(), "recovered, so no more limits");
    }

    #[test]
    fn a_failed_trial_means_another_full_timeout() {
        let s = servers();
        let t = start();
        let until = s.failed(HOST, Failure::TooManyRequests { retry_after: None }, t).unwrap();
        let later = until + secs(1);
        assert!(s.check(HOST, later).is_ok());
        assert_eq!(s.failed(HOST, ERROR, later), Some(later + PAUSE), "a single error is enough");
    }

    #[test]
    fn no_network_gets_a_short_growing_backoff_rather_than_a_timeout() {
        let s = servers();
        let t = start();
        assert_eq!(s.failed(HOST, Failure::Unreachable, t), Some(t + secs(2)));
        assert_eq!(s.failed(HOST, Failure::Unreachable, t), Some(t + secs(4)));
        for _ in 0..10 {
            s.failed(HOST, Failure::Unreachable, t);
        }
        assert_eq!(s.failed(HOST, Failure::Unreachable, t), Some(t + secs(60)));
        assert_eq!(s.paused_until(HOST, t), None);
        assert!(s.check(HOST, t + secs(61)).is_ok());
        s.succeeded(HOST, 1);
        assert_eq!(s.failed(HOST, Failure::Unreachable, t), Some(t + secs(2)), "starts over");
    }

    #[test]
    fn servers_are_tracked_separately() {
        let s = servers();
        let t = start();
        s.failed(HOST, Failure::TooManyRequests { retry_after: None }, t);
        assert!(s.check(HOST, t).is_err());
        assert!(s.check("other.example.org", t).is_ok());
    }

    #[test]
    fn timeouts_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("orbis-servers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("servers.txt");
        let now = SystemTime::now();
        let s = Servers::new(Some(path.clone()));
        let until = s.failed(HOST, Failure::TooManyRequests { retry_after: None }, now).unwrap();
        drop(s);
        let restarted = Servers::new(Some(path.clone()));
        let found = restarted.paused_until(HOST, now + secs(1)).unwrap();
        assert_eq!(unix(found), unix(until));
        assert!(restarted.check(HOST, now + secs(1)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_state_file_is_ignored() {
        let dir = std::env::temp_dir().join(format!("orbis-servers-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.txt");
        let far_future = unix(SystemTime::now()) + 10_000_000;
        std::fs::write(&path, format!("garbage\nhost notanumber\nforever.example {far_future}\n")).unwrap();
        let s = Servers::new(Some(path));
        assert!(s.check("forever.example", SystemTime::now()).is_ok());
        assert!(s.check("host", SystemTime::now()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reports_show_traffic_and_timeouts() {
        let s = servers();
        let t = start();
        s.succeeded(HOST, 1000);
        s.succeeded(HOST, 500);
        s.failed(HOST, ERROR, t);
        let r = &s.report(t)[0];
        assert_eq!((r.requests, r.bytes, r.errors, r.paused_until), (3, 1500, 1, None));
    }
}
