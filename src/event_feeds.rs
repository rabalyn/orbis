//! Event feeds from public services: earthquakes (USGS), significant eruptions and tsunamis
//! (NOAA NCEI Hazard Service) and fireballs (NASA/JPL CNEOS). Each feed has its address for a
//! date, a defensive parser that skips records it cannot read, a disk cache with an age, and a
//! worker thread so that no frame waits. Nothing is requested unless the user asks for the events.

use crate::events::{Event, Kind};
use crate::fetcher::Fetcher;
use crate::sky_time::{days_from_civil, format_utc};
use serde_json::Value;
use crate::worker::Worker;
use std::sync::Arc;
use crate::wtime::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DAY: f64 = 86_400.0;
/// Requests to a publisher are at least this far apart.
const MIN_GAP: Duration = Duration::from_millis(1200);
/// Where the live earthquake feed is used instead of a dated query: the clock is this recent.
const LIVE_DAYS: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feed {
    Quakes,
    Eruptions,
    Tsunamis,
    Fireballs,
}

impl Feed {
    pub const ALL: [Feed; 4] = [Feed::Quakes, Feed::Eruptions, Feed::Tsunamis, Feed::Fireballs];

    /// The name of the source in `EventList`, and of the command line option's value.
    pub fn source(self) -> &'static str {
        match self {
            Feed::Quakes => "quakes",
            Feed::Eruptions => "eruptions",
            Feed::Tsunamis => "tsunamis",
            Feed::Fireballs => "fireballs",
        }
    }

    pub fn from_name(name: &str) -> Option<Feed> {
        Feed::ALL.into_iter().find(|f| f.source() == name.trim().to_ascii_lowercase())
    }

    pub fn kind(self) -> Kind {
        match self {
            Feed::Quakes => Kind::Earthquake,
            Feed::Eruptions => Kind::Eruption,
            Feed::Tsunamis => Kind::Tsunami,
            Feed::Fireballs => Kind::Fireball,
        }
    }

    pub fn publisher(self) -> &'static str {
        match self {
            Feed::Quakes => "the U.S. Geological Survey",
            Feed::Eruptions | Feed::Tsunamis => "NOAA's National Centers for Environmental Information",
            Feed::Fireballs => "NASA/JPL CNEOS",
        }
    }

    /// Shown on screen while the feed's events are.
    pub fn credit(self) -> &'static str {
        match self {
            Feed::Quakes => "Earthquakes: U.S. Geological Survey",
            Feed::Eruptions => "Eruptions: NOAA NCEI, Global Significant Volcanic Eruptions Database",
            Feed::Tsunamis => "Tsunamis: NOAA NCEI/WDS Global Historical Tsunami Database (doi:10.7289/V5PN93H7)",
            Feed::Fireballs => "Fireballs: NASA/JPL Center for Near Earth Object Studies (CNEOS)",
        }
    }

    /// How long a stored answer is used without asking again.
    pub fn max_age(self) -> f64 {
        match self {
            Feed::Quakes => 10.0 * 60.0,
            _ => DAY,
        }
    }

    /// The address for events around the clock's `time`, with `now` the real time.
    pub fn url(self, time: f64, now: f64) -> String {
        let day = (time / DAY).floor() * DAY;
        let date = |t: f64| format_utc(t, false)[..10].to_string();
        let year = |t: f64| format_utc(t, false)[..4].to_string();
        match self {
            Feed::Quakes if time > now - LIVE_DAYS * DAY && time < now + DAY => {
                "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/4.5_month.geojson".to_string()
            }
            Feed::Quakes => format!(
                "https://earthquake.usgs.gov/fdsnws/event/1/query?format=geojson&starttime={}&endtime={}&minmagnitude=5&orderby=time&limit=500",
                date(day - 15.0 * DAY),
                date(day + 16.0 * DAY)
            ),
            Feed::Eruptions => format!(
                "https://www.ngdc.noaa.gov/hazel/hazard-service/api/v1/volcanoes?minYear={}&maxYear={}",
                year(day - 30.0 * DAY),
                year(day + 30.0 * DAY)
            ),
            Feed::Tsunamis => format!(
                "https://www.ngdc.noaa.gov/hazel/hazard-service/api/v1/tsunamis/events?minYear={}&maxYear={}",
                year(day - 30.0 * DAY),
                year(day + 30.0 * DAY)
            ),
            Feed::Fireballs => format!(
                "https://ssd-api.jpl.nasa.gov/fireball.api?req-loc=true&date-min={}&date-max={}&limit=300",
                date(day - 90.0 * DAY),
                date(day + 91.0 * DAY)
            ),
        }
    }

    pub fn parse(self, text: &str) -> Result<Vec<Event>, String> {
        match self {
            Feed::Quakes => parse_quakes(text),
            Feed::Eruptions => parse_eruptions(text),
            Feed::Tsunamis => parse_tsunamis(text),
            Feed::Fireballs => parse_fireballs(text),
        }
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|x| x.is_finite())
}

