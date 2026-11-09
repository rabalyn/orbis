//! Renewable generating capacity in Germany, from the Bundesnetzagentur's Marktstammdatenregister,
//! shown as bars on the globe. `tools/gen_energy.py` turns the register's export and BKG's
//! administrative areas into `data/energy_de.bin`; this module reads it, sums capacity over areas
//! for any month, and builds the bars.
//!
//! Each bar stands for a clearly bounded thing, and the overlay outlines it: a state, district or
//! municipality (square bars, area outlined on the ground), a plant site with a known position
//! (round bars, ringed at the foot), or, close up, the units of a municipality that the register
//! publishes without a position (square bars, municipality outlined).

use crate::buildings::{BuildingMesh, BuildingVertex};
use crate::geo::{enu, geodetic_to_ecef};
use crate::import::{Imported, Item, Mark, Outline, Pos, Shape};
use crate::info::{Card, Meter, Meters, Segment as Part};
use crate::sky_time::civil_from_days;
use glam::DVec3;
use std::collections::HashMap;

pub const SOURCES: usize = 11;
/// Sources from this one on are fossil or nuclear, drawn as a second column beside the renewable one.
pub const FIRST_FOSSIL: usize = 5;

/// The credit while the bars are shown.
pub const CREDIT: &str = "Plants: Marktstammdatenregister, Bundesnetzagentur, dl-de/by-2-0 (summed)\nAreas and population: © BKG (2026) dl-de/by-2-0 (simplified, summed), Datenquellen: https://sgx.geodatenzentrum.de/web_public/gdz/datenquellen/datenquellen_vg_nuts.pdf";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Solar,
    WindOnshore,
    WindOffshore,
    Hydro,
    Biomass,
    Lignite,
    HardCoal,
    Gas,
    Oil,
    OtherFossil,
    Nuclear,
}

impl Source {
    /// In the order they are stacked, from the ground up.
    pub const ALL: [Source; SOURCES] = [
        Source::Solar,
        Source::WindOnshore,
        Source::WindOffshore,
        Source::Hydro,
        Source::Biomass,
        Source::Lignite,
        Source::HardCoal,
        Source::Gas,
        Source::Oil,
        Source::OtherFossil,
        Source::Nuclear,
    ];

    pub fn fossil(self) -> bool {
        self as usize >= FIRST_FOSSIL
    }

    pub fn name(self) -> &'static str {
        match self {
            Source::Solar => "Solar",
            Source::WindOnshore => "Wind on land",
            Source::WindOffshore => "Wind at sea",
            Source::Hydro => "Hydro",
            Source::Biomass => "Biomass",
            Source::Lignite => "Lignite",
            Source::HardCoal => "Hard coal",
            Source::Gas => "Natural gas",
            Source::Oil => "Oil",
            Source::OtherFossil => "Other fossil",
            Source::Nuclear => "Nuclear",
        }
    }

    pub fn colour(self) -> [u8; 3] {
        match self {
            Source::Solar => [242, 178, 36],
            Source::WindOnshore => [70, 150, 235],
            Source::WindOffshore => [40, 70, 190],
            Source::Hydro => [40, 200, 200],
            Source::Biomass => [95, 175, 60],
            Source::Lignite => [135, 90, 55],
            Source::HardCoal => [70, 70, 76],
            Source::Gas => [230, 115, 85],
            Source::Oil => [120, 70, 125],
            Source::OtherFossil => [160, 160, 160],
            Source::Nuclear => [205, 40, 65],
        }
    }
}

/// How finely capacity is summed: the bars at one time all are of one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// The countries of the world, from Our World in Data (`world_energy.rs`).
    Countries,
    States,
    Districts,
    Towns,
    Plants,
}

/// Camera altitudes where the level changes, from coarse to fine.
/// A continent in view shows its countries; one country filling the screen shows its states;
/// districts and municipalities need a closer view.
const LEVEL_ALTITUDES: [f64; 4] = [1_600e3, 300e3, 80e3, 15e3];
/// A level is kept until the altitude is this much past its limit, so that it does not flicker.
const HYSTERESIS: f64 = 1.2;

impl Level {
    pub const ALL: [Level; 5] = [Level::Countries, Level::States, Level::Districts, Level::Towns, Level::Plants];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The level for a camera at `altitude`, given the one shown so far.
    pub fn for_altitude(altitude: f64, previous: Option<Level>) -> Level {
        let plain = |a: f64| Level::ALL[LEVEL_ALTITUDES.iter().filter(|&&l| a < l).count()];
        let Some(prev) = previous else { return plain(altitude) };
        let level = plain(altitude);
        // Only leave the previous level once well past its limits.
        let i = prev.index();
        let upper = if i == 0 { f64::INFINITY } else { LEVEL_ALTITUDES[i - 1] * HYSTERESIS };
        let lower = if i == LEVEL_ALTITUDES.len() { 0.0 } else { LEVEL_ALTITUDES[i] / HYSTERESIS };
        if altitude < upper && altitude >= lower {
            prev
        } else {
            level
        }
    }
}

/// How much detail the bars show: following the camera's altitude, or held at one level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    Auto,
    Fixed(Level),
}

impl Detail {
    pub fn key(self) -> &'static str {
        match self {
            Detail::Auto => "auto",
            Detail::Fixed(Level::Countries) => "countries",
            Detail::Fixed(Level::States) => "states",
            Detail::Fixed(Level::Districts) => "districts",
            Detail::Fixed(Level::Towns) => "municipalities",
            Detail::Fixed(Level::Plants) => "plants",
        }
    }

    pub fn parse(key: &str) -> Option<Detail> {
        Some(match key {
            "auto" => Detail::Auto,
            "countries" => Detail::Fixed(Level::Countries),
            "states" => Detail::Fixed(Level::States),
            "districts" => Detail::Fixed(Level::Districts),
            "municipalities" => Detail::Fixed(Level::Towns),
            "plants" => Detail::Fixed(Level::Plants),
            _ => return None,
        })
    }

    /// The level shown for a camera at `altitude`, given the one shown so far.
    pub fn level(self, altitude: f64, previous: Option<Level>) -> Level {
        match self {
            Detail::Auto => Level::for_altitude(altitude, previous),
            Detail::Fixed(level) => level,
        }
    }
}

/// The altitude bands of the automatic detail, in words.
pub fn auto_bands() -> String {
    let km = |m: f64| format!("{:.0} km", m / 1000.0);
    format!(
        "Countries above {}, Germany's states down to {}, districts down to {}, municipalities down to {}, plant sites below.",
        km(LEVEL_ALTITUDES[0]),
        km(LEVEL_ALTITUDES[1]),
        km(LEVEL_ALTITUDES[2]),
        km(LEVEL_ALTITUDES[3])
    )
}

impl Level {
    /// The name of the level in the plural, for the detail control.
    pub fn plural(self) -> &'static str {
        match self {
            Level::Countries => "Countries",
            Level::States => "States",
            Level::Districts => "Districts",
            Level::Towns => "Municipalities",
            Level::Plants => "Plant sites",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaKind {
    /// A state, district or municipality.
    Admin,
    /// The North Sea or the Baltic: the outline round its offshore wind parks.
    Sea,
    /// An offshore wind park: the outline round its turbines.
    Park,
}

#[derive(Clone, Debug)]
pub struct Area {
    pub level: Level,
    pub kind: AreaKind,
    pub parent: Option<u32>,
    pub name: String,
    /// Where its bar stands, inside the area; radians.
    pub label: (f64, f64),
    /// Outer rings and holes, in radians.
    pub rings: Vec<Vec<(f64, f64)>>,
}

/// Capacity in kW as a step function of the month, months counted from January 1900.
#[derive(Clone, Debug, Default, PartialEq)]
struct Series {
    /// From each month on, the capacity is the second value, until the next step.
    steps: Vec<(u16, f32)>,
}

impl Series {
    fn from_changes(mut changes: Vec<(u16, f32)>) -> Series {
        changes.sort_by_key(|c| c.0);
        let mut steps: Vec<(u16, f32)> = Vec::new();
        let mut total = 0.0f64;
        let mut acc = std::collections::BTreeMap::new();
        for (m, d) in changes {
            *acc.entry(m).or_insert(0.0f64) += d as f64;
        }
        for (m, d) in acc {
            total += d;
            steps.push((m, total.max(0.0) as f32));
        }
        Series { steps }
    }

    fn at(&self, month: u16) -> f32 {
        let i = self.steps.partition_point(|s| s.0 <= month);
        if i == 0 {
            0.0
        } else {
            self.steps[i - 1].1
        }
    }
}

/// A month of no end, for units still running.
const OPEN: u16 = u16::MAX;

#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub start: u16,
    pub end: u16,
    pub kw: f32,
}

/// Units of one source at one published position.
#[derive(Clone, Debug)]
pub struct Site {
    pub lat: f64,
    pub lon: f64,
    pub area: u32,
    pub source: Source,
    pub units: Vec<Unit>,
}

impl Site {
    pub fn kw(&self, month: u16) -> f32 {
        self.units.iter().filter(|u| u.start <= month && (u.end == OPEN || month < u.end)).map(|u| u.kw).sum()
    }

    pub fn running(&self, month: u16) -> usize {
        self.units.iter().filter(|u| u.start <= month && (u.end == OPEN || month < u.end)).count()
    }
}

/// What a bar stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Area(u32),
    /// The units of a municipality published without a position.
    Unplaced(u32),
    Site(u32),
    /// A country of `world_energy::World`, by its index there.
    Country(u16),
    /// A state of `world_states::States` beyond Germany, by its index there.
    State(u16),
    /// A plant site of `world_plants::Plants` beyond Germany, by its index there.
    Plant(u32),
}

impl Subject {
    /// Whether its bar stands for installed capacity, of which the running share can be shown.
    /// Countries and the world's states show average power instead; the world's plants carry
    /// their own estimated running share (`Bar::running`).
    pub fn has_capacity(self) -> bool {
        !matches!(self, Subject::Country(_) | Subject::State(_) | Subject::Plant(_))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub lat: f64,
    pub lon: f64,
    pub kw: [f32; SOURCES],
    pub subject: Subject,
    /// The estimated demand in kW, drawn as a slim column beside the others; 0 for none.
    pub demand: f32,
    /// Drawn lighter: a country's average of the year standing in for the month.
    pub faint: bool,
    /// For a world plant: the estimated share of its capacity running, drawn solid with the rest
    /// above it as unused capacity, in place of the shares of Germany's sources.
    pub running: Option<f32>,
    /// For a screen-space group of sites, the capacity-weighted running share of each source.
    /// This keeps grouping faithful when plants from several countries overlap on screen.
    pub running_by_source: Option<[f32; SOURCES]>,
}

impl Bar {
    pub fn total(&self) -> f32 {
        self.kw.iter().sum()
    }

    pub fn renewable(&self) -> f32 {
        self.kw[..FIRST_FOSSIL].iter().sum()
    }

    pub fn fossil(&self) -> f32 {
        self.kw[FIRST_FOSSIL..].iter().sum()
    }

