//! The States level of the energy layer beyond Germany: one bar per state or province of the
//! world, from Natural Earth's admin-1 states and provinces at 1:10m (public domain), simplified
//! by `tools/gen_world_states.py` into `data/states_world.bin` with each state's population from
//! Wikidata (CC0). Germany's states are BKG's, in `energy.rs`, and are left out here. Where a
//! country's admin-1 units are many and fine (France's departments, Italy's provinces), they
//! come merged into the regions Natural Earth groups them in, to compare with Germany's states.
//!
//! Until the world's plants give monthly output (M3), a state's bar is its country's use (Ember's
//! demand, or its generation where there is none) shared out by population, in the country's
//! mix of sources at the clock's month; it has no column of unused capacity. Its installed
//! capacity, the world plants (`world_plants.rs`) inside its outline, is told in its card.

use crate::countries::Countries;
use crate::energy::{format_power, group_thousands, Bar, Palette, Source, Subject, AREA_EDGE, SELECTED, SELECTED_FILL_ALPHA, SOURCES};
use crate::import::{Imported, Item, Outline as Edge, Pos, Shape};
use crate::info::{Card, Meter, Meters, Segment as Part};
use crate::owid::{Owid, Reader};
use crate::world_energy::{Basis, Figures, MAPPING};
use crate::world_plants::Plants;

pub const CREDIT: &str = "Country and state outlines: Natural Earth (public domain); population of states: Wikidata (CC0)";

pub struct State {
    /// The country's code, as in `countries.rs` and `owid.rs`.
    pub country: String,
    /// ISO 3166-2, such as `FR-BRE`; empty where Natural Earth has none.
    pub iso: String,
    pub name: String,
    /// Natural Earth's kind of unit, such as Province, or Region where units are merged.
    pub kind: String,
    pub population: u32,
    /// Whether the population is estimated from the country's density, Wikidata having none.
    pub estimated: bool,
    /// Latitude and longitude in degrees.
    pub label: [f64; 2],
    /// Rings of [latitude, longitude] in degrees, not closed; taken even-odd.
    pub rings: Vec<Vec<[f64; 2]>>,
}

const MAGIC: &[u8; 8] = b"ORBS\x01\0\0\0";

pub struct States {
    list: Vec<State>,
    /// The population of each state's country, summed over its states.
    people: Vec<f64>,
}

