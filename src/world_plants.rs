//! The Plants level of the energy layer beyond Germany: the world's power plants from Global
//! Energy Monitor's Global Integrated Power Tracker (September 2026 release, CC BY 4.0), filtered
//! and merged into sites by `tools/gen_world_plants.py` into `data/plants_world.bin`. Germany's
//! plants are the Marktstammdatenregister's, in `energy.rs`, and GEM's German units are left out.
//!
//! A site is the units of one source at one GEM location. Its bar stands for installed capacity
//! at the clock's month, solid up to its estimated output (`world_output.rs`: capacity times its
//! country's capacity factor for the source and month) and the rest above as unused capacity.
//! The world's states sum the sites inside their outline.

use crate::countries::Countries;
use crate::energy::{format_power, month_name, Bar, Palette, Source, Subject, Unit, SOURCES};
use crate::info::{Card, Meter, Meters, Segment as Part};
use crate::owid::{Owid, Reader};
use crate::world_energy::Figures;
use crate::world_output::{year_month, Estimate};
use crate::world_states::States;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const CREDIT: &str = "Plants outside Germany: Global Energy Monitor, Global Integrated Power Tracker, Sept. 2026 (CC BY 4.0), modified";

const MAGIC: &[u8; 8] = b"ORBW\x01\0\0\0";
const OPEN: u16 = u16::MAX;
const WIKI: &str = "https://www.gem.wiki/";
/// Size of the cells sites are sorted into, in degrees.
const CELL: f64 = 0.5;

pub struct Plant {
    /// Index into the countries' codes.
    pub country: u16,
    /// Radians.
    pub lat: f64,
    pub lon: f64,
    pub source: Source,
    /// Index into GEM's types.
    pub kind: u8,
    pub name: String,
    /// Index into the owners plus one; 0 for none.
    pub owner: u32,
    /// Index into the details (technology and fuel) plus one; 0 for none.
    pub detail: u32,
    /// The gem.wiki page after its address; empty for none.
    pub wiki: String,
    /// Units without a start year count from the release on, and come first.
    pub units: Vec<Unit>,
    /// How many of the units have no start year.
    pub unknown_start: u16,
}

impl Plant {
    pub fn kw(&self, month: u16) -> f32 {
        self.units.iter().filter(|u| running(u, month)).map(|u| u.kw).sum()
    }

    pub fn running(&self, month: u16) -> usize {
        self.units.iter().filter(|u| running(u, month)).count()
    }
}

fn running(u: &Unit, month: u16) -> bool {
    u.start <= month && (u.end == OPEN || month < u.end)
}

pub struct Plants {
    /// The release: year, month, day.
    pub release: (i64, i64, i64),
    codes: Vec<String>,
    /// Nationally aggregated distributed (<1 MW) solar by country index, in kW, which GEM lists
    /// without positions: not plants. This release has none, so the estimates leave it out; its
    /// generation falls in the gap that no plant accounts for.
    #[cfg_attr(not(test), allow(dead_code))]
    distributed: Vec<(u16, f32)>,
    kinds: Vec<String>,
    details: Vec<String>,
    owners: Vec<String>,
    pub plants: Vec<Plant>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// The latest month's estimates, kept while the clock stays in it.
    estimates: Mutex<Option<Arc<Estimates>>>,
}

/// The countries' estimates for one month, by country index; `None` without Ember's figures.
pub struct Estimates {
    pub month: u16,
    pub countries: Vec<Option<Estimate>>,
}