fn text(v: &Value) -> Option<&str> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn position(lat: f64, lon: f64) -> Option<(f64, f64)> {
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then(|| (lat.to_radians(), lon.to_radians()))
}

/// Unix seconds of a civil date and time; parts out of range are refused.
fn civil(year: f64, month: f64, day: f64, hour: f64, minute: f64, second: f64) -> Option<f64> {
    let ok = (1800.0..=2200.0).contains(&year) && (1.0..=12.0).contains(&month) && (1.0..=31.0).contains(&day) && (0.0..24.0).contains(&hour) && (0.0..60.0).contains(&minute) && (0.0..61.0).contains(&second);
    ok.then(|| days_from_civil(year as i64, month as i64, day as i64) as f64 * DAY + hour * 3600.0 + minute * 60.0 + second)
}

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|_| "not JSON".to_string())
}

#[allow(clippy::too_many_arguments)]
fn event(feed: Feed, time: f64, lat: f64, lon: f64, marker: f32, distance: f64, title: &str, detail: String, note: String) -> Event {
    Event { source: feed.source(), kind: feed.kind(), time, lat, lon, distance, set_clock: true, title: title.to_string(), detail, note, marker }
}

/// USGS GeoJSON summary or query: features with `properties.mag`, `place`, `time` (milliseconds)
/// and `geometry.coordinates` as longitude, latitude, depth in km.
pub fn parse_quakes(body: &str) -> Result<Vec<Event>, String> {
    let v = json(body)?;
    let features = v.get("features").and_then(Value::as_array).ok_or("no features")?;
    let mut out = Vec::new();
    for f in features {
        let p = &f["properties"];
        let c = f["geometry"]["coordinates"].as_array();
        let (Some(mag), Some(ms), Some(c)) = (number(&p["mag"]), number(&p["time"]), c) else { continue };
        let (Some(lon), Some(lat)) = (c.first().and_then(number), c.get(1).and_then(number)) else { continue };
        if p["type"].as_str().is_some_and(|t| t != "earthquake") {
            continue;
        }
        let Some((lat, lon)) = position(lat, lon) else { continue };
        let place = text(&p["place"]).unwrap_or("unknown place");
        let depth = c.get(2).and_then(number);
        let mut note = format!("Magnitude {mag:.1} earthquake, {place}");
        if let Some(d) = depth {
            note.push_str(&format!(", {d:.0} km deep"));
        }
        note.push('.');
        let size = (4.5 + (mag - 4.0).max(0.0) * 3.0).min(24.0) as f32;
        out.push(event(Feed::Quakes, ms / 1000.0, lat, lon, size, 1.2e6, "Earthquake", format!("M{mag:.1}  {place}"), note));
    }
    Ok(out)
}