impl States {
    /// The states built into the program.
    pub fn bundled() -> Option<&'static States> {
        static DATA: std::sync::OnceLock<Option<States>> = std::sync::OnceLock::new();
        DATA.get_or_init(|| States::parse(include_bytes!("../data/states_world.bin"))).as_ref()
    }

    pub fn parse(data: &[u8]) -> Option<States> {
        let mut r = Reader { data, at: 0 };
        if r.bytes(8)? != MAGIC {
            return None;
        }
        let per_degree = r.u16()?;
        if per_degree == 0 {
            return None;
        }
        let n = r.u16()? as usize;
        let mut list = Vec::with_capacity(n);
        let deg = |v: i64| v as f64 / per_degree as f64;
        for _ in 0..n {
            let (country, iso, name, kind) = (r.string()?, r.string()?, r.string()?, r.string()?);
            let population = r.u32()?;
            let estimated = r.u8()? & 1 != 0;
            let (lat0, lon0) = (r.varint()?, r.varint()?);
            let mut rings = Vec::new();
            for _ in 0..r.u16()? {
                let points = usize::try_from(r.varint()?).ok()?;
                if points < 3 {
                    return None;
                }
                let (mut lat, mut lon) = (lat0, lon0);
                let mut ring = Vec::with_capacity(points.min(1 << 16));
                for _ in 0..points {
                    lat += r.varint()?;
                    lon += r.varint()?;
                    ring.push([deg(lat), deg(lon)]);
                }
                rings.push(ring);
            }
            if rings.is_empty() || country.is_empty() {
                return None;
            }
            list.push(State { country, iso, name, kind, population, estimated, label: [deg(lat0), deg(lon0)], rings });
        }
        if r.at != data.len() {
            return None;
        }
        let mut sums = std::collections::HashMap::<&str, f64>::new();
        for s in &list {
            *sums.entry(s.country.as_str()).or_default() += s.population as f64;
        }
        let people = list.iter().map(|s| sums[s.country.as_str()]).collect();
        Some(States { list, people })
    }

    #[cfg(test)]
    pub fn all(&self) -> &[State] {
        &self.list
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    /// A state's country code.
    pub fn country(&self, i: usize) -> &str {
        &self.list[i].country
    }

    /// A state's ISO 3166-2 code; empty where Natural Earth has none.
    #[cfg(test)]
    pub fn iso(&self, i: usize) -> &str {
        &self.list[i].iso
    }

    /// South, west, north and east of a state, in degrees.
    pub fn bounds(&self, i: usize) -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in self.list[i].rings.iter().flatten() {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        b
    }

    /// Whether a point (degrees) lies inside a state's outline.
    pub fn contains(&self, i: usize, lat: f64, lon: f64) -> bool {
        inside(&self.list[i].rings, lat, lon)
    }

    /// A state's share of its country's population, 0 to 1.
    pub fn share(&self, i: usize) -> f64 {
        let total = self.people[i];
        if total > 0.0 {
            self.list[i].population as f64 / total
        } else {
            0.0
        }
    }

    /// The population of a country summed over its states.
    #[cfg(test)]
    pub fn country_people(&self, code: &str) -> f64 {
        self.list.iter().filter(|s| s.country == code).map(|s| s.population as f64).sum()
    }

    /// A state's use at the clock's month: the country's figures and the state's power by the
    /// program's sources with those in `shown`, in kW, and its use in kW.
    fn estimate(&self, i: usize, month: (i32, u32), shown: [bool; SOURCES]) -> Option<(Figures, [f32; SOURCES], f32)> {
        let c = Owid::bundled()?.country(&self.list[i].country)?;
        let f = Figures::of(c, month)?;
        let share = self.share(i) as f32;
        let used = f.demand.filter(|d| *d > 0.0).unwrap_or(f.generation);
        let k = if f.generation > 0.0 { used / f.generation * share } else { 0.0 };
        let kw = f.kw(shown).map(|v| v * k);
        Some((f, kw, used * share))
    }

    /// The states' bars at the clock's month, with the sources in `shown` and their use beside
    /// them if `demand`; lighter where the country's year stands in for the month.
    pub fn bars(&self, month: (i32, u32), shown: [bool; SOURCES], demand: bool) -> Vec<Bar> {
        let mut bars = Vec::new();
        for (i, s) in self.list.iter().enumerate() {
            let Some((f, kw, used)) = self.estimate(i, month, shown) else { continue };
            let b = Bar {
                lat: s.label[0].to_radians(),
                lon: s.label[1].to_radians(),
                kw,
                subject: Subject::State(i as u16),
                demand: if demand { used } else { 0.0 },
                faint: matches!(f.basis, Basis::Year(_)),
            };
            if b.total() > 0.0 {
                bars.push(b);
            }
        }
        bars
    }

    /// The states' outlines, added to `into`; the chosen one emphasised in the colour of its main
    /// source and drawn last.
    pub fn outlines(&self, into: &mut Imported, selected: Option<Subject>, month: (i32, u32), shown: [bool; SOURCES], palette: Palette) {
        let mut chosen = None;
        for (i, s) in self.list.iter().enumerate() {
            let mut item = polygon(&s.rings);
            if selected == Some(Subject::State(i as u16)) {
                let main = self.estimate(i, month, shown).and_then(|(_, kw, _)| {
                    Source::ALL.into_iter().filter(|s| kw[*s as usize] > 0.0).max_by(|a, b| kw[*a as usize].total_cmp(&kw[*b as usize]))
                });
                item.outline = Some(match main {
                    Some(m) => {
                        let [r, g, b] = palette.colour(m);
                        Edge { fill: [r, g, b, SELECTED_FILL_ALPHA], dashed: m.fossil(), ..SELECTED }
                    }
                    None => SELECTED,
                });
                chosen = Some(item);
            } else {
                item.outline = Some(Edge { width: 1.0, ..AREA_EDGE });
                into.items.push(item);
            }
        }
        into.items.extend(chosen);
    }

    /// Germany's outlines at the States level with the world's states before them, the chosen
    /// outline last of all, so that it is drawn over its neighbours' edges.
    pub fn with_germany(&self, germany: Imported, selected: Option<Subject>, month: (i32, u32), shown: [bool; SOURCES], palette: Palette) -> Imported {
        let mut out = Imported { items: Vec::new(), ..germany };
        self.outlines(&mut out, selected, month, shown, palette);
        out.items.extend(germany.items);
        out.items.sort_by_key(|i| i.outline.is_some_and(|o| o.width == SELECTED.width));
        out
    }

    /// A state's installed capacity by source at the clock's month, in kW: the bundled world
    /// plants inside its outline, with the sources in `shown`.
    pub fn capacity(&self, i: usize, (year, month): (i32, u32), shown: [bool; SOURCES]) -> Option<[f32; SOURCES]> {
        let plants = Plants::bundled()?;
        let list = Plants::bundled_by_state()?.get(i)?;
        let m = ((year - 1900) * 12 + month as i32 - 1).clamp(0, u16::MAX as i32 - 1) as u16;
        let mut kw = plants.capacity(list, m);
        for (v, on) in kw.iter_mut().zip(shown) {
            if !on {
                *v = 0.0;
            }
        }
        Some(kw)
    }

    /// The state's name and its country's.
    pub fn title(&self, i: u16) -> String {
        let Some(s) = self.list.get(i as usize) else { return String::new() };
        match self.country_name(s) {
            Some(c) => format!("{}, {c}", s.name),
            None => s.name.clone(),
        }
    }

    fn country_name(&self, s: &State) -> Option<String> {
        Countries::bundled().and_then(|c| c.get(&s.country)).map(|o| o.name.clone()).or_else(|| Owid::bundled()?.country(&s.country).map(|c| c.name.clone()))
    }

    /// Where a state's bar stands, in radians.
    pub fn position(&self, i: u16) -> Option<(f64, f64)> {
        self.list.get(i as usize).map(|s| (s.label[0].to_radians(), s.label[1].to_radians()))
    }

    /// What the information box says about a state: its estimated use by source at the month,
    /// its population and share of the country's, and the country's use.
    pub fn describe(&self, i: u16, month: (i32, u32), shown: [bool; SOURCES], palette: Palette) -> Card {
        let mut card = Card::default();
        let Some(s) = self.list.get(i as usize) else { return card };
        let country = self.country_name(s).unwrap_or_else(|| s.country.clone());
        let share = self.share(i as usize);
        let Some((f, _, used)) = self.estimate(i as usize, month, shown) else {
            card.summary = format!("No electricity figures for {country} at this time.");
            return card;
        };
        let period = match f.basis {
            Basis::Month(y, m) => crate::calibration::month_words((y, m)),
            Basis::Year(y) => format!("{y}, on average"),
        };
        card.summary = format!("{period}: about {} used, {:.1}% of {country}'s by population.", format_power(used), share * 100.0);
        let k = if f.generation > 0.0 { used / f.generation } else { 0.0 };
        let colour = |n: usize| palette.colour(if n < MAPPING.len() { MAPPING[n].1 } else { Source::OtherFossil });
        let fossil = |n: usize| n >= MAPPING.len() || MAPPING[n].1.fossil();
        let name = |n: usize| if n < MAPPING.len() { MAPPING[n].2 } else { "Other fossil" };
        let on = |n: usize| shown[if n < MAPPING.len() { MAPPING[n].1 } else { Source::OtherFossil } as usize];
        let rows: Vec<(usize, f32)> = (0..f.by_series.len()).filter(|&n| on(n)).map(|n| (n, f.by_series[n] * k)).filter(|r| r.1 > 0.0).collect();
        let largest = rows.iter().map(|r| r.1).fold(0.0f32, f32::max);
        let mut by = Meters { title: "Use by the country's mix (est.)".to_string(), max: largest, rows: Vec::new(), caption: String::new() };
        for &(n, kw) in &rows {
            by.rows.push(Meter { label: name(n).to_string(), segments: vec![Part { hatched: fossil(n), ..Part::solid(kw, colour(n)) }], text: format_power(kw) });
        }
        by.caption = "Average power over the period. Hatched: fossil or nuclear.".to_string();
        card.meters.push(by);
        let installed = self.capacity(i as usize, month, shown);
        if let Some(kw) = installed {
            let total: f32 = kw.iter().sum();
            let rows: Vec<(Source, f32)> = Source::ALL.into_iter().map(|s| (s, kw[s as usize])).filter(|r| r.1 > 0.0).collect();
            let mut cap = Meters { title: "Installed capacity (GEM plants)".to_string(), max: rows.iter().map(|r| r.1).fold(0.0, f32::max), rows: Vec::new(), caption: String::new() };
            for (s, v) in rows {
                cap.rows.push(Meter { label: s.name().to_string(), segments: vec![Part { hatched: s.fossil(), ..Part::solid(v, palette.colour(s)) }], text: format_power(v) });
            }
            cap.caption = format!("{} in all: the plants of Global Energy Monitor's tracker inside the outline.", format_power(total));
            if !cap.rows.is_empty() {
                card.meters.push(cap);
            }
        }
        card.facts.push(("Country".to_string(), country.clone()));
        if !s.kind.is_empty() {
            card.facts.push(("Kind".to_string(), s.kind.clone()));
        }
        if !s.iso.is_empty() {
            card.facts.push(("ISO 3166-2".to_string(), s.iso.clone()));
        }
        let people = group_thousands(s.population);
        card.facts.push(("Population".to_string(), if s.estimated { format!("about {people} (est.)") } else { people }));
        card.facts.push(("Share of the country".to_string(), format!("{:.2}%", share * 100.0)));
        let whole = f.demand.filter(|d| *d > 0.0);
        card.facts.push((if whole.is_some() { "Country's use" } else { "Country's generation" }.to_string(), format_power(whole.unwrap_or(f.generation))));
        card.notes.push(format!("Estimated: {country}'s use shared out by population, in the country's mix of sources."));
        if installed.is_some() {
            card.notes.push("Installed capacity: Global Energy Monitor, Global Integrated Power Tracker, September 2026 release (CC BY 4.0); filtered by status and size, so small units and rooftop solar are missing.".to_string());
        }
        if s.estimated {
            card.notes.push("Wikidata has no population for this state: it is estimated from its area and the country's population.".to_string());
        }
        if let Basis::Year(_) = f.basis {
            card.notes.push("Ember has no monthly figures for this month: the year's average stands in, and the bar is drawn lighter.".to_string());
        }
        card.notes.push("Outline: Natural Earth (public domain). Population: Wikidata (CC0). Electricity: Ember via Our World in Data (CC BY 4.0).".to_string());
        card
    }
}