impl Plants {
    /// The plants built into the program.
    pub fn bundled() -> Option<&'static Plants> {
        static DATA: std::sync::OnceLock<Option<Plants>> = std::sync::OnceLock::new();
        DATA.get_or_init(|| Plants::parse(include_bytes!("../data/plants_world.bin"))).as_ref()
    }

    pub fn parse(data: &[u8]) -> Option<Plants> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(8)? != MAGIC {
            return None;
        }
        let date = r.u32()? as i64;
        let release = (date / 10_000, date / 100 % 100, date % 100);
        let release_month = ((release.0 - 1900) * 12 + release.1 - 1).clamp(0, OPEN as i64 - 1) as u16;
        let strings = |r: &mut Reader, n: usize| (0..n).map(|_| r.string()).collect::<Option<Vec<String>>>();
        let n = r.u16()? as usize;
        let codes = strings(&mut r, n)?;
        let mut distributed = Vec::new();
        for _ in 0..r.u16()? {
            let c = u16::try_from(r.varint()?).ok().filter(|&c| (c as usize) < codes.len())?;
            distributed.push((c, r.varint()? as f32 * 100.0));
        }
        let n = r.u16()? as usize;
        let kinds = strings(&mut r, n)?;
        let n = r.u16()? as usize;
        let details = strings(&mut r, n)?;
        let n = r.u32()? as usize;
        if n > data.len() {
            return None;
        }
        let owners = strings(&mut r, n)?;
        let n = r.u32()? as usize;
        if n > data.len() {
            return None;
        }
        let mut plants = Vec::with_capacity(n);
        let (mut lat, mut lon) = (0i64, 0i64);
        for _ in 0..n {
            let country = u16::try_from(r.varint()?).ok().filter(|&c| (c as usize) < codes.len())?;
            lat += r.varint()?;
            lon += r.varint()?;
            let source = *Source::ALL.get(r.u8()? as usize)?;
            let kind = r.u8()?;
            if kind as usize >= kinds.len() {
                return None;
            }
            let name = r.string()?;
            let owner = u32::try_from(r.varint()?).ok().filter(|&o| o as usize <= owners.len())?;
            let detail = u32::try_from(r.varint()?).ok().filter(|&d| d as usize <= details.len())?;
            let wiki = r.string()?;
            let count = usize::try_from(r.varint()?).ok().filter(|&c| c <= data.len())?;
            let mut units = Vec::with_capacity(count);
            let mut unknown_start = 0u16;
            for _ in 0..count {
                let start = r.varint()?;
                let ran = r.varint()?;
                let kw = r.varint()? as f32 * 100.0;
                let unit = if start == 0 {
                    unknown_start += 1;
                    Unit { start: release_month, end: OPEN, kw }
                } else {
                    let start = u16::try_from(start - 1).ok().filter(|&m| m < OPEN)?;
                    let end = if ran == 0 { OPEN } else { u16::try_from(start as i64 + ran - 1).ok().filter(|&m| m < OPEN)? };
                    Unit { start, end, kw }
                };
                units.push(unit);
            }
            let deg = |v: i64| (v as f64 * 1e-4).to_radians();
            plants.push(Plant { country, lat: deg(lat), lon: deg(lon), source, kind, name, owner, detail, wiki, units, unknown_start });
        }
        if r.at != data.len() {
            return None;
        }
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in plants.iter().enumerate() {
            cells.entry(cell_of(p.lat, p.lon)).or_default().push(i as u32);
        }
        Some(Plants { release, codes, distributed, kinds, details, owners, plants, cells, estimates: Mutex::new(None) })
    }

    pub fn count(&self) -> usize {
        self.plants.len()
    }

    /// The country code of a plant, as in `countries.rs`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn code(&self, i: u32) -> &str {
        self.plants.get(i as usize).map_or("", |p| &self.codes[p.country as usize])
    }

    /// The countries' nationally aggregated distributed solar, in kW: code and capacity.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn distributed(&self) -> impl Iterator<Item = (&str, f32)> {
        self.distributed.iter().map(|&(c, kw)| (self.codes[c as usize].as_str(), kw))
    }

    /// The capacity of each country's plants running at `month`, by source, in kW.
    pub fn country_capacity(&self, month: u16) -> Vec<[f32; SOURCES]> {
        let mut out = vec![[0.0; SOURCES]; self.codes.len()];
        for p in &self.plants {
            out[p.country as usize][p.source as usize] += p.kw(month);
        }
        out
    }

    /// The countries' estimates at `month`, worked out once per month.
    pub fn estimates(&self, month: u16) -> Arc<Estimates> {
        let mut kept = self.estimates.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = kept.as_ref().filter(|e| e.month == month) {
            return e.clone();
        }
        let owid = Owid::bundled();
        let capacity = self.country_capacity(month);
        let countries = self
            .codes
            .iter()
            .zip(capacity)
            .map(|(code, cap)| {
                let f = Figures::of(owid?.country(code)?, year_month(month))?;
                Some(Estimate::of(&f, cap))
            })
            .collect();
        let e = Arc::new(Estimates { month, countries });
        *kept = Some(e.clone());
        e
    }

    /// A country's estimate at `month`, by its code; `None` where it has no plants in the list or
    /// Ember no figures.
    pub fn estimate(&self, code: &str, month: u16) -> Option<Estimate> {
        let c = self.codes.iter().position(|c| c == code)?;
        self.estimates(month).countries[c].clone()
    }

    /// The estimated output of `plants` by source at `month`, in kW.
    pub fn output(&self, plants: &[u32], month: u16) -> [f32; SOURCES] {
        let e = self.estimates(month);
        let mut kw = [0.0; SOURCES];
        for &i in plants {
            let p = &self.plants[i as usize];
            let f = e.countries[p.country as usize].as_ref().map_or(0.0, |e| e.factor_of(p.source));
            kw[p.source as usize] += p.kw(month) * f;
        }
        kw
    }

    /// The bars of the sites at `month`, with the sources in `shown`, within a cap of (lat, lon,
    /// radius in radians) around a point if `near`.
    pub fn bars(&self, month: u16, shown: [bool; SOURCES], near: Option<(f64, f64, f64)>) -> Vec<Bar> {
        let mut bars = Vec::new();
        let estimates = self.estimates(month);
        let mut push = |i: u32| {
            let p = &self.plants[i as usize];
            if !shown[p.source as usize] || near.is_some_and(|(lat, lon, r)| angle_between(p.lat, p.lon, lat, lon) > r) {
                return;
            }
            let kw = p.kw(month);
            if kw > 0.0 {
                let mut v = [0.0; SOURCES];
                v[p.source as usize] = kw;
                let e = estimates.countries[p.country as usize].as_ref();
                let running = Some(e.map_or(0.0, |e| e.factor_of(p.source)));
                let faint = e.is_some_and(|e| matches!(e.basis, crate::world_energy::Basis::Year(_)));
                bars.push(Bar { lat: p.lat, lon: p.lon, kw: v, subject: Subject::Plant(i), demand: 0.0, faint, running });
            }
        };
        match near {
            Some((lat, lon, r)) => {
                let wide = r / lat.cos().max(0.05);
                let (lo, hi) = (cell_of(lat - r, lon - wide), cell_of(lat + r, lon + wide));
                if hi.1 - lo.1 >= (360.0 / CELL) as i32 || hi.0 - lo.0 >= (180.0 / CELL) as i32 {
                    (0..self.plants.len() as u32).for_each(push);
                } else {
                    let columns = (360.0 / CELL) as i32;
                    for y in lo.0..=hi.0 {
                        for x in lo.1..=hi.1 {
                            // Across the antimeridian, cells wrap round.
                            let x = (x + columns / 2).rem_euclid(columns) - columns / 2;
                            for &i in self.cells.get(&(y, x)).into_iter().flatten() {
                                push(i);
                            }
                        }
                    }
                }
            }
            None => (0..self.plants.len() as u32).for_each(push),
        }
        bars
    }

    /// For each state of `states`, the plants inside its outline (in the state's country).
    pub fn by_state(&self, states: &States) -> Vec<Vec<u32>> {
        let mut out = vec![Vec::new(); states.count()];
        let mut of_country: HashMap<&str, Vec<usize>> = HashMap::new();
        for i in 0..states.count() {
            of_country.entry(states.country(i)).or_default().push(i);
        }
        let bounds: Vec<[f64; 4]> = (0..states.count()).map(|i| states.bounds(i)).collect();
        for (n, p) in self.plants.iter().enumerate() {
            let (lat, lon) = (p.lat.to_degrees(), p.lon.to_degrees());
            let Some(list) = of_country.get(self.codes[p.country as usize].as_str()) else { continue };
            let hit = list.iter().copied().find(|&i| {
                let [s, w, nn, e] = bounds[i];
                lat >= s && lat <= nn && lon >= w && lon <= e && states.contains(i, lat, lon)
            });
            if let Some(i) = hit {
                out[i].push(n as u32);
            }
        }
        out
    }

    /// The bundled plants of each bundled state, worked out once.
    pub fn bundled_by_state() -> Option<&'static Vec<Vec<u32>>> {
        static DATA: std::sync::OnceLock<Option<Vec<Vec<u32>>>> = std::sync::OnceLock::new();
        DATA.get_or_init(|| Some(Plants::bundled()?.by_state(States::bundled()?))).as_ref()
    }

    /// The installed capacity of `plants` by source at `month`, in kW.
    pub fn capacity(&self, plants: &[u32], month: u16) -> [f32; SOURCES] {
        let mut kw = [0.0; SOURCES];
        for &i in plants {
            let p = &self.plants[i as usize];
            kw[p.source as usize] += p.kw(month);
        }
        kw
    }

    /// The name of a plant and its country.
    pub fn title(&self, i: u32) -> String {
        let Some(p) = self.plants.get(i as usize) else { return String::new() };
        match self.country_name(p) {
            Some(c) => format!("{}, {c}", p.name),
            None => p.name.clone(),
        }
    }

    fn country_name(&self, p: &Plant) -> Option<String> {
        let code = &self.codes[p.country as usize];
        Countries::bundled().and_then(|c| c.get(code)).map(|o| o.name.clone())
    }

    /// Where a plant stands, in radians.
    pub fn position(&self, i: u32) -> Option<(f64, f64)> {
        self.plants.get(i as usize).map(|p| (p.lat, p.lon))
    }

    /// GEM's type of a plant, such as "coal".
    pub fn kind(&self, i: u32) -> &str {
        self.plants.get(i as usize).map_or("", |p| &self.kinds[p.kind as usize])
    }

    pub fn owner(&self, i: u32) -> Option<&str> {
        let p = self.plants.get(i as usize)?;
        (p.owner > 0).then(|| self.owners[p.owner as usize - 1].as_str())
    }

    pub fn detail(&self, i: u32) -> Option<&str> {
        let p = self.plants.get(i as usize)?;
        (p.detail > 0).then(|| self.details[p.detail as usize - 1].as_str())
    }

    /// The plant's page on gem.wiki, if GEM gives one.
    pub fn wiki_url(&self, i: u32) -> Option<String> {
        let p = self.plants.get(i as usize)?;
        (!p.wiki.is_empty()).then(|| format!("{WIKI}{}", p.wiki))
    }

    /// What the information box says about a plant at `month`.
    pub fn describe(&self, i: u32, month: u16, palette: Palette) -> Card {
        let mut card = Card::default();
        let Some(p) = self.plants.get(i as usize) else { return card };
        let kw = p.kw(month);
        let all: f32 = p.units.iter().map(|u| u.kw).sum();
        let estimate = self.estimates(month).countries[p.country as usize].clone();
        let cf = estimate.as_ref().map_or(0.0, |e| e.factor_of(p.source));
        let out = kw * cf;
        card.summary = if kw > 0.0 && estimate.is_some() {
            format!("About {} running in {} (est.), of {} installed.", format_power(out), month_name(month), format_power(kw))
        } else if kw > 0.0 {
            format!("{} installed in {}.", format_power(kw), month_name(month))
        } else if p.units.iter().all(|u| u.start > month) {
            format!("Not built yet in {}.", month_name(month))
        } else {
            format!("Not running any more in {}.", month_name(month))
        };
        let colour = palette.colour(p.source);
        card.meters.push(Meters {
            title: String::new(),
            max: all.max(1.0),
            rows: {
                let mut rows = vec![Meter { label: "Installed".to_string(), segments: vec![Part { alpha: 150, hatched: p.source.fossil(), ..Part::solid(kw, colour) }], text: format_power(kw) }];
                if estimate.is_some() && kw > 0.0 {
                    let segments = vec![Part { hatched: p.source.fossil(), ..Part::solid(out, colour) }, Part { alpha: 60, ..Part::solid(kw - out, colour) }];
                    rows.push(Meter { label: "Running (est.)".to_string(), segments, text: format_power(out) });
                }
                rows
            },
            caption: if (all - kw).abs() > 0.5 { format!("Of {} over all its units' years.", format_power(all)) } else { String::new() },
        });
        let kind = self.kind(i);
        let mut kind = kind.chars().next().map_or(String::new(), |c| c.to_uppercase().collect::<String>() + &kind[c.len_utf8()..]);
        if let Some(d) = self.detail(i) {
            kind.push_str(&format!(" ({d})"));
        }
        card.facts.push(("Type".to_string(), kind));
        card.facts.push(("Drawn as".to_string(), p.source.name().to_string()));
        card.facts.push(("Capacity".to_string(), format_power(kw)));
        if let Some(e) = &estimate {
            let period = match e.basis {
                crate::world_energy::Basis::Month(y, m) => crate::calibration::month_words((y, m)),
                crate::world_energy::Basis::Year(y) => format!("{y} on average"),
            };
            card.facts.push(("Output (est.)".to_string(), format_power(out)));
            card.facts.push(("Capacity factor (est.)".to_string(), format!("{:.0}% ({period})", cf * 100.0)));
        }
        let years = |months: Vec<u16>| {
            let lo = months.iter().min().map(|m| 1900 + m / 12);
            let hi = months.iter().max().map(|m| 1900 + m / 12);
            match (lo, hi) {
                (Some(a), Some(b)) if a == b => a.to_string(),
                (Some(a), Some(b)) => format!("{a}\u{2013}{b}"),
                _ => String::new(),
            }
        };
        // The units without a start year come first.
        let known: Vec<u16> = p.units[(p.unknown_start as usize).min(p.units.len())..].iter().map(|u| u.start).collect();
        let mut start = years(known);
        if p.unknown_start > 0 {
            start = if start.is_empty() { "unknown".to_string() } else { format!("{start} (some units unknown)") };
        }
        card.facts.push(("Start".to_string(), start));
        let retired: Vec<u16> = p.units.iter().filter(|u| u.end != OPEN).map(|u| u.end.saturating_sub(12)).collect();
        if !retired.is_empty() {
            card.facts.push(("Retired".to_string(), years(retired)));
        }
        if p.units.len() > 1 {
            card.facts.push(("Units".to_string(), format!("{} ({} running)", p.units.len(), p.running(month))));
        }
        if let Some(o) = self.owner(i) {
            card.facts.push(("Owner".to_string(), o.to_string()));
        }
        card.facts.push(("Country".to_string(), self.country_name(p).unwrap_or_else(|| self.codes[p.country as usize].clone())));
        if let Some(url) = self.wiki_url(i) {
            card.links.push(("gem.wiki page".to_string(), url));
        }
        card.notes.push("Installed capacity of the units of one source at one GEM location.".to_string());
        match &estimate {
            Some(e) => {
                let capped = if e.factor_of(p.source) >= 1.0 { " Capped at 100%: the plants in the list cannot make all of Ember's figure, the rest counts as not in the plant list." } else { "" };
                card.notes.push(format!("Output estimated: the capacity times the country's capacity factor for {} in the month, Ember's generation (via Our World in Data) over the capacity Global Energy Monitor lists for the country running then, at most 100%. The same for every plant of the source in the country.{capped}", p.source.name().to_lowercase()));
            }
            None => card.notes.push("Ember has no generation for the country at this time: no output estimate.".to_string()),
        }
        if p.unknown_start > 0 {
            card.notes.push(format!("GEM gives no start year for {} of the units: they count from the release ({}) on.", p.unknown_start, month_name(self.release_month())));
        }
        card.notes.push("Global Energy Monitor, Global Integrated Power Tracker, September 2026 release (CC BY 4.0); filtered by status and size, units merged into sites.".to_string());
        card
    }

    fn release_month(&self) -> u16 {
        ((self.release.0 - 1900) * 12 + self.release.1 - 1).clamp(0, OPEN as i64 - 1) as u16
    }
}