    /// The capacity of its taller column.
    pub fn height(&self) -> f32 {
        self.renewable().max(self.fossil())
    }
}

/// Size of the cells sites are sorted into, in degrees.
const CELL: f64 = 0.1;

pub struct Energy {
    /// The day the register was exported: year, month, day.
    pub exported: (i64, i64, i64),
    pub areas: Vec<Area>,
    pub sites: Vec<Site>,
    /// For each area, everything in it, by source.
    totals: Vec<[Series; SOURCES]>,
    /// For each municipality, its units without a position, by source.
    unplaced: Vec<[Series; SOURCES]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// The largest bar of each level at the export date, for the scale.
    largest: [f32; 5],
    /// The typical width of an area of each level, in metres: the median of the square roots of
    /// their areas. Bars are sized against it, so they keep a fixed size in the landscape.
    widths: [f64; 5],
    /// How many bars each level has with all sources at the export date.
    counts: [usize; 5],
    /// Each area's size in km² (municipalities only; 0 for others).
    km2: Vec<f64>,
    /// The density of units without a position, in kW per km², that gets the full shade.
    dense: f32,
    /// Each area's population (states, districts and municipalities; 0 for others), and all of
    /// Germany's; empty without the population file.
    people: Vec<u32>,
    people_total: f64,
}

pub fn month_of(unix: f64) -> u16 {
    let (y, m, _) = civil_from_days((unix / 86_400.0).floor() as i64);
    ((y - 1900) * 12 + (m - 1)).clamp(0, OPEN as i64 - 1) as u16
}

pub fn month_name(month: u16) -> String {
    const NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    format!("{} {}", NAMES[month as usize % 12], 1900 + month as usize / 12)
}

/// Reads the file `gen_energy.py` writes.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(b)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn varint(&mut self) -> Option<i64> {
        let mut v: u64 = 0;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(((v >> 1) as i64) ^ -((v & 1) as i64));
            }
        }
        None
    }
    fn angle(&mut self) -> Option<f64> {
        Some((self.i32()? as f64 * 1e-6).to_radians())
    }
    fn changes(&mut self) -> Option<Vec<(u16, f32)>> {
        let n = self.u32()? as usize;
        let mut out = Vec::with_capacity(n.min(1 << 16));
        let mut month = 0i64;
        for _ in 0..n {
            month += self.varint()?;
            out.push((u16::try_from(month).ok()?, self.varint()? as f32 / 10.0));
        }
        Some(out)
    }
}

const MAGIC: &[u8; 8] = b"ORBE\x01\0\0\0";
const POPULATION_MAGIC: &[u8; 8] = b"ORBP\x01\0\0\0";