/// Even-odd test over all rings, in degrees.
fn inside(rings: &[Vec<[f64; 2]>], lat: f64, lon: f64) -> bool {
    let mut hit = false;
    for ring in rings {
        let mut prev = ring[ring.len() - 1];
        for &p in ring {
            let ([y0, x0], [y1, x1]) = (prev, p);
            if (y0 > lat) != (y1 > lat) && lon < x0 + (lat - y0) * (x1 - x0) / (y1 - y0) {
                hit = !hit;
            }
            prev = p;
        }
    }
    hit
}

fn polygon(rings: &[Vec<[f64; 2]>]) -> Item {
    let rings = rings.iter().map(|r| r.iter().map(|p| Pos { lat: p[0].to_radians(), lon: p[1].to_radians(), alt: None }).collect()).collect();
    Item::new("", Shape::Polygon(rings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::energy::{Energy, Level};

    fn states() -> &'static States {
        States::bundled().expect("bundled states parse")
    }

    #[test]
    fn outlines_parse_with_a_country_each() {
        let s = states();
        assert!(s.count() > 2500, "{} states", s.count());
        let countries = Countries::bundled().unwrap();
        for st in s.all() {
            assert!(countries.get(&st.country).is_some(), "{} ({}) has no country", st.name, st.country);
            assert!(!st.rings.is_empty() && st.rings.iter().all(|r| r.len() >= 3));
            assert!((-90.0..=90.0).contains(&st.label[0]) && (-180.0..=180.0).contains(&st.label[1]));
        }
        let idf = s.all().iter().find(|st| inside(&st.rings, 48.86, 2.35)).expect("Paris lies in a state");
        assert_eq!((idf.name.as_str(), idf.country.as_str()), ("Île-de-France", "FRA"));
        assert!((idf.label[0] - 48.7).abs() < 0.5 && (idf.label[1] - 2.5).abs() < 0.7, "{:?}", idf.label);
        assert!(s.all().iter().any(|st| st.iso == "US-CA" && st.country == "USA"));
    }

    #[test]
    fn fine_units_are_merged_into_regions_like_germanys_states() {
        let s = states();
        let count = |code: &str| s.all().iter().filter(|st| st.country == code).count();
        assert!((10..=20).contains(&count("FRA")), "France: {}", count("FRA"));
        assert!((18..=22).contains(&count("ITA")), "Italy: {}", count("ITA"));
        assert!((15..=20).contains(&count("ESP")), "Spain: {}", count("ESP"));
        assert!((4..=13).contains(&count("GBR")), "United Kingdom: {}", count("GBR"));
        assert_eq!(count("POL"), 16);
        assert_eq!(count("USA"), 51);
        // Merged outlines are one piece where the region is, not a ring per department.
        let bzh = s.all().iter().find(|st| st.name == "Bretagne").unwrap();
        assert_eq!(bzh.iso, "FR-BRE");
        assert!(bzh.rings.len() <= 3, "{} rings", bzh.rings.len());
        assert!((3_000_000..4_000_000).contains(&bzh.population));
        let ni = s.all().iter().find(|st| st.name == "Northern Ireland").unwrap();
        assert!((1_500_000..2_500_000).contains(&ni.population), "{}", ni.population);
    }

    #[test]
    fn germany_keeps_its_own_states() {
        let s = states();
        assert!(s.all().iter().all(|st| st.country != "DEU"));
        // No world state stands in Germany: its bars at the States level are BKG's alone.
        let de = Countries::bundled().unwrap().get("DEU").unwrap();
        assert!(s.all().iter().all(|st| !de.contains(st.label[0], st.label[1]) || st.country != "DEU"));
        let all = [true; SOURCES];
        let bars = s.bars((2024, 6), all, false);
        assert!(bars.iter().all(|b| matches!(b.subject, Subject::State(_))));
        if let Some(e) = Energy::bundled() {
            let german = e.bars(Level::States, 124 * 12 + 5, all, None);
            assert!(!german.is_empty() && german.iter().all(|b| matches!(b.subject, Subject::Area(_))));
            assert_eq!(german, e.bars(Level::States, 124 * 12 + 5, all, None));
        }
    }

    #[test]
    fn population_sums_near_the_countrys() {
        let s = states();
        let owid = Owid::bundled().unwrap();
        let mut checked = 0;
        for code in ["USA", "CHN", "IND", "BRA", "FRA", "GBR", "ITA", "ESP", "POL", "JPN", "NGA", "IDN", "RUS", "CAN", "AUS", "MEX", "ZAF", "EGY", "TUR", "AUT"] {
            let want = owid.country(code).and_then(|c| c.latest_year(crate::owid::Series::Population, 2024)).map(|p| p.1).unwrap();
            let got = s.country_people(code);
            assert!((got / want - 1.0).abs() < 0.2, "{code}: states sum to {got}, OWID has {want}");
            checked += 1;
        }
        assert_eq!(checked, 20);
        // Every country's shares add up to one.
        let fr: f64 = (0..s.count()).filter(|&i| s.all()[i].country == "FRA").map(|i| s.share(i)).sum();
        assert!((fr - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_states_bar_is_its_countrys_use_by_population() {
        let s = states();
        let all = [true; SOURCES];
        let month = (2024, 6);
        let bars = s.bars(month, all, true);
        let fra: Vec<&Bar> = bars.iter().filter(|b| matches!(b.subject, Subject::State(i) if s.all()[i as usize].country == "FRA")).collect();
        let c = Owid::bundled().unwrap().country("FRA").unwrap();
        let f = Figures::of(c, month).unwrap();
        let used = f.demand.unwrap();
        let sum: f32 = fra.iter().map(|b| b.total()).sum();
        assert!((sum / used - 1.0).abs() < 0.01, "{sum} against {used}");
        assert!(fra.iter().all(|b| (b.demand / b.total() - 1.0).abs() < 1e-3));
        // Brittany: the country's mix, mostly nuclear.
        let i = s.all().iter().position(|st| st.iso == "FR-BRE").unwrap();
        let card = s.describe(i as u16, month, all, Palette::Standard);
        assert!(card.summary.contains("of France's by population"), "{}", card.summary);
        assert!(card.facts.iter().any(|f| f.0 == "Country" && f.1 == "France"));
        assert!(card.facts.iter().any(|f| f.0 == "ISO 3166-2" && f.1 == "FR-BRE"));
        assert!(card.meters[0].rows.iter().any(|r| r.label == "Nuclear"));
        assert_eq!(s.title(i as u16), "Bretagne, France");
    }

    #[test]
    fn the_chosen_state_is_drawn_last() {
        let s = states();
        let mut out = Imported { name: "Energy".to_string(), items: Vec::new(), truncated: false };
        s.outlines(&mut out, Some(Subject::State(5)), (2024, 6), [true; SOURCES], Palette::Standard);
        assert_eq!(out.items.len(), s.count());
        assert_eq!(out.items.last().and_then(|i| i.outline).map(|o| o.width), Some(SELECTED.width));
    }

    #[test]
    fn rejects_other_files() {
        assert!(States::parse(b"ORBC\x01\0\0\0\0\0").is_none());
        assert!(States::parse(b"ORBS\x01\0\0\0\xfa\0\x01\0").is_none());
        assert!(States::parse(b"ORBS\x01\0\0\0\0\0\0\0").is_none());
        assert!(States::parse(b"ORBS\x01\0\0\0\xfa\0\0\0").is_some_and(|s| s.count() == 0));
    }
}