fn cell_of(lat: f64, lon: f64) -> (i32, i32) {
    ((lat.to_degrees() / CELL).floor() as i32, (lon.to_degrees() / CELL).floor() as i32)
}

fn angle_between(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let h = ((lat2 - lat1) / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * h.sqrt().min(1.0).asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by `gen_world_plants.py tests/fixtures/gem/plants.csv tests/fixtures/gem/plants_world.bin`.
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/gem/plants_world.bin");

    fn fixture() -> Plants {
        Plants::parse(FIXTURE).expect("fixture parses")
    }

    fn find<'a>(p: &'a Plants, name: &str) -> (u32, &'a Plant) {
        let i = p.plants.iter().position(|x| x.name == name).unwrap_or_else(|| panic!("no {name}"));
        (i as u32, &p.plants[i])
    }

    const fn month(y: u16, m: u16) -> u16 {
        (y - 1900) * 12 + m - 1
    }

    #[test]
    fn fixture_keeps_operating_mothballed_and_retired_only() {
        let p = fixture();
        assert_eq!(p.release, (2026, 9, 1));
        let names: Vec<&str> = p.plants.iter().map(|x| x.name.as_str()).collect();
        for gone in ["Planned Solar Park", "Cancelled Coal", "Building Wind", "Shelved Gas", "Retired Unknown Year", "Tiny Solar", "Small Far Wind", "TZ Solar"] {
            assert!(!names.contains(&gone), "{gone} should be left out: {names:?}");
        }
        for kept in ["Kraftwerk Wien", "Alpen Wasser", "Pas Solar", "Old Coal Station", "Mothballed Gas", "North Sea Offshore", "Iowa Wind", "Big Far Solar"] {
            assert!(names.contains(&kept), "{kept} missing: {names:?}");
        }
        // Germany comes from MaStR.
        assert!((0..p.count() as u32).all(|i| p.code(i) != "DEU"));
        assert!(!names.contains(&"Berlin Gas"));
    }

    #[test]
    fn types_map_onto_sources() {
        let p = fixture();
        let source = |n: &str| find(&p, n).1.source;
        assert_eq!(source("Pas Solar"), Source::Solar);
        assert_eq!(source("Iowa Wind"), Source::WindOnshore);
        assert_eq!(source("North Sea Offshore"), Source::WindOffshore);
        assert_eq!(source("Alpen Wasser"), Source::Hydro);
        assert_eq!(source("Old Coal Station"), Source::HardCoal);
        assert_eq!(source("Brown Coal Plant"), Source::Lignite);
        assert_eq!(source("Kraftwerk Wien"), Source::Gas);
        assert_eq!(source("Diesel Island"), Source::Oil);
        assert_eq!(source("Steel Works Gas"), Source::OtherFossil);
        assert_eq!(source("Reactor One"), Source::Nuclear);
        assert_eq!(source("Hot Springs"), Source::Biomass);
        assert_eq!(source("Wood Power"), Source::Biomass);
    }

    #[test]
    fn units_of_one_location_make_one_site_with_years() {
        let p = fixture();
        let (i, coal) = find(&p, "Old Coal Station");
        assert_eq!(coal.units.len(), 2);
        // Unit 1: 1970 to its retirement in 2015; unit 2 from 1980 on.
        assert_eq!(coal.kw(month(1975, 6)), 200_000.0);
        assert_eq!(coal.kw(month(2015, 12)), 450_000.0);
        assert_eq!(coal.kw(month(2016, 1)), 250_000.0);
        assert_eq!(coal.kw(month(1960, 1)), 0.0);
        assert_eq!(p.owner(i), Some("Coal Co; Other Co"));
        assert_eq!(p.wiki_url(i).as_deref(), Some("https://www.gem.wiki/Old_Coal_Station"));
        let card = p.describe(i, month(2020, 1), Palette::Standard);
        assert!(card.facts.iter().any(|f| f.0 == "Start" && f.1 == "1970\u{2013}1980"), "{:?}", card.facts);
        assert!(card.facts.iter().any(|f| f.0 == "Retired" && f.1 == "2015"), "{:?}", card.facts);
        assert!(card.facts.iter().any(|f| f.0 == "Owner" && f.1 == "Coal Co; Other Co"));
        assert!(card.facts.iter().any(|f| f.0 == "Capacity" && f.1 == format_power(250_000.0)));
        assert_eq!(card.links.len(), 1);
        // A unit without a start year counts from the release on.
        let (j, gas) = find(&p, "Mothballed Gas");
        assert_eq!(gas.unknown_start, 1);
        assert_eq!(gas.kw(month(2026, 8)), 0.0);
        assert_eq!(gas.kw(month(2026, 9)), 50_000.0);
        assert!(p.describe(j, month(2026, 10), Palette::Standard).facts.iter().any(|f| f.0 == "Start" && f.1 == "unknown"));
    }

    #[test]
    fn distributed_solar_is_kept_per_country_not_as_plants() {
        let p = fixture();
        let d: Vec<(&str, f32)> = p.distributed().collect();
        assert_eq!(d, vec![("FRA", 1_234_500.0)]);
        assert!(!p.plants.iter().any(|x| x.name.contains("distributed")));
    }

    #[test]
    fn bars_near_a_point_and_across_the_antimeridian() {
        let p = fixture();
        let all = [true; SOURCES];
        let now = month(2025, 1);
        let every = p.bars(now, all, None);
        assert!(every.iter().all(|b| matches!(b.subject, Subject::Plant(_)) && b.total() > 0.0));
        let (vienna, _) = find(&p, "Kraftwerk Wien");
        let near = p.bars(now, all, Some((48.2f64.to_radians(), 16.4f64.to_radians(), 0.01)));
        assert_eq!(near.iter().map(|b| b.subject).collect::<Vec<_>>(), vec![Subject::Plant(vienna)]);
        let (fiji, _) = find(&p, "Diesel Island");
        let east = p.bars(now, all, Some((-17.8f64.to_radians(), (-179.9f64).to_radians(), 0.01)));
        assert!(east.iter().any(|b| b.subject == Subject::Plant(fiji)));
        let mut solar_only = [false; SOURCES];
        solar_only[Source::Solar as usize] = true;
        assert!(p.bars(now, solar_only, None).iter().all(|b| b.kw[Source::Solar as usize] == b.total()));
    }

    #[test]
    fn a_states_capacity_sums_its_plants() {
        let p = fixture();
        let s = States::bundled().unwrap();
        let by = p.by_state(s);
        let state = |iso: &str| (0..s.count()).find(|&i| s.iso(i) == iso).unwrap();
        let w = state("AT-9");
        let (wien, _) = find(&p, "Kraftwerk Wien");
        assert_eq!(by[w], vec![wien]);
        let now = month(2025, 1);
        let iowa = state("US-IA");
        let kw = p.capacity(&by[iowa], now);
        let want: f32 = by[iowa].iter().map(|&i| p.plants[i as usize].kw(now)).sum();
        assert!(by[iowa].len() >= 2);
        assert_eq!(kw.iter().sum::<f32>(), want);
        assert_eq!(kw[Source::WindOnshore as usize], 300_000.0);
        // Offshore wind lies in no state.
        let (sea, _) = find(&p, "North Sea Offshore");
        assert!(by.iter().all(|l| !l.contains(&sea)));
    }

    #[test]
    fn bundled_plants_parse() {
        let p = Plants::bundled().expect("bundled plants parse");
        assert!(p.count() > 40_000, "{}", p.count());
        assert!((0..p.count() as u32).all(|i| p.code(i) != "DEU"));
        let countries = Countries::bundled().unwrap();
        let unknown: Vec<&str> = (0..p.count() as u32).map(|i| p.code(i)).filter(|c| countries.get(c).is_none()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        eprintln!("codes not in countries.bin: {unknown:?}");
        assert!(unknown.len() <= 2, "codes not in countries.bin: {unknown:?}");
        // Some large, well-known plants.
        let kw = |name: &str| p.plants.iter().filter(|x| x.name.starts_with(name)).map(|x| x.kw(month(2025, 1))).sum::<f32>();
        assert!(kw("Three Gorges") > 15e6, "{}", kw("Three Gorges"));
        assert!(kw("Kashiwazaki") > 5e6, "{}", kw("Kashiwazaki"));
    }

    #[test]
    fn bundled_states_get_their_plants() {
        let t = std::time::Instant::now();
        let p = Plants::bundled().unwrap();
        let s = States::bundled().unwrap();
        let by = Plants::bundled_by_state().unwrap();
        eprintln!("plants by state in {:?}", t.elapsed());
        let placed: usize = by.iter().map(Vec::len).sum();
        assert!(placed as f64 > 0.85 * p.count() as f64, "{placed} of {}", p.count());
        let hubei = (0..s.count()).find(|&i| s.iso(i) == "CN-HB").unwrap();
        let kw = s.capacity(hubei, (2025, 1), [true; SOURCES]).unwrap();
        assert!(kw[Source::Hydro as usize] > 22e6, "{}", kw[Source::Hydro as usize]);
        assert_eq!(kw, p.capacity(&by[hubei], month(2025, 1)));
    }

    /// The height of a lone bar drawn on flat ground, in metres.
    fn drawn_height(bar: &Bar, scale: crate::energy::Scale, shares: Option<&[f32; SOURCES]>) -> f64 {
        let base = crate::geo::geodetic_to_ecef(bar.lat, bar.lon, 0.0);
        let up = crate::geo::enu(bar.lat, bar.lon).2;
        let m = crate::energy::mesh(std::slice::from_ref(bar), scale, base, &|_, _| 0.0, shares, &crate::energy::Style::default()).mesh;
        m.vertices.iter().map(|v| glam::DVec3::new(v.pos[0] as f64, v.pos[1] as f64, v.pos[2] as f64).dot(up)).fold(0.0, f64::max)
    }

    #[test]
    fn equal_capacity_stands_equally_tall_in_germany_and_beyond() {
        use crate::energy::{Energy, Level, Scale};
        let Some(e) = Energy::bundled() else { return };
        let now = month(2026, 1);
        let all = [true; SOURCES];
        let p = Plants::bundled().unwrap();
        // A German site and a Polish plant of the Plants level, the latter given the former's capacity.
        let de = e.bars(Level::Plants, now, all, Some((51.0f64.to_radians(), 10.0f64.to_radians(), 0.05))).into_iter().find(|b| matches!(b.subject, Subject::Site(_))).unwrap();
        let mut pl = p.bars(now, all, Some((52.0f64.to_radians(), 19.0f64.to_radians(), 0.1))).into_iter().next().unwrap();
        pl.kw = de.kw;
        let scale = Scale::for_view(e, Level::Plants, 50_000.0, 800.0);
        let (a, b) = (drawn_height(&de, scale, None), drawn_height(&pl, scale, None));
        assert!(a > 0.0 && (a / b - 1.0).abs() < 1e-3, "German site {a} m, world plant {b} m");
        // 1 GW, either side of the border.
        let mut gw = [0.0; SOURCES];
        gw[Source::Gas as usize] = 1e6;
        let (mut de, mut pl) = (de, pl);
        de.kw = gw;
        pl.kw = gw;
        assert!((drawn_height(&de, scale, None) / drawn_height(&pl, scale, None) - 1.0).abs() < 1e-3);
        assert!((drawn_height(&pl, scale, None) / (1e6 * scale.per_kw) - 1.0).abs() < 1e-3);
        // With SMARD's shares, Germany's columns split into running and idle parts of the same height.
        let shares = [0.3; SOURCES];
        assert!((drawn_height(&de, scale, Some(&shares)) / drawn_height(&pl, scale, Some(&shares)) - 1.0).abs() < 1e-3);
        // The States level: a German state's bar and a world state's of the same power.
        let s = States::bundled().unwrap();
        let mut german = e.bars(Level::States, now, all, None).into_iter().next().unwrap();
        let mut world = s.bars((2026, 1), all, false).into_iter().next().unwrap();
        german.kw = gw;
        world.kw = gw;
        let scale = Scale::for_view(e, Level::States, 2_000_000.0, 800.0);
        assert!((drawn_height(&german, scale, None) / drawn_height(&world, scale, None) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn rejects_other_files() {
        assert!(Plants::parse(b"ORBS\x01\0\0\0").is_none());
        assert!(Plants::parse(&FIXTURE[..FIXTURE.len() - 1]).is_none());
    }

    #[test]
    fn world_plants_run_at_their_countrys_factor_and_say_so() {
        let p = Plants::bundled().unwrap();
        let m = month(2025, 6);
        let bars = p.bars(m, [true; SOURCES], None);
        assert!(bars.iter().all(|b| b.running.is_some_and(|r| (0.0..=1.0).contains(&r))));
        let i = (0..p.count() as u32).find(|&i| p.code(i) == "FRA" && p.plants[i as usize].source == Source::Nuclear && p.plants[i as usize].kw(m) > 0.0).unwrap();
        let e = p.estimate("FRA", m).unwrap();
        let b = bars.iter().find(|b| b.subject == Subject::Plant(i)).unwrap();
        assert_eq!(b.running, Some(e.factor_of(Source::Nuclear)));
        let card = p.describe(i, m, Palette::Standard);
        assert!(card.facts.iter().any(|f| f.0 == "Capacity factor (est.)" && f.1.starts_with(&format!("{:.0}%", e.factor_of(Source::Nuclear) * 100.0))), "{:?}", card.facts);
        assert!(card.facts.iter().any(|f| f.0 == "Output (est.)"));
        assert!(card.meters[0].rows.iter().any(|r| r.label == "Running (est.)"));
        // The country's card tells the plants' share and what is not in the list; Germany's not.
        let w = crate::world_energy::World::bundled().unwrap();
        let card_of = |code: &str| {
            let k = (0..w.count() as u16).find(|&k| w.code(k) == Some(code)).unwrap();
            w.describe(k, (2025, 6), Palette::Standard, false)
        };
        let fra = card_of("FRA");
        assert!(fra.facts.iter().any(|f| f.0 == "Plants account for"), "{:?}", fra.facts);
        assert!(fra.facts.iter().any(|f| f.0 == "Not in plant list"));
        let deu = card_of("DEU");
        assert!(!deu.facts.iter().any(|f| f.0 == "Plants account for"));
    }
}