impl Energy {
    /// The data built into the program.
    pub fn bundled() -> Option<&'static Energy> {
        static DATA: std::sync::OnceLock<Option<Energy>> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            let mut e = Energy::parse(include_bytes!("../data/energy_de.bin"))?;
            if !e.attach_population(include_bytes!("../data/population_de.bin")) {
                eprintln!("energy: the population file does not fit the areas; no demand is estimated");
            }
            Some(e)
        })
        .as_ref()
    }

    /// Takes in the population of the areas from `tools/gen_population.py`. Returns whether it
    /// fits: one figure for each area before the seas and parks.
    pub fn attach_population(&mut self, data: &[u8]) -> bool {
        let mut r = Reader { data, at: 0 };
        let read = |r: &mut Reader| -> Option<Vec<u32>> {
            if r.bytes(8)? != POPULATION_MAGIC {
                return None;
            }
            r.u32()?;
            let n = r.u32()? as usize;
            let admin = self.areas.iter().take_while(|a| a.kind == AreaKind::Admin).count();
            if n != admin {
                return None;
            }
            let mut out = vec![0; self.areas.len()];
            for p in out.iter_mut().take(n) {
                *p = u32::try_from(r.varint()?).ok()?;
            }
            Some(out)
        };
        let Some(people) = read(&mut r) else { return false };
        self.people_total = self.areas.iter().zip(&people).filter(|(a, _)| a.level == Level::States).map(|(_, &p)| p as f64).sum();
        self.people = people;
        self.people_total > 0.0
    }

    /// The population of an area, if known.
    pub fn people(&self, area: u32) -> Option<u32> {
        self.people.get(area as usize).copied().filter(|&p| p > 0)
    }

    /// The estimated demand of what a bar stands for, in kW, from Germany's consumption in MW:
    /// its share of the population. For the units of a municipality without a position, the
    /// municipality's; nothing for sites, seas and parks, where hardly anyone lives.
    pub fn demand_kw(&self, subject: Subject, consumption_mw: f32) -> Option<f32> {
        let area = match subject {
            Subject::Area(i) => i,
            Subject::Unplaced(i) => i,
            Subject::Site(_) | Subject::Country(_) | Subject::State(_) | Subject::Plant(_) => return None,
        };
        let p = self.people(area)?;
        Some((consumption_mw as f64 * 1000.0 * p as f64 / self.people_total) as f32)
    }

    pub fn parse(data: &[u8]) -> Option<Energy> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(8)? != MAGIC {
            return None;
        }
        let date = r.u32()? as i64;
        let exported = (date / 10_000, date / 100 % 100, date % 100);
        let area_count = r.u32()? as usize;
        let mut areas = Vec::with_capacity(area_count.min(1 << 16));
        for _ in 0..area_count {
            // The file counts from the states.
            let level = *Level::ALL.get(r.u8()? as usize + 1)?;
            let kind = match r.u8()? {
                0 => AreaKind::Admin,
                1 => AreaKind::Sea,
                _ => AreaKind::Park,
            };
            let parent = Some(r.u32()?).filter(|&p| p != u32::MAX && (p as usize) < area_count);
            let len = r.u8()? as usize;
            let name = String::from_utf8_lossy(r.bytes(len)?).into_owned();
            let label = (r.angle()?, r.angle()?);
            let ring_count = r.u16()? as usize;
            let mut rings = Vec::with_capacity(ring_count);
            for _ in 0..ring_count {
                let n = r.u32()? as usize;
                let (mut lat, mut lon) = (r.i32()? as i64, r.i32()? as i64);
                let mut ring = Vec::with_capacity(n.min(1 << 20));
                ring.push((lat, lon));
                for _ in 1..n {
                    lat += r.varint()? * 10;
                    lon += r.varint()? * 10;
                    ring.push((lat, lon));
                }
                rings.push(ring.into_iter().map(|(a, b)| ((a as f64 * 1e-6).to_radians(), (b as f64 * 1e-6).to_radians())).collect());
            }
            areas.push(Area { level, kind, parent, name, label, rings });
        }
        let mut changes: Vec<[Vec<(u16, f32)>; SOURCES]> = vec![Default::default(); areas.len()];
        let mut unplaced: Vec<[Series; SOURCES]> = vec![Default::default(); areas.len()];
        for _ in 0..r.u32()? {
            let area = r.u32()? as usize;
            let source = r.u8()? as usize;
            let c = r.changes()?;
            if area >= areas.len() || source >= SOURCES {
                return None;
            }
            changes[area][source].extend_from_slice(&c);
            unplaced[area][source] = Series::from_changes(c);
        }
        let site_count = r.u32()? as usize;
        let mut sites = Vec::with_capacity(site_count.min(1 << 22));
        let (mut area, mut lat, mut lon) = (0i64, 0i64, 0i64);
        for _ in 0..site_count {
            area += r.varint()?;
            lat += r.varint()?;
            lon += r.varint()?;
            let source = *Source::ALL.get(r.u8()? as usize)?;
            let a = usize::try_from(area).ok().filter(|&a| a < areas.len())?;
            let n = r.varint()? as usize;
            let mut units = Vec::with_capacity(n.min(1 << 16));
            for _ in 0..n {
                let start = u16::try_from(r.varint()?).ok().filter(|&m| m < OPEN)?;
                let ran = r.varint()?;
                let end = if ran == 0 { OPEN } else { u16::try_from(start as i64 + ran - 1).ok().filter(|&m| m < OPEN)? };
                let unit = Unit { start, end, kw: r.varint()? as f32 / 10.0 };
                changes[a][source as usize].push((unit.start, unit.kw));
                if unit.end != OPEN {
                    changes[a][source as usize].push((unit.end, -unit.kw));
                }
                units.push(unit);
            }
            let deg = |v: i64| (v as f64 * 1e-4).to_radians();
            sites.push(Site { lat: deg(lat), lon: deg(lon), area: a as u32, source, units });
        }
        // Areas come children after parents in no particular order, so changes are passed up
        // from the finest level first.
        for level in [Level::Towns, Level::Districts] {
            for i in 0..areas.len() {
                if areas[i].level == level {
                    if let Some(p) = areas[i].parent {
                        let c = changes[i].clone();
                        for (s, list) in c.into_iter().enumerate() {
                            changes[p as usize][s].extend(list);
                        }
                    }
                }
            }
        }
        let totals: Vec<[Series; SOURCES]> = changes.into_iter().map(|c| c.map(Series::from_changes)).collect();
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, s) in sites.iter().enumerate() {
            cells.entry(cell_of(s.lat, s.lon)).or_default().push(i as u32);
        }
        let km2 = areas.iter().map(|a| if a.level == Level::Towns { area_km2(&a.rings) } else { 0.0 }).collect();
        let mut energy = Energy { exported, areas, sites, totals, unplaced, cells, largest: [1.0; 5], widths: [PLANT_WIDTH; 5], counts: [0; 5], km2, dense: 1.0, people: Vec::new(), people_total: 0.0 };
        for level in [Level::States, Level::Districts, Level::Towns] {
            let mut sides: Vec<f64> = energy
                .areas
                .iter()
                .filter(|a| a.level == level && a.kind == AreaKind::Admin)
                .map(|a| area_km2(&a.rings).sqrt() * 1000.0)
                .collect();
            sides.sort_by(f64::total_cmp);
            if let Some(&median) = sides.get(sides.len() / 2) {
                energy.widths[level.index()] = median;
            }
        }
        let last = ((exported.0 - 1900) * 12 + exported.1 - 1).clamp(0, OPEN as i64 - 1) as u16;
        let all = [true; SOURCES];
        for level in Level::ALL {
            // A few huge bars would flatten all others, so the scale is set by the 98th percentile.
            let mut totals: Vec<f32> = energy.bars(level, last, all, None).iter().map(Bar::height).collect();
            totals.sort_by(f32::total_cmp);
            let at = (totals.len() as f64 * 0.98) as usize;
            energy.largest[level.index()] = totals.get(at.min(totals.len().saturating_sub(1))).copied().unwrap_or(1.0).max(1.0);
            energy.counts[level.index()] = totals.len();
        }
        // Likewise the shade of units without a position.
        let mut densities: Vec<f32> = (0..energy.areas.len() as u32).filter_map(|i| energy.unplaced_density(i, last, all)).filter(|&d| d > 0.0).collect();
        densities.sort_by(f32::total_cmp);
        let at = (densities.len() as f64 * 0.98) as usize;
        energy.dense = densities.get(at.min(densities.len().saturating_sub(1))).copied().unwrap_or(1.0).max(1e-3);
        Some(energy)
    }

    /// The bars of `level` at `month`, with the sources in `shown`. `near` limits them to a cap
    /// of (lat, lon, radius in radians) around a point.
    pub fn bars(&self, level: Level, month: u16, shown: [bool; SOURCES], near: Option<(f64, f64, f64)>) -> Vec<Bar> {
        let close = |lat: f64, lon: f64| near.is_none_or(|(clat, clon, r)| angle_between(lat, lon, clat, clon) <= r);
        let pick = |series: &[Series; SOURCES]| {
            let mut kw = [0.0f32; SOURCES];
            for (i, s) in series.iter().enumerate() {
                if shown[i] {
                    kw[i] = s.at(month);
                }
            }
            kw
        };
        let mut bars = Vec::new();
        if level != Level::Plants {
            for (i, a) in self.areas.iter().enumerate() {
                if a.level == level && close(a.label.0, a.label.1) {
                    bars.push(Bar { lat: a.label.0, lon: a.label.1, kw: pick(&self.totals[i]), subject: Subject::Area(i as u32), demand: 0.0, faint: false, running: None, running_by_source: None });
                }
            }
        } else {
            // Units without a position are not bars here: their municipality is shaded instead.
            let mut push_site = |i: u32| {
                let s = &self.sites[i as usize];
                if shown[s.source as usize] && close(s.lat, s.lon) {
                    let mut kw = [0.0; SOURCES];
                    kw[s.source as usize] = s.kw(month);
                    bars.push(Bar { lat: s.lat, lon: s.lon, kw, subject: Subject::Site(i), demand: 0.0, faint: false, running: None, running_by_source: None });
                }
            };
            match near {
                Some((lat, lon, r)) => {
                    let (lo, hi) = (cell_of(lat - r, lon - r / lat.cos().max(0.05)), cell_of(lat + r, lon + r / lat.cos().max(0.05)));
                    for y in lo.0..=hi.0 {
                        for x in lo.1..=hi.1 {
                            for &i in self.cells.get(&(y, x)).into_iter().flatten() {
                                push_site(i);
                            }
                        }
                    }
                }
                None => (0..self.sites.len() as u32).for_each(push_site),
            }
        }
        // Units the register gives only by district (most large power plants) stand at their district
        // wherever the bars are finer than districts.
        if level >= Level::Towns {
            for (i, a) in self.areas.iter().enumerate() {
                if a.level == Level::Districts && a.kind == AreaKind::Admin && close(a.label.0, a.label.1) {
                    bars.push(Bar { lat: a.label.0, lon: a.label.1, kw: pick(&self.unplaced[i]), subject: Subject::Unplaced(i as u32), demand: 0.0, faint: false, running: None, running_by_source: None });
                }
            }
        }
        bars.retain(|b| b.total() > 0.0);
        bars
    }

    /// The capacity of each source in Germany at `month`, in kW.
    pub fn national(&self, month: u16) -> [f32; SOURCES] {
        let mut out = [0.0; SOURCES];
        for (i, a) in self.areas.iter().enumerate() {
            if a.level == Level::States {
                for (s, series) in self.totals[i].iter().enumerate() {
                    out[s] += series.at(month);
                }
            }
        }
        out
    }

    /// How many bars a level has, with all sources, at the register's date.
    pub fn count(&self, level: Level) -> usize {
        self.counts[level.index()]
    }

    /// The capacity of a tall bar of a level: 98% of the level's bars at the export date, with
    /// all sources, are lower.
    pub fn largest(&self, level: Level) -> f32 {
        self.largest[level.index()]
    }

    /// Capacity of a municipality's units without a position, in kW per km², with the sources in
    /// `shown`; `None` for other areas.
    pub fn unplaced_density(&self, area: u32, month: u16, shown: [bool; SOURCES]) -> Option<f32> {
        let i = area as usize;
        if self.km2.get(i).is_none_or(|&k| k <= 0.0) {
            return None;
        }
        let kw: f32 = self.unplaced[i].iter().zip(shown).filter(|(_, on)| *on).map(|(s, _)| s.at(month)).sum();
        Some((kw as f64 / self.km2[i]) as f32)
    }

    /// The density that gets the full shade, in kW per km²: 98% of municipalities are below it.
    pub fn dense(&self) -> f32 {
        self.dense
    }

    /// The municipality at a position, if any.
    pub fn town_at(&self, lat: f64, lon: f64) -> Option<u32> {
        self.areas.iter().position(|a| a.level == Level::Towns && a.kind == AreaKind::Admin && inside(&a.rings, lat, lon)).map(|i| i as u32)
    }

    /// What is drawn on the ground for the bars of `level`. For areas: every area that has some
    /// of the shown capacity at some time, outlined; these don't follow the clock, so that moving
    /// it does not redraw the ground. Close up: every municipality with units without a position,
    /// outlined and shaded by their capacity per km² at `month`. The selected area is emphasised,
    /// or a selected site ringed.
    pub fn outlines(&self, level: Level, shown: [bool; SOURCES], selected: Option<Subject>, month: u16, palette: Palette) -> Imported {
        let mut items = Vec::new();
        let has = |series: &[Series; SOURCES]| series.iter().zip(shown).any(|(s, on)| on && s.steps.iter().any(|st| st.1 > 0.0));
        let (outline_level, series) = match level {
            Level::Plants => (Level::Towns, &self.unplaced),
            l => (l, &self.totals),
        };
        let chosen_area = match selected {
            Some(Subject::Area(i) | Subject::Unplaced(i)) => Some(i as usize),
            _ => None,
        };
        let chosen = selected.map(|subject| self.selected_outline(subject, month, shown, palette));
        for (i, a) in self.areas.iter().enumerate() {
            if a.level != outline_level || (!has(&series[i]) && chosen_area != Some(i)) {
                continue;
            }
            let rings = a.rings.iter().map(|r| r.iter().map(|&(lat, lon)| Pos { lat, lon, alt: None }).collect()).collect();
            let mut item = Item::new("", Shape::Polygon(rings));
            item.outline = Some(if chosen_area == Some(i) {
                chosen.unwrap_or(SELECTED)
            } else if level == Level::Plants {
                let d = self.unplaced_density(i as u32, month, shown).unwrap_or(0.0);
                let mut o = UNPLACED_EDGE;
                o.fill[3] = ((d / self.dense).clamp(0.0, 1.0) * SHADE_ALPHA as f32).round() as u8;
                o
            } else {
                AREA_EDGE
            });
            items.push(item);
        }
        if level >= Level::Towns {
            for (i, a) in self.areas.iter().enumerate() {
                if a.level == Level::Districts && a.kind == AreaKind::Admin && (has(&self.unplaced[i]) || chosen_area == Some(i)) {
                    let rings = a.rings.iter().map(|r| r.iter().map(|&(lat, lon)| Pos { lat, lon, alt: None }).collect()).collect();
                    let mut item = Item::new("", Shape::Polygon(rings));
                    item.outline = Some(if chosen_area == Some(i) {
                        chosen.unwrap_or(SELECTED)
                    } else {
                        let kw = |r: std::ops::Range<usize>| r.filter(|&s| shown[s]).map(|s| self.unplaced[i][s].at(month)).sum::<f32>();
                        Outline { dashed: kw(FIRST_FOSSIL..SOURCES) > kw(0..FIRST_FOSSIL), ..DISTRICT_ONLY_EDGE }
                    });
                    items.push(item);
                }
            }
        }
        // The chosen one last, so that it is drawn over its neighbours' edges.
        items.sort_by_key(|i| chosen.is_some() && i.outline == chosen);
        if let Some(Subject::Site(i)) = selected {
            let s = &self.sites[i as usize];
            let mut item = Item::new("", Shape::Point(Pos { lat: s.lat, lon: s.lon, alt: None }));
            item.mark = Some(Mark { radius: 9.0, colour: [255, 255, 255, 240] });
            items.push(item);
        }
        Imported { name: "Energy".to_string(), items, truncated: false }
    }

    /// The name of what a bar stands for, with its parents: "Landkreis Rosenheim, Bayern".
    pub fn subject_title(&self, subject: Subject) -> String {
        let chain = |mut i: Option<u32>| {
            let mut names = Vec::new();
            while let Some(a) = i {
                let area = &self.areas[a as usize];
                if names.last() != Some(&area.name) {
                    names.push(area.name.clone());
                }
                i = area.parent;
            }
            names.join(", ")
        };
        match subject {
            Subject::Area(i) => chain(Some(i)),
            Subject::Unplaced(i) if self.areas[i as usize].level == Level::Districts => format!("Units given only by district: {}", chain(Some(i))),
            Subject::Unplaced(i) => format!("Units without a published position in {}", chain(Some(i))),
            Subject::Site(i) => {
                let s = &self.sites[i as usize];
                format!("{} site in {}", s.source.name(), chain(Some(s.area)))
            }
            Subject::Country(_) | Subject::State(_) | Subject::Plant(_) => String::new(),
        }
    }

    /// The capacity of one source in what a bar stands for, in kW.
    fn subject_kw(&self, subject: Subject, source: Source, month: u16) -> f32 {
        match subject {
            Subject::Area(i) => self.totals[i as usize][source as usize].at(month),
            Subject::Unplaced(i) => self.unplaced[i as usize][source as usize].at(month),
            Subject::Site(i) => {
                let site = &self.sites[i as usize];
                if site.source == source {
                    site.kw(month)
                } else {
                    0.0
                }
            }
            Subject::Country(_) | Subject::State(_) | Subject::Plant(_) => 0.0,
        }
    }

    /// The source with the most capacity in what a bar stands for, among those in `shown`.
    pub fn main_source(&self, subject: Subject, month: u16, shown: [bool; SOURCES]) -> Option<Source> {
        Source::ALL
            .into_iter()
            .filter(|&s| shown[s as usize])
            .map(|s| (s, self.subject_kw(subject, s, month)))
            .filter(|&(_, kw)| kw > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s)
    }

    /// The chosen area's outline: filled in the colour of its main source, hatched and with a
    /// dashed edge when that is fossil or nuclear.
    fn selected_outline(&self, subject: Subject, month: u16, shown: [bool; SOURCES], palette: Palette) -> Outline {
        match self.main_source(subject, month, shown) {
            Some(main) => {
                let [r, g, b] = palette.colour(main);
                Outline { fill: [r, g, b, SELECTED_FILL_ALPHA], dashed: main.fossil(), ..SELECTED }
            }
            None => SELECTED,
        }
    }

    /// What the information box says about a bar: the gist, bars that compare what is installed,
    /// what runs and what is used, one bar per source, then what the bar stands for and how far
    /// to trust the figures. With `output`, the estimated output at a time beside the capacity.
    pub fn describe(&self, subject: Subject, month: u16, shown: [bool; SOURCES], palette: Palette, output: Option<At<'_>>) -> Card {
        let mut card = Card::default();
        let share = |s: Source| output.map_or(1.0, |o| o.shares[s as usize].clamp(0.0, 1.0));
        let rows: Vec<(Source, f32)> = Source::ALL
            .into_iter()
            .filter(|s| shown[*s as usize])
            .map(|s| (s, self.subject_kw(subject, s, month)))
            .filter(|&(_, kw)| kw > 0.0)
            .collect();
        let renewable: f32 = rows.iter().filter(|r| !r.0.fossil()).map(|r| r.1).sum();
        let fossil: f32 = rows.iter().filter(|r| r.0.fossil()).map(|r| r.1).sum();
        let running: f32 = rows.iter().map(|&(s, kw)| kw * share(s)).sum();
        let total = renewable + fossil;
        // Clamped, so that a rounding residue never prints as "-0%".
        let pct = |part: f32, of: f32| (100.0 * part / of.max(1e-6)).max(0.0);
        let largest = rows.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1));
        let demand = output.and_then(|o| self.demand_kw(subject, o.consumption_mw));
        let main = largest.map_or(String::new(), |(s, kw)| format!(" Largest: {} ({:.0}%).", s.name().to_lowercase(), pct(kw, total)));
        card.summary = match (total > 0.0, output) {
            (false, _) => format!("No capacity of the shown sources in {}.", month_name(month)),
            (true, Some(o)) => format!("About {} running at {}, of {} installed ({:.0}%).{main}", format_power(running), o.when, format_power(total), pct(running, total)),
            (true, None) => format!("{} installed in {}, {:.0}% renewable.{main}", format_power(total), month_name(month), pct(renewable, total)),
        };
        let colour = |s: Source| palette.colour(s);
        if total > 0.0 {
            // Installed, running and used on one scale.
            let installed: Vec<Part> = rows.iter().map(|&(s, kw)| Part { alpha: 150, hatched: s.fossil(), ..Part::solid(kw, colour(s)) }).collect();
            let mut group = Meters { title: String::new(), max: total.max(demand.unwrap_or(0.0)), rows: Vec::new(), caption: String::new() };
            group.rows.push(Meter { label: "Installed".to_string(), segments: installed, text: format_power(total) });
            if output.is_some() {
                let ran: Vec<Part> = rows.iter().map(|&(s, kw)| Part { hatched: s.fossil(), ..Part::solid(kw * share(s), colour(s)) }).collect();
                group.rows.push(Meter { label: "Running".to_string(), segments: ran, text: format_power(running) });
            }
            if let Some(d) = demand {
                let label = if matches!(subject, Subject::Unplaced(_)) { "Town's use" } else { "Use (est.)" };
                group.rows.push(Meter { label: label.to_string(), segments: vec![Part { ink: true, ..Part::solid(d, DEMAND) }], text: format!("\u{2248} {}", format_power(d)) });
            }
            group.caption = format!("{:.0}% renewable, {:.0}% fossil or nuclear installed", pct(renewable, total), pct(fossil, total));
            if let Some(d) = demand.filter(|&d| d > 0.0) {
                group.caption.push_str(&format!("; running covers {:.0}% of the use", pct(running, d)));
            }
            card.meters.push(group);
            // Each source on its own: output solid, the rest of its capacity faint.
            if rows.len() > 1 || output.is_some() {
                let mut by_source = Meters { title: "By source".to_string(), max: largest.map_or(1.0, |l| l.1), rows: Vec::new(), caption: String::new() };
                for &(s, kw) in &rows {
                    let out = kw * share(s);
                    let mut segments = vec![Part { hatched: s.fossil(), ..Part::solid(out, colour(s)) }];
                    let text = if output.is_some() {
                        segments.push(Part { alpha: 60, ..Part::solid(kw - out, colour(s)) });
                        format!("{} of {}", format_power(out), format_power(kw))
                    } else {
                        format_power(kw)
                    };
                    by_source.rows.push(Meter { label: s.name().to_string(), segments, text });
                }
                if output.is_some() {
                    by_source.caption = "Solid: running now. Faint: the rest of what is installed.".to_string();
                }
                card.meters.push(by_source);
            }
        }
        let area = match subject {
            Subject::Area(i) | Subject::Unplaced(i) => Some(i),
            Subject::Site(_) | Subject::Country(_) | Subject::State(_) | Subject::Plant(_) => None,
        };
        if let Some(p) = area.and_then(|i| self.people(i)) {
            card.facts.push(("Population".to_string(), group_thousands(p)));
        }
        match subject {
            Subject::Unplaced(i) if self.areas[i as usize].level != Level::Districts => {
                let d = self.unplaced_density(i, month, shown).unwrap_or(0.0);
                card.facts.push(("Area".to_string(), format!("{:.1} km²", self.km2[i as usize])));
                card.meters.push(Meters {
                    title: "Shade on the map".to_string(),
                    max: self.dense,
                    rows: vec![Meter {
                        label: "Per km²".to_string(),
                        segments: vec![Part::solid(d.min(self.dense), [255, 80, 200])],
                        text: format!("{} ({:.0}%)", format_power(d), pct(d, self.dense)),
                    }],
                    caption: format!("Full shade at {} per km².", format_power(self.dense)),
                });
            }
            Subject::Site(i) => card.facts.push(("Units running".to_string(), self.sites[i as usize].running(month).to_string())),
            _ => {}
        }
        card.notes.push(match subject {
            Subject::Area(i) => match self.areas[i as usize].kind {
                AreaKind::Admin => format!("All units registered in this {}.", level_noun(self.areas[i as usize].level)),
                AreaKind::Sea => "All offshore wind parks in this sea.".to_string(),
                AreaKind::Park => "All turbines of this offshore wind park.".to_string(),
            },
            Subject::Unplaced(i) if self.areas[i as usize].level == Level::Districts => {
                "Units the register gives only by district, mostly large power plants: one bar for the district, outlined in orange.".to_string()
            }
            Subject::Unplaced(_) => "Units without a published position, mostly private rooftop solar. They shade the municipality instead of standing as a bar.".to_string(),
            Subject::Site(_) => "Units of one source at one published position.".to_string(),
            Subject::Country(_) | Subject::State(_) | Subject::Plant(_) => String::new(),
        });
        if output.is_some() {
            card.notes.push("Output is estimated: Germany's output of each source (SMARD) shared out by capacity.".to_string());
        }
        if demand.is_some() {
            card.notes.push(
                "Use is estimated: Germany's consumption (SMARD) shared out by population (BKG, end of 2025). Industry, about 45% of the total, sits where the factories are, so places with much industry use more than shown."
                    .to_string(),
            );
        }
        card.notes.push(format!(
            "Marktstammdatenregister, {}-{:02}-{:02}: net power of units in operation.",
            self.exported.0, self.exported.1, self.exported.2
        ));
        card
    }
}