/// NCEI volcano events: `items` with year, optional month and day, latitude, longitude, name,
/// country, vei, and `eruption` (events that were not eruptions are left out).
pub fn parse_eruptions(body: &str) -> Result<Vec<Event>, String> {
    let v = json(body)?;
    let items = v.get("items").and_then(Value::as_array).ok_or("no items")?;
    let mut out = Vec::new();
    for i in items {
        if i["eruption"].as_bool() == Some(false) {
            continue;
        }
        let (Some(y), Some(lat), Some(lon)) = (number(&i["year"]), number(&i["latitude"]), number(&i["longitude"])) else { continue };
        let at = |k: &str, d: f64| number(&i[k]).unwrap_or(d);
        let Some(time) = civil(y, at("month", 1.0), at("day", 1.0), 0.0, 0.0, 0.0).or_else(|| civil(y, 1.0, 1.0, 0.0, 0.0, 0.0)) else { continue };
        let Some((lat, lon)) = position(lat, lon) else { continue };
        let name = text(&i["name"]).unwrap_or("unnamed volcano");
        let place = match text(&i["country"]) {
            Some(c) => format!("{name}, {c}"),
            None => name.to_string(),
        };
        let vei = number(&i["vei"]);
        let detail = match vei {
            Some(v) => format!("VEI {v:.0}  {place}"),
            None => place.clone(),
        };
        let note = format!("Eruption of {place}{}. Only the day is known.", vei.map_or(String::new(), |v| format!(", explosivity index {v:.0}")));
        let size = (7.0 + vei.unwrap_or(2.0).clamp(0.0, 8.0) * 2.0) as f32;
        out.push(event(Feed::Eruptions, time, lat, lon, size, 6.0e5, "Eruption", detail, note));
    }
    Ok(out)
}

/// NCEI tsunami events: `items` with date and time parts, `eqMagnitude`, `locationName`,
/// `country`, `maxWaterHeight` in metres, latitude and longitude.
pub fn parse_tsunamis(body: &str) -> Result<Vec<Event>, String> {
    let v = json(body)?;
    let items = v.get("items").and_then(Value::as_array).ok_or("no items")?;
    let mut out = Vec::new();
    for i in items {
        let (Some(y), Some(lat), Some(lon)) = (number(&i["year"]), number(&i["latitude"]), number(&i["longitude"])) else { continue };
        let at = |k: &str, d: f64| number(&i[k]).unwrap_or(d);
        let Some(time) = civil(y, at("month", 1.0), at("day", 1.0), at("hour", 0.0), at("minute", 0.0), at("second", 0.0)).or_else(|| civil(y, 1.0, 1.0, 0.0, 0.0, 0.0)) else { continue };
        let Some((lat, lon)) = position(lat, lon) else { continue };
        let place = [text(&i["locationName"]), text(&i["country"])].into_iter().flatten().collect::<Vec<_>>().join(", ");
        let place = if place.is_empty() { "unknown place".to_string() } else { place };
        let mag = number(&i["eqMagnitude"]);
        let height = number(&i["maxWaterHeight"]).filter(|h| *h > 0.0);
        let mut detail = mag.map_or(String::new(), |m| format!("M{m:.1}  "));
        detail.push_str(&place);
        let mut note = format!("Tsunami, {place}");
        if let Some(h) = height {
            note.push_str(&format!(", highest water {h:.1} m"));
        }
        note.push('.');
        let size = 8.0 + (mag.unwrap_or(6.0).clamp(4.0, 9.5) - 4.0) * 2.0;
        let size = size as f32;
        out.push(event(Feed::Tsunamis, time, lat, lon, size, 2.0e6, "Tsunami", detail, note));
    }
    Ok(out)
}

