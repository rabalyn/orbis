//! Electricity generation by source and consumption in Germany, every 15 minutes, from the
//! Bundesnetzagentur's SMARD platform (CC BY 4.0, "Bundesnetzagentur | SMARD.de"). SMARD serves
//! each series in weekly files that start on Mondays at midnight, local time, listed in an index.
//! Values are energy per quarter hour in MWh, so four times the value is the mean power in MW.
//!
//! Like the event feeds, everything is fetched on a thread of its own, only while the energy bars
//! show output, and kept on disk: weeks that ended more than a day ago for good, the current week
//! for ten minutes.

use crate::energy::{Source, SOURCES};
use crate::event_feeds::{pack, unix_now, unpack};
use crate::fetcher::Fetcher;
use serde_json::Value;
use crate::worker::Worker;
use std::sync::Arc;
use crate::wtime::{Duration, Instant};

pub const CREDIT: &str = "Generation and consumption: Bundesnetzagentur | SMARD.de (CC BY 4.0)";
const BASE: &str = "https://www.smard.de/app/chart_data";
const QUARTER_MS: i64 = 900_000;
const WEEK_MS: i64 = 7 * 86_400_000;
/// Requests to SMARD are at least this far apart.
const MIN_GAP: Duration = Duration::from_millis(300);
/// How long the index and the current week are used before asking again.
const FRESH: f64 = 600.0;

/// A SMARD series: its filter number and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Series {
    /// Generation of sources of the register, by index into `Source::ALL`; several sources may
    /// share one series.
    Generation(&'static [usize]),
    /// Generation SMARD reports that the register's sources here do not cover.
    OtherGeneration,
    Consumption,
}

/// The series asked for, in a fixed order.
pub const FILTERS: [(u32, Series); 13] = [
    (4068, Series::Generation(&[0])),     // Photovoltaik
    (4067, Series::Generation(&[1])),     // Wind an Land
    (1225, Series::Generation(&[2])),     // Wind auf See
    (1226, Series::Generation(&[3])),     // Wasserkraft
    (4066, Series::Generation(&[4])),     // Biomasse
    (1223, Series::Generation(&[5])),     // Braunkohle
    (4069, Series::Generation(&[6])),     // Steinkohle
    (4071, Series::Generation(&[7])),     // Erdgas
    (1227, Series::Generation(&[8, 9])),  // Sonstige Konventionelle: oil, other gases, waste
    (1224, Series::Generation(&[10])),    // Kernenergie
    (1228, Series::OtherGeneration),      // Sonstige Erneuerbare
    (4070, Series::OtherGeneration),      // Pumpspeicher
    (410, Series::Consumption),           // Stromverbrauch: Gesamt (Netzlast)
];

/// One week of all series: mean power in MW per quarter hour, `None` where SMARD has no value yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Week {
    pub start_ms: i64,
    pub values: Vec<Vec<Option<f32>>>,
}

/// The mix at one quarter hour.
#[derive(Clone, Debug, PartialEq)]
pub struct Moment {
    /// Start of the quarter hour, unix seconds.
    pub time: f64,
    /// Mean power in MW for each series of `FILTERS`.
    pub mw: Vec<f32>,
}

impl Moment {
    pub fn generation(&self) -> f32 {
        FILTERS.iter().zip(&self.mw).filter(|(f, _)| f.1 != Series::Consumption).map(|(_, v)| v).sum()
    }

    pub fn renewable(&self) -> f32 {
        let renewable = |s: &Series| match s {
            Series::Generation(src) => !Source::ALL[src[0]].fossil(),
            // Other renewables count; pumped storage only moves energy in time, so it does not.
            Series::OtherGeneration => false,
            Series::Consumption => false,
        };
        FILTERS.iter().zip(&self.mw).filter(|(f, _)| renewable(&f.1)).map(|(_, v)| v).sum::<f32>() + self.mw[10]
    }

    pub fn consumption(&self) -> f32 {
        self.mw[FILTERS.len() - 1]
    }

    /// The share of each source's capacity that ran, from the national capacity in kW of each
    /// source. Sources that share a series share its share. Clamped to 0..1.
    pub fn shares(&self, capacity_kw: &[f32; SOURCES]) -> [f32; SOURCES] {
        let mut out = [0.0; SOURCES];
        for ((_, series), mw) in FILTERS.iter().zip(&self.mw) {
            if let Series::Generation(sources) = series {
                let cap: f32 = sources.iter().map(|&s| capacity_kw[s]).sum();
                let share = if cap > 0.0 { (mw * 1000.0 / cap).clamp(0.0, 1.0) } else { 0.0 };
                for &s in *sources {
                    out[s] = share;
                }
            }
        }
        out
    }
}