/// The moment the information box tells about: the share of each source's capacity running,
/// the time in words, and Germany's consumption then in MW.
#[derive(Clone, Copy, Debug)]
pub struct At<'a> {
    pub shares: &'a [f32; SOURCES],
    pub when: &'a str,
    pub consumption_mw: f32,
}

/// A count with thin spaces between thousands.
pub fn group_thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

fn level_noun(level: Level) -> &'static str {
    match level {
        Level::States => "state",
        Level::Districts => "district",
        Level::Towns => "municipality",
        Level::Plants => "site",
        Level::Countries => "country",
    }
}

pub fn format_power(kw: f32) -> String {
    let kw = kw as f64;
    if kw >= 1e6 {
        format!("{:.2} GW", kw / 1e6)
    } else if kw >= 1e3 {
        format!("{:.1} MW", kw / 1e3)
    } else {
        format!("{kw:.0} kW")
    }
}

pub(crate) const AREA_EDGE: Outline = Outline { edge: [255, 80, 200, 230], fill: [255, 80, 200, 0], width: 1.6, dashed: false };
const UNPLACED_EDGE: Outline = Outline { edge: [255, 80, 200, 170], fill: [255, 80, 200, 0], width: 1.2, dashed: false };
/// Dashed where most of the district's own units are fossil or nuclear.
const DISTRICT_ONLY_EDGE: Outline = Outline { edge: [255, 150, 30, 230], fill: [255, 150, 30, 0], width: 2.0, dashed: false };
/// The opacity of the densest shade.
const SHADE_ALPHA: u8 = 110;
/// The chosen area's edge; it is filled in the colour of its main source, hatched for a fossil one.
pub(crate) const SELECTED: Outline = Outline { edge: [255, 240, 120, 250], fill: [255, 240, 120, 55], width: 2.4, dashed: false };
/// The opacity of the chosen area's fill.
pub(crate) const SELECTED_FILL_ALPHA: u8 = 95;

/// The size of an area in km², for rings in radians whose holes run the other way round.
fn area_km2(rings: &[Vec<(f64, f64)>]) -> f64 {
    let r = 6371.0;
    let mut sum = 0.0;
    for ring in rings {
        let k = (ring.iter().map(|p| p.0).sum::<f64>() / ring.len().max(1) as f64).cos();
        for w in ring.windows(2) {
            let (x0, y0, x1, y1) = (w[0].1 * k * r, w[0].0 * r, w[1].1 * k * r, w[1].0 * r);
            sum += x0 * y1 - x1 * y0;
        }
    }
    (sum / 2.0).abs()
}

/// Even-odd test over all rings, so that holes count.
fn inside(rings: &[Vec<(f64, f64)>], lat: f64, lon: f64) -> bool {
    let mut hit = false;
    for ring in rings {
        for w in ring.windows(2) {
            let ((y0, x0), (y1, x1)) = (w[0], w[1]);
            if (y0 > lat) != (y1 > lat) && lon < x0 + (lat - y0) * (x1 - x0) / (y1 - y0) {
                hit = !hit;
            }
        }
    }
    hit
}

fn cell_of(lat: f64, lon: f64) -> (i32, i32) {
    ((lat.to_degrees() / CELL).floor() as i32, (lon.to_degrees() / CELL).floor() as i32)
}

