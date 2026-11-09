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

/// Which of OWID's (Ember's) sources each series of `FILTERS` counts towards; `None` for pumped
/// storage, which Ember leaves out, and for consumption. `tools/check_owid.py` has the same table.
pub fn owid_series(i: usize) -> Option<crate::owid::Series> {
    use crate::owid::Series as O;
    match FILTERS[i].0 {
        4068 => Some(O::Solar),
        4067 | 1225 => Some(O::Wind),
        1226 => Some(O::Hydro),
        4066 => Some(O::Bioenergy),
        1223 | 4069 => Some(O::Coal),
        4071 => Some(O::Gas),
        1227 => Some(O::Oil),
        1224 => Some(O::Nuclear),
        1228 => Some(O::OtherRenewables),
        _ => None,
    }
}

/// Lifecycle emissions per series of `FILTERS` in g CO₂-eq per kWh: the medians of IPCC AR5 WG3
/// Annex III, table A.III.2 (2014). Its coal figure (pulverised coal) stands for lignite and hard
/// coal, and, as it has none for oil or the gases and waste of "other conventional", for those
/// too. Pumped storage moves energy that was counted when it was generated, so it adds none.
pub const EMISSIONS: [f32; 13] = [
    48.0,  // solar PV, utility scale (rooftop 41)
    11.0,  // wind on land
    12.0,  // wind at sea
    24.0,  // hydro
    230.0, // biomass, dedicated
    820.0, // lignite: coal
    820.0, // hard coal
    490.0, // natural gas, combined cycle
    820.0, // other conventional: coal's figure
    12.0,  // nuclear
    38.0,  // other renewables: geothermal
    0.0,   // pumped storage
    0.0,   // consumption
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

    /// The carbon intensity of the mix by `EMISSIONS` alone, in g per kWh, not calibrated.
    /// Pumped storage is left out of the mix, as Ember leaves it out.
    pub fn intensity(&self) -> Option<f32> {
        mix_intensity(&self.mw.iter().map(|&v| v as f64).collect::<Vec<_>>()).map(|v| v as f32)
    }

    /// The moment with each series multiplied by its factor.
    pub fn scaled(&self, factors: &[f32; FILTERS.len()]) -> Moment {
        Moment { time: self.time, mw: self.mw.iter().zip(factors).map(|(v, f)| v * f).collect() }
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

/// The carbon intensity by `EMISSIONS` of amounts per series of `FILTERS` (power or energy).
pub fn mix_intensity(amounts: &[f64]) -> Option<f64> {
    let counted = |i: usize| owid_series(i).is_some();
    let total: f64 = (0..FILTERS.len()).filter(|&i| counted(i)).map(|i| amounts[i]).sum();
    let emitted: f64 = (0..FILTERS.len()).filter(|&i| counted(i)).map(|i| amounts[i] * EMISSIONS[i] as f64).sum();
    (total > 0.0).then(|| emitted / total)
}

/// One year of all series by month: MWh per series of `FILTERS` and month, `None` where SMARD
/// has no figure (yet).
#[derive(Clone, Debug, PartialEq)]
pub struct Year {
    pub year: i32,
    pub mwh: Vec<[Option<f64>; 12]>,
}

impl Year {
    /// All series of a month (1 to 12), if SMARD has the whole of it.
    pub fn month(&self, month: u32) -> Option<Vec<f64>> {
        self.mwh.iter().map(|m| m.get(month as usize - 1).copied().flatten()).collect()
    }
}

/// Midnight of 1 January, German time, in milliseconds: where SMARD's monthly files start.
pub fn year_start_ms(year: i32) -> i64 {
    (crate::sky_time::days_from_civil(year as i64, 1, 1) * 86_400 - 3600) * 1000
}

/// One series of a year file at monthly resolution, in MWh per month. A month counts once the
/// next one has begun, as the file's last value grows until then.
pub fn parse_months(body: &str, year: i32, now_ms: i64) -> Option<[Option<f64>; 12]> {
    let v: Value = serde_json::from_str(body).ok()?;
    let mut out = [None; 12];
    for pair in v.get("series")?.as_array()? {
        let (Some(t), Some(value)) = (pair.get(0).and_then(Value::as_i64), pair.get(1).and_then(Value::as_f64)) else { continue };
        // Months start at midnight German time, an hour or two before midnight UTC.
        let (y, m, _) = crate::sky_time::civil_from_days((t + 3 * 3_600_000).div_euclid(86_400_000));
        let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        let ended = crate::sky_time::days_from_civil(ny, nm, 1) * 86_400_000 <= now_ms;
        if y == year as i64 && value.is_finite() && value >= 0.0 && ended {
            out[m as usize - 1] = Some(value);
        }
    }
    Some(out)
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
    Year(i32, Result<Year, String>),
}

#[derive(Clone)]
enum Job {
    Index,
    Week(i64),
    Year(i32),
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

/// A year by month. Years that ended two months ago are kept for good, as SMARD settles its
/// figures by then; later ones for a day.
fn load_year(fetcher: &Fetcher, year: i32, wait: &mut Option<Instant>) -> Result<Year, String> {
    let start = year_start_ms(year);
    let now_ms = (unix_now() * 1000.0) as i64;
    let max_age = if year_start_ms(year + 1) + 61 * 86_400_000 < now_ms { f64::INFINITY } else { 86_400.0 };
    let mut mwh = Vec::new();
    for (filter, _) in FILTERS {
        let url = format!("{BASE}/{filter}/DE/{filter}_DE_month_{start}.json");
        let body = get(fetcher, &url, &format!("power/smard-month-{filter}-{start}.json"), max_age, wait)?;
        mwh.push(if body == MISSING { [Some(0.0); 12] } else { parse_months(&body, year, now_ms).ok_or("SMARD's answer could not be read")? });
    }
    Ok(Year { year, mwh })
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
            Job::Year(year) => Reply::Year(year, load_year(&fetcher, year, &mut wait)),
        });
        Self { worker }
    }

    pub fn ask_index(&self) {
        self.worker.submit(Job::Index);
    }

    pub fn ask_week(&self, start_ms: i64) {
        self.worker.submit(Job::Week(start_ms));
    }

    /// Asks for a year by month, for the factors to Ember's monthly figures.
    pub fn ask_year(&self, year: i32) {
        self.worker.submit(Job::Year(year));
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

    #[test]
    fn the_mapping_to_owid_is_the_check_scripts() {
        // MAPPING in tools/check_owid.py: "    4068: \"solar\"," or "    4070: None,".
        let script = include_str!("../tools/check_owid.py");
        let table = &script[script.find("MAPPING = {").unwrap()..];
        let table = &table[..table.find("\n}").unwrap()];
        let mut seen = 0;
        for line in table.lines().skip(1) {
            let (filter, rest) = line.trim().split_once(':').unwrap();
            let filter: u32 = filter.parse().unwrap();
            let key = rest.split('#').next().unwrap().trim().trim_end_matches(',').trim_matches('"');
            let i = FILTERS.iter().position(|f| f.0 == filter).unwrap_or_else(|| panic!("{filter} is not asked for"));
            assert_eq!(owid_series(i).map(|s| s.key()).unwrap_or("None"), key, "series {filter}");
            seen += 1;
        }
        assert_eq!(seen, FILTERS.len() - 1, "every generation series is in the script's table");
    }

    #[test]
    fn year_files_count_months_that_have_ended() {
        let start = year_start_ms(2026);
        assert_eq!(start, 1_767_222_000_000, "as in SMARD's index of monthly files");
        let feb = 1_769_900_400_000i64; // 1 February 2026, German time
        let body = format!(r#"{{"series":[[{start},100.0],[{feb},50.0],[{},null]]}}"#, feb + 28 * 86_400_000);
        let mid_feb = feb + 14 * 86_400_000;
        let m = parse_months(&body, 2026, mid_feb).unwrap();
        assert_eq!(m[0], Some(100.0));
        assert_eq!(m[1], None, "February is not over");
        let m = parse_months(&body, 2026, year_start_ms(2027)).unwrap();
        assert_eq!(m[1], Some(50.0));
        assert_eq!(m[2], None);
        let y = Year { year: 2026, mwh: vec![m; FILTERS.len()] };
        assert_eq!(y.month(1).unwrap().len(), FILTERS.len());
        assert!(y.month(3).is_none());
    }
}