impl Week {
    /// The quarter hour holding `time`, if all series have a value for it.
    pub fn moment(&self, time: f64) -> Option<Moment> {
        let ms = (time * 1000.0) as i64;
        if ms < self.start_ms {
            return None;
        }
        let slot = ((ms - self.start_ms) / QUARTER_MS) as usize;
        let mw: Option<Vec<f32>> = self.values.iter().map(|v| v.get(slot).copied().flatten()).collect();
        Some(Moment { time: (self.start_ms + slot as i64 * QUARTER_MS) as f64 / 1000.0, mw: mw? })
    }

    /// The latest quarter hour that all series have, for a clock past the data.
    pub fn last_moment(&self) -> Option<Moment> {
        let len = self.values.iter().map(|v| v.iter().rposition(Option::is_some).map_or(0, |i| i + 1)).min()?;
        let t = (self.start_ms + (len as i64 - 1) * QUARTER_MS) as f64 / 1000.0;
        (len > 0).then(|| self.moment(t)).flatten()
    }
}

/// The week files' start times from an index answer, in milliseconds.
pub fn parse_index(body: &str) -> Option<Vec<i64>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut t: Vec<i64> = v.get("timestamps")?.as_array()?.iter().filter_map(Value::as_i64).collect();
    t.sort_unstable();
    (!t.is_empty()).then_some(t)
}

/// One series of a week file, as MW per quarter hour from its start.
pub fn parse_series(body: &str, start_ms: i64) -> Option<Vec<Option<f32>>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut out: Vec<Option<f32>> = Vec::new();
    for pair in v.get("series")?.as_array()? {
        let (Some(t), value) = (pair.get(0).and_then(Value::as_i64), pair.get(1).and_then(Value::as_f64)) else { continue };
        if t < start_ms || (t - start_ms) % QUARTER_MS != 0 {
            continue;
        }
        let slot = ((t - start_ms) / QUARTER_MS) as usize;
        if slot > (WEEK_MS / QUARTER_MS) as usize + 8 {
            continue;
        }
        if out.len() <= slot {
            out.resize(slot + 1, None);
        }
        out[slot] = value.filter(|x| x.is_finite() && *x >= 0.0).map(|mwh| (mwh * 4.0) as f32);
    }
    Some(out)
}

/// The week file that holds `time`, from the index.
pub fn week_for(index: &[i64], time: f64) -> Option<i64> {
    let ms = (time * 1000.0) as i64;
    let i = index.partition_point(|&t| t <= ms);
    (i > 0).then(|| index[i - 1]).filter(|&start| ms < start + WEEK_MS + 3_600_000)
}

pub enum Reply {
    Index(Result<Vec<i64>, String>),
    Week(i64, Result<Week, String>),
}

#[derive(Clone)]
enum Job {
    Index,
    Week(i64),
}

pub struct PowerFeed {
    worker: Worker<Job, Reply>,
}

/// What a week file holds where SMARD has none: nuclear since the last plants shut down in 2023.
const MISSING: &str = "missing";

/// Reads a stored answer if it is fresh enough, else fetches it and stores it. A file SMARD does
/// not have is `MISSING`.
fn get(fetcher: &Fetcher, url: &str, name: &str, max_age: f64, wait: &mut Option<Instant>) -> Result<String, String> {
    let stored = fetcher.cache().get(name);
    let parsed = stored.as_deref().and_then(unpack).map(|(t, b)| (t, b.to_string()));
    if let Some((stamp, body)) = &parsed {
        if fetcher.is_offline() || unix_now() - stamp < max_age {
            return Ok(body.clone());
        }
    }
    if fetcher.is_offline() {
        return Err("no stored data and the program is offline".to_string());
    }
    if let Some(last) = *wait {
        crate::wtime::pause((last + MIN_GAP).saturating_duration_since(Instant::now()));
    }
    *wait = Some(Instant::now());
    let answer = match fetcher.fetch_service(url) {
        Err(crate::fetcher::FetchError::Missing) => Some(MISSING.to_string()),
        r => r.ok().and_then(|d| String::from_utf8(d).ok()),
    };
    match answer {
        Some(body) => {
            if let Err(e) = fetcher.cache().put(name, &pack(unix_now(), body.as_bytes())) {
                eprintln!("cache: could not store {name}: {e}");
            }
            Ok(body)
        }
        // A stored answer that is too old still beats nothing.
        None => parsed.map(|p| p.1).ok_or_else(|| "SMARD could not be reached".to_string()),
    }
}