fn angle_between(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let h = ((lat2 - lat1) / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * h.sqrt().min(1.0).asin()
}

/// The tall bar of a level, as a share of the typical width of its areas.
pub const HEIGHT_SHARE: f64 = 0.8;
/// A bar's width, as a share of the typical width of its level's areas.
const WIDTH_SHARE: f64 = 0.07;
/// Bars seen from far away stay this many pixels wide.
const MIN_WIDTH_PX: f64 = 5.0;
/// What plant sites are sized against, in metres, in place of an area's width.
const PLANT_WIDTH: f64 = 2500.0;

/// The share of the plant bars in view below the one the Plants level is scaled for.
pub const PLANT_PERCENTILE: f64 = 0.98;
/// The least capacity the Plants level is scaled for, in kW, so that a view of a few rooftops
/// does not blow them up.
pub const PLANT_FLOOR_KW: f32 = 1000.0;

/// The height of a kW the plant bars in view need: the [`PLANT_PERCENTILE`] of the heights of
/// the plant bars (Germany's sites and the world's alike) for which `visible` holds, at least
/// [`PLANT_FLOOR_KW`], stands [`HEIGHT_SHARE`] of [`PLANT_WIDTH`] tall, as [`Energy::largest`] does
/// for all of Germany's. `None` with no plant bar in view.
pub fn plants_needed(bars: &[Bar], visible: &dyn Fn(f64, f64) -> bool) -> Option<f64> {
    let mut heights: Vec<f32> = bars.iter().filter(|b| matches!(b.subject, Subject::Site(_) | Subject::Plant(_)) && visible(b.lat, b.lon)).map(Bar::height).filter(|h| *h > 0.0).collect();
    if heights.is_empty() {
        return None;
    }
    let at = ((heights.len() as f64 * PLANT_PERCENTILE) as usize).min(heights.len() - 1);
    let (_, percentile, _) = heights.select_nth_unstable_by(at, f32::total_cmp);
    Some(HEIGHT_SHARE * PLANT_WIDTH / percentile.max(PLANT_FLOOR_KW) as f64)
}

/// Screen-space grouping for the drawing pass. Site bars remain individual in `bars` for
/// picking and counts; only this temporary mesh input is grouped when many sites overlap.
pub fn visual_bars(
    bars: &[Bar],
    size: (f64, f64),
    shares: Option<&[f32; SOURCES]>,
    project: &dyn Fn(f64, f64) -> Option<glam::DVec2>,
) -> Vec<Bar> {
    const MAX_INDIVIDUAL: usize = 12_000;
    const TARGET_CELLS: f64 = 500.0;
    let site_count = bars.iter().filter(|b| matches!(b.subject, Subject::Site(_) | Subject::Plant(_))).count();
    let grouping = site_count > MAX_INDIVIDUAL;
    let cell = ((size.0.max(1.0) * size.1.max(1.0) / TARGET_CELLS).sqrt()).max(8.0);
    let mut out = Vec::with_capacity(if grouping { MAX_INDIVIDUAL } else { bars.len() });
    let mut groups: HashMap<(i32, i32, bool), Vec<(Bar, glam::DVec2)>> = HashMap::new();
    for bar in bars {
        if !matches!(bar.subject, Subject::Site(_) | Subject::Plant(_)) {
            out.push(bar.clone());
            continue;
        }
        let Some(p) = project(bar.lat, bar.lon).filter(|p| p.x >= 0.0 && p.y >= 0.0 && p.x <= size.0 && p.y <= size.1) else { continue };
        if grouping {
            groups.entry(((p.x / cell).floor() as i32, (p.y / cell).floor() as i32, bar.faint)).or_default().push((bar.clone(), p));
        } else {
            out.push(bar.clone());
        }
    }
    for ((_, _, faint), members) in groups {
        if members.len() == 1 {
            out.push(members.into_iter().next().unwrap().0);
            continue;
        }
        let mut kw = [0.0; SOURCES];
        let mut output = [0.0; SOURCES];
        let mut direction = glam::DVec3::ZERO;
        let mut largest = 0.0f32;
        let mut subject = Subject::Plant(0);
        for (bar, _) in &members {
            direction += geodetic_to_ecef(bar.lat, bar.lon, 0.0).normalize();
            if bar.total() > largest {
                largest = bar.total();
                subject = bar.subject;
            }
            for s in 0..SOURCES {
                let capacity = bar.kw[s].max(0.0);
                kw[s] += capacity;
                let factor = bar.running_by_source.map(|r| r[s]).or(bar.running).unwrap_or_else(|| shares.map_or(1.0, |r| r[s])).clamp(0.0, 1.0);
                output[s] += capacity * factor;
            }
        }
        let direction = direction.normalize_or_zero();
        let lat = direction.z.atan2(direction.x.hypot(direction.y));
        let lon = direction.y.atan2(direction.x);
        let running_by_source = Some(std::array::from_fn(|s| if kw[s] > 0.0 { output[s] / kw[s] } else { 0.0 }));
        out.push(Bar { lat, lon, kw, subject, demand: 0.0, faint, running: None, running_by_source });
    }
    out
}

/// How the bars are sized: their width and the height of a kilowatt, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub width: f64,
    pub per_kw: f64,
}

impl Scale {
    /// Bars in proportion to the ground they stand for: the tall bar of a level (see
    /// [`Energy::largest`]) is [`HEIGHT_SHARE`] of the typical width of the level's areas, and a
    /// bar is [`WIDTH_SHARE`] of it wide. They keep that size in the landscape as the camera moves,
    /// only never thinner than [`MIN_WIDTH_PX`] pixels for a camera `distance` from what it looks
    /// at and a view `height_px` tall.
    pub fn for_view(energy: &Energy, level: Level, distance: f64, height_px: f64) -> Scale {
        Scale::for_side(energy.widths[level.index()], HEIGHT_SHARE, energy.largest(level), distance, height_px)
    }

    /// The same for areas `side` metres wide whose tall bar of `largest` kW is `share` times
    /// their width.
    pub fn for_side(side: f64, share: f64, largest: f32, distance: f64, height_px: f64) -> Scale {
        let metres_per_px = 2.0 * distance * (crate::camera::FOV_Y / 2.0).tan() / height_px.max(1.0);
        Scale { width: (WIDTH_SHARE * side).max(MIN_WIDTH_PX * metres_per_px), per_kw: share * side / largest.max(1.0) as f64 }
    }
}

/// The colours of the sources: a set of palettes to choose from in the energy display window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Standard,
    /// After Okabe and Ito: told apart with any common colour vision deficiency.
    ColourBlind,
    /// The standard colours, softened, for a calmer picture over busy imagery.
    Muted,
}

impl Palette {
    pub const ALL: [Palette; 3] = [Palette::Standard, Palette::ColourBlind, Palette::Muted];

    pub fn name(self) -> &'static str {
        match self {
            Palette::Standard => "Standard",
            Palette::ColourBlind => "Colour-blind safe",
            Palette::Muted => "Muted",
        }
    }

    /// The identifier in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Palette::Standard => "standard",
            Palette::ColourBlind => "colour_blind",
            Palette::Muted => "muted",
        }
    }

    pub fn parse(key: &str) -> Option<Palette> {
        Palette::ALL.into_iter().find(|p| p.key() == key)
    }

    pub fn colour(self, source: Source) -> [u8; 3] {
        match self {
            Palette::Standard => source.colour(),
            Palette::ColourBlind => match source {
                Source::Solar => [240, 228, 66],
                Source::WindOnshore => [86, 180, 233],
                Source::WindOffshore => [0, 114, 178],
                Source::Hydro => [0, 158, 115],
                Source::Biomass => [150, 200, 90],
                Source::Lignite => [150, 100, 55],
                Source::HardCoal => [80, 80, 86],
                Source::Gas => [213, 94, 0],
                Source::Oil => [204, 121, 167],
                Source::OtherFossil => [165, 165, 165],
                Source::Nuclear => [230, 159, 0],
            },
            Palette::Muted => source.colour().map(|c| (c as f32 * 0.6 + 190.0 * 0.4) as u8),
        }
    }
}

/// How capacity that is not running at the clock's quarter hour is shown, above the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// In its source's colour, see-through.
    Source,
    /// In white, see-through.
    White,
    /// Not at all: the bars show output only.
    Hidden,
}

impl Idle {
    pub const ALL: [Idle; 3] = [Idle::Source, Idle::White, Idle::Hidden];

    pub fn name(self) -> &'static str {
        match self {
            Idle::Source => "Source colour, see-through",
            Idle::White => "White, see-through",
            Idle::Hidden => "Hidden (output only)",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Idle::Source => "source",
            Idle::White => "white",
            Idle::Hidden => "hidden",
        }
    }

    pub fn parse(key: &str) -> Option<Idle> {
        Idle::ALL.into_iter().find(|p| p.key() == key)
    }
}

/// How the bars and the day chart look, as set in the energy display window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub palette: Palette,
    pub idle: Idle,
    /// The opacity of unused capacity, 0 to 1.
    pub idle_opacity: f64,
    /// Factors on the usual height and width of the bars.
    pub height: f64,
    pub width: f64,
}

/// The limits of the factors on the bars' height and width.
pub const HEIGHT_RANGE: (f64, f64) = (0.25, 4.0);
pub const WIDTH_RANGE: (f64, f64) = (0.5, 3.0);
pub const OPACITY_RANGE: (f64, f64) = (0.08, 0.9);

impl Default for Style {
    fn default() -> Self {
        Style { palette: Palette::Standard, idle: Idle::Source, idle_opacity: 0.3, height: 1.0, width: 1.0 }
    }
}

impl Style {
    pub fn colour(&self, source: Source) -> [u8; 3] {
        self.palette.colour(source)
    }

    /// The colour and opacity (0 to 255) of a source's unused capacity, if it is shown.
    pub fn idle_colour(&self, source: Source) -> Option<([u8; 3], u8)> {
        let alpha = (self.idle_opacity.clamp(OPACITY_RANGE.0, OPACITY_RANGE.1) * 255.0).round() as u8;
        match self.idle {
            Idle::Source => Some((self.colour(source), alpha)),
            Idle::White => Some((IDLE_WHITE, alpha)),
            Idle::Hidden => None,
        }
    }

    /// The scale for a view with these factors applied.
    pub fn apply(&self, scale: Scale) -> Scale {
        Scale {
            width: scale.width * self.width.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1),
            per_kw: scale.per_kw * self.height.clamp(HEIGHT_RANGE.0, HEIGHT_RANGE.1),
        }
    }
}

const IDLE_WHITE: [u8; 3] = [236, 238, 244];

/// The bars as one mesh round `center`, standing on the ground that `ground` gives (metres).
/// Its indices hold the solid faces first, then the see-through ones from `solid` on, which are
/// drawn blended after everything solid.
pub struct BarMesh {
    pub mesh: BuildingMesh,
    pub solid: u32,
}

/// With `shares` (the part of each source's capacity running), each column shows the output in
/// the sources' colours and above it the rest of each source's capacity as `style` says;
/// without, the whole capacity in the sources' colours.
pub fn mesh(bars: &[Bar], scale: Scale, center: DVec3, ground: &dyn Fn(f64, f64) -> f64, shares: Option<&[f32; SOURCES]>, style: &Style) -> BarMesh {
    let mut out = BuildingMesh { center, vertices: Vec::new(), indices: Vec::new() };
    let mut ghost: Vec<u32> = Vec::new();
    for bar in bars {
        let (sides, width) = match bar.subject {
            Subject::Site(_) | Subject::Plant(_) => (8, scale.width * 0.7),
            _ => (4, scale.width),
        };
        let (east, north, up) = enu(bar.lat, bar.lon);
        let base = geodetic_to_ecef(bar.lat, bar.lon, ground(bar.lat, bar.lon)) - center;
        // Renewable sources in one column, fossil and nuclear in another east of it.
        let groups = [(0..FIRST_FOSSIL, bar.renewable()), (FIRST_FOSSIL..SOURCES, bar.fossil())];
        let both = groups.iter().all(|g| g.1 > 0.0);
        for (n, (range, sum)) in groups.into_iter().enumerate() {
            if sum <= 0.0 {
                continue;
            }
            let shift = if both { east * (width * if n == 0 { -0.55 } else { 0.55 }) } else { DVec3::ZERO };
            let mut segments: Vec<Segment> = Vec::new();
            let mut idle: Vec<Segment> = Vec::new();
            for s in range.filter(|&s| bar.kw[s] > 0.0) {
                let source = Source::ALL[s];
                let share = bar.running_by_source.map(|r| r[s]).or(bar.running).map_or_else(
                    || shares.filter(|_| bar.subject.has_capacity()).map_or(1.0, |sh| sh[s] as f64),
                    f64::from,
                ).clamp(0.0, 1.0);
                let colour = if bar.faint { lighter(style.colour(source)) } else { style.colour(source) };
                segments.push(Segment { colour, alpha: 255, height: bar.kw[s] as f64 * share * scale.per_kw });
                if let Some((colour, alpha)) = style.idle_colour(source) {
                    idle.push(Segment { colour, alpha, height: bar.kw[s] as f64 * (1.0 - share) * scale.per_kw });
                }
            }
            segments.extend(idle);
            segments.retain(|s| s.height > 0.0);
            if segments.is_empty() {
                continue;
            }
            // The fossil and nuclear column is dashed all the way up, so that it never reads as
            // one of the renewable ones, whatever the colours.
            let dash = (n == 1).then_some(width * 0.6);
            column(&mut out, &mut ghost, base + shift, (east, north, up), sides, width, scale.width * 0.3, &segments, dash);
        }
        // The estimated demand: a slim column west of the others, in the colour of the day
        // chart's consumption line.
        if bar.demand > 0.0 {
            let left = if both { -1.05 } else { -0.5 } * width;
            let slim = width * DEMAND_WIDTH;
            let shift = east * (left - width * 0.12 - slim / 2.0);
            let seg = [Segment { colour: DEMAND, alpha: 255, height: bar.demand as f64 * scale.per_kw }];
            column(&mut out, &mut ghost, base + shift, (east, north, up), 4, slim, scale.width * 0.3, &seg, None);
        }
    }
    let solid = out.indices.len() as u32;
    out.indices.extend(ghost);
    BarMesh { mesh: out, solid }
}