/// CNEOS fireballs: `fields` names the columns of each row in `data`; date, lat, lat-dir,
/// lon, lon-dir, energy (kt of TNT) and alt (km) are used. A reply without `data` is no events.
pub fn parse_fireballs(body: &str) -> Result<Vec<Event>, String> {
    let v = json(body)?;
    if v.get("signature").is_none() && v.get("data").is_none() {
        return Err("not a fireball reply".into());
    }
    let Some(data) = v.get("data").and_then(Value::as_array) else { return Ok(Vec::new()) };
    let fields: Vec<&str> = v.get("fields").and_then(Value::as_array).ok_or("no fields")?.iter().filter_map(Value::as_str).collect();
    let col = |name: &str| fields.iter().position(|f| *f == name);
    let (Some(cd), Some(clat), Some(clon)) = (col("date"), col("lat"), col("lon")) else { return Err("columns missing".into()) };
    let (cld, clod, ce, ca) = (col("lat-dir"), col("lon-dir"), col("energy"), col("alt"));
    let mut out = Vec::new();
    for row in data {
        let Some(row) = row.as_array() else { continue };
        let get = |c: usize| row.get(c).unwrap_or(&Value::Null);
        let Some(date) = text(get(cd)) else { continue };
        let Some(time) = crate::sky_time::parse_iso8601(date) else { continue };
        let (Some(lat), Some(lon)) = (number(get(clat)), number(get(clon))) else { continue };
        let south = cld.and_then(|c| text(get(c))) == Some("S");
        let west = clod.and_then(|c| text(get(c))) == Some("W");
        let Some((lat, lon)) = position(if south { -lat } else { lat }, if west { -lon } else { lon }) else { continue };
        let energy = ce.and_then(|c| number(get(c)));
        let alt = ca.and_then(|c| number(get(c)));
        let detail = match energy {
            Some(e) => format!("{e} kt"),
            None => "energy unknown".to_string(),
        };
        let mut note = format!("Fireball seen by sensors in the sky, radiated energy {}", energy.map_or("unknown".to_string(), |e| format!("{e} kt of TNT")));
        if let Some(a) = alt {
            note.push_str(&format!(", at {a:.0} km height"));
        }
        note.push('.');
        let size = (8.0 + energy.unwrap_or(1.0).max(0.1).log10() * 3.0).clamp(5.0, 20.0) as f32;
        out.push(event(Feed::Fireballs, time, lat, lon, size, 2.0e6, "Fireball", detail, note));
    }
    Ok(out)
}

/// Why a feed has nothing to show, in words for the panel.
#[derive(Debug, Clone, PartialEq)]
pub enum FeedError {
    Offline,
    Unreachable,
    Garbled,
}

impl FeedError {
    pub fn message(&self, feed: Feed) -> String {
        match self {
            FeedError::Offline => format!("No stored {} and the program is offline.", feed.source()),
            FeedError::Unreachable => format!("Could not reach {} for the {}.", feed.publisher(), feed.source()),
            FeedError::Garbled => format!("The answer from {} for the {} could not be read.", feed.publisher(), feed.source()),
        }
    }
}

pub struct Reply {
    pub feed: Feed,
    pub answer: Result<Vec<Event>, FeedError>,
}

/// Stored answers carry the time they were fetched in a first line.
pub(crate) fn pack(now: f64, body: &[u8]) -> Vec<u8> {
    let mut out = format!("orbis-feed {now:.0}\n").into_bytes();
    out.extend_from_slice(body);
    out
}

pub(crate) fn unpack(data: &[u8]) -> Option<(f64, &str)> {
    let end = data.iter().position(|b| *b == b'\n')?;
    let stamp = std::str::from_utf8(&data[..end]).ok()?.strip_prefix("orbis-feed ")?.parse().ok()?;
    Some((stamp, std::str::from_utf8(&data[end + 1..]).ok()?))
}

fn cache_name(feed: Feed, url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("events/{}-{h:016x}.json", feed.source())
}