fn load_week(fetcher: &Fetcher, start: i64, wait: &mut Option<Instant>) -> Result<Week, String> {
    // Weeks that ended more than a day ago do not change any more.
    let done = (start + WEEK_MS) as f64 / 1000.0 + 86_400.0 < unix_now();
    let max_age = if done { f64::INFINITY } else { FRESH };
    let mut values = Vec::new();
    for (filter, _) in FILTERS {
        let url = format!("{BASE}/{filter}/DE/{filter}_DE_quarterhour_{start}.json");
        let body = get(fetcher, &url, &format!("power/smard-{filter}-{start}.json"), max_age, wait)?;
        if body == MISSING {
            // A series SMARD no longer has stands for no generation.
            values.push(vec![Some(0.0); (WEEK_MS / QUARTER_MS) as usize + 4]);
            continue;
        }
        values.push(parse_series(&body, start).ok_or("SMARD's answer could not be read")?);
    }
    Ok(Week { start_ms: start, values })
}

impl PowerFeed {
    pub fn new(fetcher: Arc<Fetcher>) -> Self {
        let mut wait = None;
        let worker = Worker::new(false, MIN_GAP, move |job: Job| match job {
            Job::Index => {
                let url = format!("{BASE}/410/DE/index_quarterhour.json");
                let r = get(&fetcher, &url, "power/smard-index.json", FRESH, &mut wait);
                Reply::Index(r.and_then(|b| parse_index(&b).ok_or_else(|| "SMARD's index could not be read".to_string())))
            }
            Job::Week(start) => Reply::Week(start, load_week(&fetcher, start, &mut wait)),
        });
        Self { worker }
    }

    pub fn ask_index(&self) {
        self.worker.submit(Job::Index);
    }

    pub fn ask_week(&self, start_ms: i64) {
        self.worker.submit(Job::Week(start_ms));
    }

    pub fn poll(&self) -> Option<Reply> {
        self.worker.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_are_read_as_megawatts_per_quarter_hour() {
        let start = 1_793_574_000_000;
        let body = format!(r#"{{"meta_data":{{}},"series":[[{start},1000.0],[{},null],[{},250.5],[12,5],["x",1]]}}"#, start + QUARTER_MS, start + 2 * QUARTER_MS);
        let s = parse_series(&body, start).unwrap();
        assert_eq!(s, vec![Some(4000.0), None, Some(1002.0)]);
        for bad in ["", "{}", "[]", "{\"series\":3}", "<html>"] {
            assert!(parse_series(bad, start).is_none_or(|s| s.is_empty()), "{bad}");
        }
        assert_eq!(parse_index(r#"{"timestamps":[3,1,2]}"#), Some(vec![1, 2, 3]));
        assert_eq!(parse_index("{}"), None);
    }

    #[test]
    fn the_week_and_the_quarter_hour_follow_the_clock() {
        let index = [1_000_000_000_000i64, 1_000_000_000_000 + WEEK_MS];
        assert_eq!(week_for(&index, 999_999_999.0), None);
        assert_eq!(week_for(&index, 1_000_000_100.0), Some(index[0]));
        assert_eq!(week_for(&index, (index[1] / 1000) as f64 + 10.0), Some(index[1]));
        assert_eq!(week_for(&index, (index[1] / 1000) as f64 + 30.0 * 86_400.0), None, "far past the last week");
        let n = FILTERS.len();
        let mut values = vec![vec![Some(1000.0), Some(2000.0), None]; n];
        values[0] = vec![Some(8000.0), Some(4000.0), Some(1.0)];
        let w = Week { start_ms: index[0], values };
        let m = w.moment(1_000_000_000.0 + 1000.0).unwrap();
        assert_eq!(m.time, 1_000_000_900.0);
        assert_eq!(m.mw[0], 4000.0);
        assert!(w.moment(1_000_000_000.0 + 1900.0).is_none(), "not all series have it");
        assert_eq!(w.last_moment().unwrap().time, 1_000_000_900.0);
        assert_eq!(m.consumption(), 2000.0);
        assert_eq!(m.generation(), 4000.0 + 2000.0 * (n as f32 - 2.0));
        // 4 GW of solar from 8 GW of capacity runs at half; oil and other share their series.
        let mut cap = [1e6f32; SOURCES];
        cap[0] = 8e6;
        let shares = m.shares(&cap);
        assert_eq!(shares[0], 0.5);
        assert_eq!(shares[8], 1.0, "2 GW from 2 GW of oil and other together");
        assert_eq!(shares[9], 1.0);
    }
}