/// A colour halfway to white, for bars that stand in with a coarser figure.
pub fn lighter(c: [u8; 3]) -> [u8; 3] {
    c.map(|v| (v as u16 + (255 - v as u16) / 2) as u8)
}

/// The colour of the demand column.
pub const DEMAND: [u8; 3] = [246, 246, 250];
/// The demand column's width, as a share of the others'.
const DEMAND_WIDTH: f64 = 0.45;

/// A part of a column: its colour, opacity (255 solid) and height in metres.
#[derive(Clone, Copy, Debug)]
struct Segment {
    colour: [u8; 3],
    alpha: u8,
    height: f64,
}

/// One column of stacked segments standing at `base`, reaching `below` metres into the ground.
/// Solid faces go to the mesh's indices, see-through ones to `ghost`. With `dash`, its walls are
/// cut into bands that tall, every second one darkened, counted from the ground so that the
/// bands run on across segments.
#[allow(clippy::too_many_arguments)]
fn column(
    out: &mut BuildingMesh,
    ghost: &mut Vec<u32>,
    base: DVec3,
    (east, north, up): (DVec3, DVec3, DVec3),
    sides: usize,
    width: f64,
    below: f64,
    segments: &[Segment],
    dash: Option<f64>,
) {
    let corners: Vec<DVec3> = (0..sides)
        .map(|k| {
            // Squares are turned to face north; octagons have a flat side to the north too.
            let a = std::f64::consts::TAU * (k as f64 + 0.5) / sides as f64;
            let r = width / 2.0 / (std::f64::consts::PI / sides as f64).cos();
            east * (a.cos() * r) + north * (a.sin() * r)
        })
        .collect();
    let colour = |c: [u8; 3], alpha: u8, shade: f32| {
        let [r, g, b] = [0, 1, 2].map(|i| (c[i] as f32 * shade).min(255.0) as u8);
        [r, g, b, alpha]
    };
    // Below the ground a little, to meet it on slopes and where the terrain is not loaded.
    let mut z0 = -below;
    for (n, seg) in segments.iter().enumerate() {
        let z1 = z0.max(0.0) + seg.height;
        let indices = if seg.alpha == 255 { &mut out.indices } else { &mut *ghost };
        // The walls' pieces: from, to, and whether darkened.
        let mut pieces: Vec<(f64, f64, bool)> = Vec::new();
        match dash.filter(|&d| d > 0.0 && (z1 - z0) / d < 2000.0) {
            Some(d) => {
                let mut z = z0;
                while z < z1 {
                    let band = (z.max(0.0) / d + 1e-9).floor();
                    let to = if z < 0.0 { 0.0f64.min(z1).max(z + 1e-6) } else { ((band + 1.0) * d).min(z1) };
                    pieces.push((z, to, z >= 0.0 && band as i64 % 2 == 1));
                    z = to;
                }
            }
            None => pieces.push((z0, z1, false)),
        }
        for (a0, a1, dark) in pieces {
            // Shaded from 0.8 at the segment's foot to 1.0 at its top, as without bands.
            let shade = |z: f64| (0.8 + 0.2 * ((z - z0) / (z1 - z0).max(1e-9))) as f32 * if dark { DASH_SHADE } else { 1.0 };
            for k in 0..sides {
                let (a, b) = (corners[k], corners[(k + 1) % sides]);
                let first = out.vertices.len() as u32;
                for (p, z) in [(a, a0), (b, a0), (b, a1), (a, a1)] {
                    out.vertices.push(BuildingVertex { pos: (base + p + up * z).as_vec3().to_array(), colour: colour(seg.colour, seg.alpha, shade(z)), roof: [0, 0] });
                }
                indices.extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
            }
        }
        // A lid on the top, and on the last solid segment below see-through ones, so that the
        // solid part does not look hollow through them.
        let next_alpha = segments.get(n + 1).map(|s| s.alpha);
        if next_alpha.is_none() || (seg.alpha == 255 && next_alpha != Some(255)) {
            let first = out.vertices.len() as u32;
            for p in &corners {
                out.vertices.push(BuildingVertex { pos: (base + *p + up * z1).as_vec3().to_array(), colour: colour(seg.colour, seg.alpha, 1.1), roof: [0, 0] });
            }
            for k in 1..sides as u32 - 1 {
                indices.extend_from_slice(&[first, first + k, first + k + 1]);
            }
        }
        z0 = z1;
    }
}

/// How dark the gaps between the dashes of a fossil column are.
const DASH_SHADE: f32 = 0.42;