pub(crate) fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn load(fetcher: &Fetcher, feed: Feed, url: &str, wait: &mut Option<Instant>) -> Result<Vec<Event>, FeedError> {
    let name = cache_name(feed, url);
    let stored = fetcher.cache().get(&name);
    let parsed = stored.as_deref().and_then(unpack).and_then(|(t, b)| feed.parse(b).ok().map(|e| (t, e)));
    if let Some((stamp, events)) = &parsed {
        if fetcher.is_offline() || unix_now() - stamp < feed.max_age() {
            return Ok(events.clone());
        }
    }
    if fetcher.is_offline() {
        return Err(FeedError::Offline);
    }
    if let Some(last) = *wait {
        crate::wtime::pause((last + MIN_GAP).saturating_duration_since(Instant::now()));
    }
    *wait = Some(Instant::now());
    // A stored answer that is too old still beats nothing when the publisher cannot be reached.
    let stale = || parsed.clone().map(|(_, e)| e).ok_or(FeedError::Unreachable);
    let Ok(data) = fetcher.fetch_service(url) else { return stale() };
    let Ok(body) = std::str::from_utf8(&data) else { return stale().map_err(|_| FeedError::Garbled) };
    match feed.parse(body) {
        Ok(events) => {
            if let Err(e) = fetcher.cache().put(&name, &pack(unix_now(), &data)) {
                eprintln!("cache: could not store {}: {e}", feed.source());
            }
            Ok(events)
        }
        Err(_) => stale().map_err(|_| FeedError::Garbled),
    }
}

/// Asks the feeds on a thread of its own.
pub struct Feeds {
    worker: Worker<(Feed, String), Reply>,
}

impl Feeds {
    pub fn new(fetcher: Arc<Fetcher>) -> Self {
        let mut wait = None;
        let worker = Worker::new(false, MIN_GAP, move |(feed, url): (Feed, String)| {
            let answer = load(&fetcher, feed, &url, &mut wait);
            Reply { feed, answer }
        });
        Self { worker }
    }

    pub fn submit(&self, feed: Feed, time: f64) {
        self.worker.submit((feed, feed.url(time, unix_now())));
    }