/// The bar under a screen position: the nearest one whose drawn box holds it.
pub fn hit(bars: &[Bar], scale: Scale, ground: &dyn Fn(f64, f64) -> f64, project: &dyn Fn(DVec3) -> Option<glam::DVec2>, at: glam::DVec2) -> Option<Subject> {
    let mut best: Option<(f64, Subject)> = None;
    for bar in bars {
        let up = enu(bar.lat, bar.lon).2;
        let base = geodetic_to_ecef(bar.lat, bar.lon, ground(bar.lat, bar.lon));
        let top = base + up * (bar.height().max(bar.demand) as f64 * scale.per_kw);
        let (Some(b), Some(t)) = (project(base), project(top)) else { continue };
        // A bar with demand beside it is wider: the demand column stands west of the others.
        let reach = if bar.demand > 0.0 { 1.8 } else { 0.5 };
        let half = project(base + enu(bar.lat, bar.lon).0 * scale.width * reach).map_or(4.0, |e| (e - b).length()).max(4.0) + 2.0;
        // Distance from the point to the bar's axis on screen, and how far along it lies.
        let axis = t - b;
        let len = axis.length().max(1e-6);
        let along = (at - b).dot(axis) / len;
        let across = ((at - b).perp_dot(axis) / len).abs();
        if across <= half && along >= -half && along <= len + half {
            let depth = (at - b).length();
            if best.is_none_or(|(d, _)| depth < d) {
                best = Some((depth, bar.subject));
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn varint(out: &mut Vec<u8>, v: i64) {
        let mut z = ((v << 1) ^ (v >> 63)) as u64;
        loop {
            let b = (z & 0x7f) as u8;
            z >>= 7;
            if z == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    /// Writes a file as `gen_energy.py` does, for tests.
    pub fn sample() -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(20261107u32.to_le_bytes());
        let deg = |v: f64| ((v * 1e6).round() as i32).to_le_bytes();
        let square = |lat: f64, lon: f64, d: f64| vec![(lat, lon), (lat + d, lon), (lat + d, lon + d), (lat, lon + d), (lat, lon)];
        // A state, a district in it and two municipalities.
        let areas = [(0u8, u32::MAX, "Land", square(48.0, 11.0, 1.0)), (1, 0, "Kreis", square(48.0, 11.0, 0.5)), (2, 1, "Nord", square(48.25, 11.0, 0.25)), (2, 1, "Süd", square(48.0, 11.0, 0.25))];
        out.extend((areas.len() as u32).to_le_bytes());
        for (level, parent, name, ring) in &areas {
            out.push(*level);
            out.push(0);
            out.extend(parent.to_le_bytes());
            out.push(name.len() as u8);
            out.extend(name.as_bytes());
            out.extend(deg(ring[0].0 + 0.1));
            out.extend(deg(ring[0].1 + 0.1));
            out.extend(1u16.to_le_bytes());
            out.extend((ring.len() as u32).to_le_bytes());
            out.extend(deg(ring[0].0));
            out.extend(deg(ring[0].1));
            for w in ring.windows(2) {
                for d in [w[1].0 - w[0].0, w[1].1 - w[0].1] {
                    varint(&mut out, (d * 1e5).round() as i64);
                }
            }
        }
        // Rooftop solar without positions in "Nord": 10 kW from January 2010, 5 kW more from 2020.
        out.extend(2u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.push(0);
        out.extend(2u32.to_le_bytes());
        for v in [110 * 12, 100, 10 * 12, 50] {
            varint(&mut out, v);
        }
        // A 900 MW lignite plant given only by district ("Kreis"), from 1990.
        out.extend(1u32.to_le_bytes());
        out.push(5);
        out.extend(1u32.to_le_bytes());
        for v in [90 * 12, 9_000_000] {
            varint(&mut out, v);
        }
        // A wind site in "Süd": a 3 MW turbine from 2005, taken down in 2025.
        out.extend(1u32.to_le_bytes());
        for v in [3, 481_000, 111_000] {
            varint(&mut out, v);
        }
        out.push(1);
        for v in [1, 105 * 12, 20 * 12 + 1, 30_000] {
            varint(&mut out, v);
        }
        out
    }

    #[test]
    fn capacity_adds_up_through_the_levels_and_follows_the_date() {
        let e = Energy::parse(&sample()).unwrap();
        assert_eq!(e.exported, (2026, 11, 7));
        let all = [true; SOURCES];
        let state = |m| e.bars(Level::States, m, all, None);
        assert_eq!(state(100 * 12)[0].renewable(), 0.0, "nothing renewable before 2005");
        assert_eq!(state(115 * 12)[0].kw[..2], [10.0, 3000.0]);
        assert_eq!(state(115 * 12)[0].renewable(), 3010.0);
        assert_eq!(state(126 * 12)[0].renewable(), 15.0, "the turbine is gone");
        let towns = e.bars(Level::Towns, 115 * 12, all, None);
        assert_eq!(towns.len(), 3, "two municipalities and the district's own units");
        assert!(towns.iter().any(|b| b.subject == Subject::Unplaced(1) && b.fossil() == 900_000.0 && b.renewable() == 0.0));
        assert_eq!(state(115 * 12)[0].fossil(), 900_000.0);
        let plants = e.bars(Level::Plants, 115 * 12, all, None);
        assert_eq!(plants.len(), 2, "units without a position are no bar close up, but the district's are");
        assert!(plants.iter().any(|b| b.subject == Subject::Site(0) && b.kw[1] == 3000.0));
        let mut no_wind = [true; SOURCES];
        no_wind[1] = false;
        assert_eq!(e.bars(Level::Plants, 115 * 12, no_wind, None).len(), 1);
        // "Nord" is a quarter degree square: about 515 km², so 10 kW is about 0.019 kW per km².
        let d = e.unplaced_density(2, 115 * 12, all).unwrap();
        assert!((d - 10.0 / 515.0).abs() < 0.002, "{d}");
        assert_eq!(e.town_at(48.3f64.to_radians(), 11.1f64.to_radians()), Some(2));
        assert_eq!(e.town_at(48.1f64.to_radians(), 11.1f64.to_radians()), Some(3));
        assert_eq!(e.town_at(47.0f64.to_radians(), 11.1f64.to_radians()), None);
        let card = e.describe(Subject::Area(1), 115 * 12, all, Palette::Standard, None);
        let row = |card: &Card, group: &str, label: &str| card.meters.iter().find(|g| g.title == group).and_then(|g| g.rows.iter().find(|r| r.label == label)).map(|r| r.text.clone());
        assert_eq!(row(&card, "By source", "Wind on land").as_deref(), Some("3.0 MW"), "{card:?}");
        assert_eq!(row(&card, "", "Installed").as_deref(), Some("903.0 MW"));
        assert!(card.meters[0].caption.starts_with("0% renewable, 100% fossil"), "{}", card.meters[0].caption);
        assert!(card.summary.starts_with("903.0 MW installed in January 2015, 0% renewable. Largest: lignite"), "{}", card.summary);
        let mut shares = [0.0; SOURCES];
        shares[1] = 0.5;
        let at = At { shares: &shares, when: "12:00 UTC", consumption_mw: 60_000.0 };
        let card = e.describe(Subject::Area(1), 115 * 12, all, Palette::Standard, Some(at));
        assert_eq!(row(&card, "By source", "Wind on land").as_deref(), Some("1.5 MW of 3.0 MW"), "{card:?}");
        assert_eq!(row(&card, "", "Running").as_deref(), Some("1.5 MW"));
        assert_eq!(row(&card, "", "Use (est.)"), None, "no population, no demand");
        assert!(card.summary.starts_with("About 1.5 MW running at 12:00 UTC, of 903.0 MW installed (0%)"), "{}", card.summary);
        assert!(card.notes.iter().any(|n| n.contains("estimated")));
        assert_eq!(e.national(115 * 12)[1], 3000.0);
        assert_eq!(e.subject_title(Subject::Area(3)), "Süd, Kreis, Land");
    }

    /// A population file as `gen_population.py` writes it, for the sample's four areas.
    fn population(people: &[i64]) -> Vec<u8> {
        let mut out = POPULATION_MAGIC.to_vec();
        out.extend(20251231u32.to_le_bytes());
        out.extend((people.len() as u32).to_le_bytes());
        for &p in people {
            varint(&mut out, p);
        }
        out
    }

    #[test]
    fn demand_is_germanys_consumption_shared_by_population() {
        let mut e = Energy::parse(&sample()).unwrap();
        assert!(!e.attach_population(&population(&[100, 100, 60])), "one area short");
        assert!(!e.attach_population(b"nonsense"));
        assert_eq!(e.demand_kw(Subject::Area(0), 1000.0), None);
        assert!(e.attach_population(&population(&[1000, 1000, 600, 400])));
        // 1 GW for 1000 people: the state uses all of it, "Nord" with 600 people 60%.
        assert_eq!(e.demand_kw(Subject::Area(0), 1000.0), Some(1_000_000.0));
        assert_eq!(e.demand_kw(Subject::Area(2), 1000.0), Some(600_000.0));
        assert_eq!(e.demand_kw(Subject::Unplaced(2), 1000.0), Some(600_000.0), "the municipality's");
        assert_eq!(e.demand_kw(Subject::Site(0), 1000.0), None);
        let shares = [0.5; SOURCES];
        let at = At { shares: &shares, when: "12:00 UTC", consumption_mw: 1.0 };
        let card = e.describe(Subject::Area(2), 126 * 12, [true; SOURCES], Palette::Standard, Some(at));
        let first = &card.meters[0];
        let used = first.rows.iter().find(|r| r.label == "Use (est.)").unwrap();
        assert_eq!(used.text, "\u{2248} 600 kW");
        assert!(used.segments[0].ink);
        assert_eq!(first.max, 600.0, "the scale holds the demand, larger than the 15 kW installed");
        assert!(first.caption.contains("running covers 1% of the use"), "{}", first.caption);
        assert!(card.facts.contains(&("Population".to_string(), "600".to_string())));
        assert_eq!(group_thousands(83_467_117), "83\u{2009}467\u{2009}117");
    }

    #[test]
    fn the_bundled_population_fits_the_bundled_areas() {
        let e = Energy::bundled().unwrap();
        let total = e.people_total;
        assert!((80e6..90e6).contains(&total), "{total}");
        // Germany at 50 GW: Berlin, with about 3.9 million people, some 2.3 GW.
        let berlin = e.areas.iter().position(|a| a.level == Level::States && a.name == "Berlin").unwrap() as u32;
        let gw = e.demand_kw(Subject::Area(berlin), 50_000.0).unwrap() / 1e6;
        assert!((2.0..2.6).contains(&gw), "{gw}");
    }

    #[test]
    fn outlines_follow_the_level_and_the_choice() {
        let e = Energy::parse(&sample()).unwrap();
        let all = [true; SOURCES];
        assert_eq!(e.outlines(Level::Towns, all, None, 126 * 12, Palette::Standard).items.len(), 3, "and the district of the plant");
        // Close up only municipalities with unplaced units are outlined, and shaded fully when they
        // are the densest.
        let close = e.outlines(Level::Plants, all, None, 126 * 12, Palette::Standard);
        assert_eq!(close.items.len(), 2);
        assert_eq!(close.items[0].outline.unwrap().fill[3], SHADE_ALPHA);
        assert_eq!(e.outlines(Level::Plants, all, None, 100 * 12, Palette::Standard).items[0].outline.unwrap().fill[3], 0, "nothing yet in 2000");
        let chosen = e.outlines(Level::Towns, all, Some(Subject::Area(3)), 126 * 12, Palette::Standard);
        assert_eq!(chosen.items.last().unwrap().outline, Some(SELECTED));
        // A chosen area is filled in its main source's colour: "Nord" in solar's, solid, and the
        // district of the lignite plant (given only by district) hatched.
        let nord = e.outlines(Level::Towns, all, Some(Subject::Area(2)), 126 * 12, Palette::Standard).items.last().unwrap().outline.unwrap();
        let [r, g, b] = Source::Solar.colour();
        assert_eq!((nord.fill, nord.dashed, nord.edge), ([r, g, b, SELECTED_FILL_ALPHA], false, SELECTED.edge));
        assert_eq!(e.main_source(Subject::Unplaced(1), 126 * 12, all), Some(Source::Lignite));
        let plant = e.outlines(Level::Towns, all, Some(Subject::Unplaced(1)), 126 * 12, Palette::Standard).items.last().unwrap().outline.unwrap();
        assert!(plant.dashed && plant.fill[..3] == Source::Lignite.colour());
        // Unchosen, that district's outline is dashed too, unless lignite is hidden.
        let district = |shown| e.outlines(Level::Towns, shown, None, 126 * 12, Palette::Standard).items.into_iter().find(|i| i.outline.unwrap().edge == DISTRICT_ONLY_EDGE.edge).unwrap();
        assert!(district(all).outline.unwrap().dashed);
        let mut no_lignite = all;
        no_lignite[Source::Lignite as usize] = false;
        assert_eq!(e.main_source(Subject::Unplaced(1), 126 * 12, no_lignite), None);
        let site = e.outlines(Level::Plants, all, Some(Subject::Site(0)), 126 * 12, Palette::Standard);
        assert!(matches!(site.items.last().unwrap().shape, Shape::Point(_)));
        let ring = &e.areas[0].rings[0];
        assert!((ring[2].0.to_degrees() - 49.0).abs() < 1e-5 && (ring[2].1.to_degrees() - 12.0).abs() < 1e-5);
    }

    #[test]
    fn the_bundled_register_reads_and_adds_up() {
        let e = Energy::bundled().expect("data/energy_de.bin reads");
        let month = month_of(sky_time_unix(2026, 6));
        let all = [true; SOURCES];
        let states: f32 = e.bars(Level::States, month, all, None).iter().map(Bar::total).sum();
        let towns: f32 = e.bars(Level::Towns, month, all, None).iter().map(Bar::total).sum();
        let plants: f32 = e.bars(Level::Plants, month, all, None).iter().map(Bar::total).sum();
        // About 215 GW of renewable and 80 GW of fossil capacity in mid 2026; every level counts all of it.
        assert!((250e6..350e6).contains(&states), "{states}");
        let shaded: f32 = (0..e.areas.len() as u32).filter_map(|i| e.unplaced_density(i, month, all).map(|d| d as f64 * e.km2[i as usize])).sum::<f64>() as f32;
        assert!((towns / states - 1.0).abs() < 1e-3 && ((plants + shaded) / states - 1.0).abs() < 1e-3, "{states} {towns} {plants} {shaded}");
        assert_eq!(e.areas.iter().filter(|a| a.level == Level::States && a.kind == AreaKind::Admin).count(), 16);
    }

    fn sky_time_unix(year: i64, month: i64) -> f64 {
        crate::sky_time::days_from_civil(year, month, 15) as f64 * 86_400.0
    }

    #[test]
    fn the_fossil_column_is_dashed_and_the_renewable_one_solid() {
        let mut kw = [0.0; SOURCES];
        kw[Source::Solar as usize] = 100.0;
        kw[Source::Gas as usize] = 100.0;
        let bar = Bar { lat: 0.8, lon: 0.2, kw, subject: Subject::Area(0), demand: 0.0, faint: false, running: None, running_by_source: None };
        let scale = Scale { width: 10.0, per_kw: 1.0 };
        let m = mesh(&[bar], scale, geodetic_to_ecef(0.8, 0.2, 0.0), &|_, _| 0.0, None, &Style::default()).mesh;
        // Each colour comes in one shade of wall per height; the dark bands give gas a second set.
        let (east, ..) = enu(0.8, 0.2);
        let shades = |west: bool| {
            let mut v: Vec<[u8; 4]> = m.vertices.iter().filter(|v| (glam::Vec3::from_array(v.pos).as_dvec3().dot(east) < 0.0) == west).map(|v| v.colour).collect();
            v.sort();
            v.dedup();
            v.len()
        };
        let (solar, gas) = (shades(true), shades(false));
        // A gas column 100 m tall in bands of 6 m: about 17 bands, half of them dark.
        assert!(gas > solar * 5, "solar {solar} shades, gas {gas}");
        assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
    }

    #[test]
    fn the_plants_scale_takes_the_98th_percentile_of_the_plants_in_view() {
        let bar = |i: u32, kw: f32, subject: Subject| {
            let mut v = [0.0; SOURCES];
            v[Source::Solar as usize] = kw;
            Bar { lat: 0.8, lon: 0.001 * i as f64, kw: v, subject, demand: 0.0, faint: false, running: None, running_by_source: None }
        };
        // 100 sites of 1 to 100 MW: the 99th smallest sets the scale, the largest does not.
        let mut bars: Vec<Bar> = (1..=100).map(|i| bar(i, i as f32 * 1000.0, if i % 2 == 0 { Subject::Site(i) } else { Subject::Plant(i) })).collect();
        let per_kw = |bars: &[Bar], visible: &dyn Fn(f64, f64) -> bool| plants_needed(bars, visible).unwrap();
        assert!((per_kw(&bars, &|_, _| true) - HEIGHT_SHARE * PLANT_WIDTH / 99_000.0).abs() < 1e-12);
        // Areas and bars out of view do not count.
        bars.push(bar(0, 1e9, Subject::Area(0)));
        assert!((per_kw(&bars, &|_, lon| lon < 0.0105) - HEIGHT_SHARE * PLANT_WIDTH / 10_000.0).abs() < 1e-12);
        // A view of rooftops is scaled for at least the floor; none in view, nothing.
        let roofs: Vec<Bar> = (0..10).map(|i| bar(i, 10.0, Subject::Site(i))).collect();
        assert!((per_kw(&roofs, &|_, _| true) - HEIGHT_SHARE * PLANT_WIDTH / PLANT_FLOOR_KW as f64).abs() < 1e-12);
        assert!(plants_needed(&roofs, &|_, _| false).is_none());
        // Held while the view needs less than 1.5 times more or less, then eased over half a second.
        let mut e = crate::world_energy::Easing::new(per_kw(&bars, &|_, _| true));
        let start = e.per_kw;
        assert!(!e.step(Some(start * 1.4), 0.1) && e.per_kw == start);
        assert!(e.step(Some(start / 2.0), 0.25) && e.per_kw < start && e.per_kw > start / 2.0);
        assert!(!e.step(Some(start / 2.0), 0.3) && (e.per_kw - start / 2.0).abs() < 1e-12);
    }

    #[test]
    fn visual_plant_bars_group_overlapping_sites_without_losing_capacity_or_output() {
        let mut kw = [0.0; SOURCES];
        kw[Source::Solar as usize] = 10.0;
        let bars: Vec<_> = (0..13_000).map(|i| Bar {
            lat: 0.8, lon: i as f64 * 1e-9, kw, subject: Subject::Plant(i), demand: 0.0,
            faint: false, running: Some(0.4), running_by_source: None,
        }).collect();
        let mut shares = [0.0; SOURCES];
        shares[Source::Solar as usize] = 0.9;
        let visual = visual_bars(&bars, (1000.0, 800.0), Some(&shares), &|_, _| Some(glam::DVec2::new(400.0, 300.0)));
        assert!(visual.len() < 10);
        assert!((visual.iter().map(Bar::total).sum::<f32>() - bars.iter().map(Bar::total).sum::<f32>()).abs() < 1.0);
        let solar = Source::Solar as usize;
        let output: f32 = visual.iter().map(|b| b.kw[solar] * b.running_by_source.map_or_else(|| b.running.unwrap_or(shares[solar]), |r| r[solar])).sum();
        assert!((output - 13_000.0 * 10.0 * 0.4).abs() < 1.0);
    }

    #[test]
    fn over_germany_the_plants_scale_is_about_the_fixed_one() {
        let e = Energy::bundled().unwrap();
        let last = ((e.exported.0 - 1900) * 12 + e.exported.1 - 1) as u16;
        let fixed = Scale::for_view(e, Level::Plants, 1e4, 1000.0).per_kw;
        let all = e.bars(Level::Plants, last, [true; SOURCES], None);
        // All of Germany: about the fixed scale, which takes the 98th percentile of all the
        // level's bars (the bars of towns' units without a position among them).
        let whole = plants_needed(&all, &|_, _| true).unwrap();
        assert!((whole / fixed - 1.0).abs() < 0.05, "{whole} against {fixed}");
        // Views of 50 km around German places stay within a few times of it: closer views follow
        // their own sites (a city of rooftops stands taller, a lignite field lower).
        for (name, lat, lon) in [("Kassel", 51.31, 9.48), ("Munich", 48.14, 11.58), ("Husum", 54.48, 9.05), ("Cottbus", 51.76, 14.33), ("Ruhr", 51.45, 7.0)] {
            let (lat, lon) = (f64::to_radians(lat), f64::to_radians(lon));
            let near = plants_needed(&all, &|a, b| angle_between(a, b, lat, lon) < 50.0 / 6371.0).unwrap();
            assert!((0.4..=2.5).contains(&(near / fixed)), "{name}: {near} against {fixed}");
        }
    }

    #[test]
    fn a_world_plant_runs_at_its_own_share_and_germanys_bars_keep_theirs() {
        let mut kw = [0.0; SOURCES];
        kw[Source::Solar as usize] = 100.0;
        let site = Bar { lat: 0.8, lon: 0.2, kw, subject: Subject::Site(0), demand: 0.0, faint: false, running: None, running_by_source: None };
        let plant = Bar { lat: 0.8, lon: 0.25, kw, subject: Subject::Plant(0), demand: 0.0, faint: false, running: Some(0.4), running_by_source: None };
        let scale = Scale { width: 10.0, per_kw: 1.0 };
        let center = geodetic_to_ecef(0.8, 0.2, 0.0);
        let mut shares = [0.0; SOURCES];
        shares[Source::Solar as usize] = 0.9;
        let style = Style::default();
        let alone = mesh(std::slice::from_ref(&site), scale, center, &|_, _| 0.0, Some(&shares), &style).mesh;
        let both = mesh(&[site, plant.clone()], scale, center, &|_, _| 0.0, Some(&shares), &style).mesh;
        // Germany's site is drawn exactly as without the world plant beside it.
        let bytes = |v: &[crate::buildings::BuildingVertex]| bytemuck::cast_slice::<_, u8>(v).to_vec();
        assert_eq!(bytes(&alone.vertices), bytes(&both.vertices[..alone.vertices.len()]));
        // The plant's solid part is 40% of its capacity, whatever Germany's shares say.
        let (.., up) = enu(0.8, 0.25);
        let base = (geodetic_to_ecef(0.8, 0.25, 0.0) - center).dot(up);
        let m = mesh(&[plant], scale, center, &|_, _| 0.0, Some(&shares), &style).mesh;
        let top = m.vertices.iter().filter(|v| v.colour[3] == 255).map(|v| glam::Vec3::from_array(v.pos).as_dvec3().dot(up) - base).fold(f64::MIN, f64::max);
        assert!((top - 40.0).abs() < 1.0, "{top}");
    }

    #[test]
    fn demand_stands_slim_and_west_of_the_columns() {
        let mut kw = [0.0; SOURCES];
        kw[Source::Solar as usize] = 100.0;
        kw[Source::Gas as usize] = 100.0;
        let bar = Bar { lat: 0.8, lon: 0.2, kw, subject: Subject::Area(0), demand: 300.0, faint: false, running: None, running_by_source: None };
        let scale = Scale { width: 10.0, per_kw: 1.0 };
        let center = geodetic_to_ecef(0.8, 0.2, 0.0);
        let m = mesh(&[bar], scale, center, &|_, _| 0.0, None, &Style::default()).mesh;
        let (east, _, up) = enu(0.8, 0.2);
        let demand: Vec<glam::DVec3> = m.vertices.iter().filter(|v| v.colour[..3] == [DEMAND[0], DEMAND[1], DEMAND[2]] || v.colour[0] > 240 && v.colour[1] > 240 && v.colour[2] > 240).map(|v| glam::Vec3::from_array(v.pos).as_dvec3()).collect();
        assert!(!demand.is_empty());
        let top = demand.iter().map(|p| p.dot(up)).fold(f64::MIN, f64::max);
        assert!((top - 300.0).abs() < 1.0, "300 kW at 1 m per kW: {top}");
        // West of the solar column, whose west side is at -10.5 m, and 4.5 m wide.
        let xs: Vec<f64> = demand.iter().map(|p| p.dot(east)).collect();
        let (lo, hi) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
        assert!(hi < -10.5 && (hi - lo - 4.5).abs() < 0.1, "{lo}..{hi}");
    }

    #[test]
    fn bars_keep_their_size_in_the_landscape() {
        let e = Energy::bundled().unwrap();
        let near = Scale::for_view(e, Level::States, 4.0e5, 1000.0);
        let far = Scale::for_view(e, Level::States, 8.0e5, 1000.0);
        assert_eq!(near.per_kw, far.per_kw, "height doesn't follow the camera");
        // The tall state bar is a fraction of a state's width: tens of kilometres, not hundreds.
        let tall = e.largest(Level::States) as f64 * near.per_kw;
        assert!((2.0e4..1.5e5).contains(&tall), "{tall}");
        // From far away bars stay a few pixels wide.
        let globe = Scale::for_view(e, Level::States, 2.0e7, 1000.0);
        assert!(globe.width > near.width);
        assert!(e.widths[Level::States.index()] > e.widths[Level::Districts.index()]);
        assert!(e.widths[Level::Districts.index()] > e.widths[Level::Towns.index()]);
    }

    #[test]
    fn levels_change_with_altitude_and_hold_near_the_limits() {
        assert_eq!(Level::for_altitude(5e6, None), Level::Countries, "a continent shows countries");
        assert_eq!(Level::for_altitude(1.7e6, None), Level::Countries);
        assert_eq!(Level::for_altitude(1.4e6, None), Level::States);
        // Countries are kept a little below their limit, and states a little above theirs.
        assert_eq!(Level::for_altitude(1.4e6, Some(Level::Countries)), Level::Countries);
        assert_eq!(Level::for_altitude(1.2e6, Some(Level::Countries)), Level::States);
        assert_eq!(Level::for_altitude(1.8e6, Some(Level::States)), Level::States);
        assert_eq!(Level::for_altitude(2.0e6, Some(Level::States)), Level::Countries);
        assert_eq!(Level::for_altitude(620e3, None), Level::States, "Germany filling the screen shows states");
        assert_eq!(Level::for_altitude(150e3, None), Level::Districts);
        assert_eq!(Level::for_altitude(30e3, None), Level::Towns);
        assert_eq!(Level::for_altitude(5e3, None), Level::Plants);
        assert_eq!(Level::for_altitude(330e3, Some(Level::Districts)), Level::Districts);
        assert_eq!(Level::for_altitude(400e3, Some(Level::Districts)), Level::States);
        assert_eq!(Level::for_altitude(14e3, Some(Level::Towns)), Level::Towns);
    }

    #[test]
    fn months_and_powers_read_well() {
        assert_eq!(month_of(0.0), 70 * 12);
        assert_eq!(month_name(126 * 12 + 10), "November 2026");
        assert_eq!(format_power(1500.0), "1.5 MW");
        assert_eq!(format_power(2_340_000.0), "2.34 GW");
    }
}