    pub fn poll(&self) -> Option<Reply> {
        self.worker.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky_time::parse_iso8601;

    const QUAKES: &str = r#"{"type":"FeatureCollection","features":[
      {"properties":{"mag":6.1,"place":"10 km N of Somewhere","time":1712600000000,"type":"earthquake"},"geometry":{"type":"Point","coordinates":[-104.5,25.25,12.5]}},
      {"properties":{"mag":null,"place":"no magnitude","time":1712600000000},"geometry":{"coordinates":[1,2,3]}},
      {"properties":{"mag":5.0,"place":"blast","time":1712600000000,"type":"quarry blast"},"geometry":{"coordinates":[1,2,3]}},
      {"properties":{"mag":5.0,"time":1712600000000},"geometry":{"coordinates":[1]}},
      {"properties":{"mag":4.5,"time":1712600000000},"geometry":{"coordinates":[200,2,3]}},
      {"properties":{"mag":4.7,"time":1712600000000},"geometry":{"coordinates":[10,-20]}}]}"#;

    #[test]
    fn quakes_are_read_and_bad_records_skipped() {
        let e = parse_quakes(QUAKES).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].time, 1_712_600_000.0);
        assert!((e[0].lat.to_degrees() - 25.25).abs() < 1e-9 && (e[0].lon.to_degrees() + 104.5).abs() < 1e-9);
        assert_eq!(e[0].detail, "M6.1  10 km N of Somewhere");
        assert!(e[0].note.contains("12 km deep") || e[0].note.contains("13 km deep"));
        assert!(e[0].marker > e[1].marker);
        assert_eq!(e[1].detail, "M4.7  unknown place");
    }

    #[test]
    fn malformed_input_is_an_error_or_empty_never_a_panic() {
        for bad in ["", "<html>", "[]", "{}", "null", "{\"features\":3}", "{\"items\":{}}", "{\"features\":[1,null,\"x\"]}"] {
            for feed in Feed::ALL {
                let r = feed.parse(bad);
                assert!(r.is_err() || r.unwrap().is_empty(), "{bad} {feed:?}");
            }
        }
        assert!(parse_fireballs("{\"signature\":{},\"count\":\"0\"}").unwrap().is_empty());
        assert!(parse_fireballs("{\"signature\":{},\"fields\":[\"date\"],\"data\":[[\"x\"]]}").is_err());
    }

    #[test]
    fn eruptions_use_the_date_that_is_known() {
        let body = r#"{"items":[
          {"year":2023,"month":12,"day":3,"name":"Marapi","country":"Indonesia","latitude":-0.38,"longitude":100.474,"vei":2,"eruption":true},
          {"year":2020,"name":"Nowhere","latitude":10,"longitude":20},
          {"year":2020,"month":1,"day":1,"name":"Not one","latitude":10,"longitude":20,"eruption":false},
          {"year":"x","latitude":1,"longitude":1},
          {"year":2021,"month":13,"day":40,"latitude":5,"longitude":5}]}"#;
        let e = parse_eruptions(body).unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].time, parse_iso8601("2023-12-03").unwrap());
        assert_eq!(e[0].detail, "VEI 2  Marapi, Indonesia");
        assert_eq!(e[1].time, parse_iso8601("2020-01-01").unwrap());
        assert_eq!(e[2].time, parse_iso8601("2021-01-01").unwrap());
    }

    #[test]
    fn tsunamis_read_time_magnitude_and_place() {
        let body = r#"{"items":[{"year":2023,"month":1,"day":9,"hour":17,"minute":47,"second":35,"eqMagnitude":7.6,"country":"INDONESIA","locationName":"SOUTH MALUKU","latitude":-7.049,"longitude":130.038,"maxWaterHeight":0.09},
          {"year":2022,"latitude":1,"longitude":1}]}"#;
        let e = parse_tsunamis(body).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].time, parse_iso8601("2023-01-09T17:47:35Z").unwrap());
        assert_eq!(e[0].detail, "M7.6  SOUTH MALUKU, INDONESIA");
        assert_eq!(e[1].detail, "unknown place");
    }

    #[test]
    fn fireballs_follow_the_field_names_and_signs() {
        let body = r#"{"signature":{"source":"NASA/JPL Fireball Data API","version":"1.2"},"count":"3",
          "fields":["date","energy","impact-e","lat","lat-dir","lon","lon-dir","alt","vel"],
          "data":[["2026-10-04 03:14:45","5.4","0.18","41.8","S","173.1","W","26.9","13.8"],
                  ["2026-09-15 11:26:13","2.2","0.079","37.6","N","161.6","E",null,null],
                  ["bad","1","1","1","N","1","E",null,null]]}"#;
        let e = parse_fireballs(body).unwrap();
        assert_eq!(e.len(), 2);
        assert!((e[0].lat.to_degrees() + 41.8).abs() < 1e-9 && (e[0].lon.to_degrees() + 173.1).abs() < 1e-9);
        assert!(e[1].lon > 0.0 && e[1].lat > 0.0);
        assert_eq!(e[0].detail, "5.4 kt");
    }

    #[test]
    fn addresses_follow_the_date() {
        let now = parse_iso8601("2026-11-07T12:00:00Z").unwrap();
        let recent = Feed::Quakes.url(now - 2.0 * DAY, now);
        assert!(recent.ends_with("4.5_month.geojson"));
        let old = Feed::Quakes.url(parse_iso8601("2024-04-08T18:17:19Z").unwrap(), now);
        assert!(old.contains("starttime=2024-03-24&endtime=2024-04-24"), "{old}");
        let t = parse_iso8601("2024-04-08T18:17:19Z").unwrap();
        assert!(Feed::Eruptions.url(t, now).ends_with("minYear=2024&maxYear=2024"));
        assert!(Feed::Fireballs.url(t, now).contains("date-min=2024-01-09&date-max=2024-07-08"));
        // The same day gives the same address, so that the stored answer is found again.
        assert_eq!(Feed::Tsunamis.url(t, now), Feed::Tsunamis.url(t + 3000.0, now));
    }

    #[test]
    fn stored_answers_carry_their_age() {
        let packed = pack(1234.0, b"{\"a\":1}");
        assert_eq!(unpack(&packed), Some((1234.0, "{\"a\":1}")));
        assert_eq!(unpack(b"{}"), None);
        assert!(Feed::Quakes.max_age() < Feed::Eruptions.max_age());
        assert_eq!(Feed::from_name("Quakes"), Some(Feed::Quakes));
        assert_eq!(Feed::from_name("x"), None);
    }
}
