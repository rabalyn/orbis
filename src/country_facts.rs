//! The Data tab of the information box: a country's population and electricity for the clock's
//! year, each with a small line over the years, from the Our World in Data figures built into the
//! program (`owid.rs`). Nothing is requested for it.
//!
//! A box is about a country when its Wikidata item is one of Natural Earth's countries
//! (`countries.rs` keeps the item of each), or, for a click on a country's name or on open ground
//! from far enough out to see whole countries, when the click lies inside a country's outline.

use crate::countries::Countries;
use crate::info::Trend;
use crate::owid::{Country, Owid, Series};

/// Where OWID's energy figures are explained, and their licence.
pub const OWID_URL: &str = "https://ourworldindata.org/energy";
pub const CC_BY: &str = "https://creativecommons.org/licenses/by/4.0/";

/// What the Data tab shows about a country.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    /// The code of `owid.rs` and `countries.rs`.
    pub code: String,
    pub name: String,
    /// The clock's year, which the figures are for where they are out.
    pub asked: i32,
    /// Said when some figures are from an earlier year than the one asked for.
    pub note: Option<String>,
    pub rows: Vec<Row>,
}

/// One figure: its line over the years, with the figure and its year in the heading and its
/// source below.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub year: i32,
    pub trend: Trend,
}

/// The country a click's box is about, by its Wikidata item, or else by the outline under the
/// clicked point when `by_outline` (a country's name, or open ground at the scale of countries).
pub fn match_code<'a>(countries: &'a Countries, item: Option<&str>, at: Option<(f64, f64)>, by_outline: bool) -> Option<&'a str> {
    if let Some(o) = item.and_then(|q| countries.by_wikidata(q)) {
        return Some(&o.code);
    }
    let (lat, lon) = at.filter(|_| by_outline)?;
    countries.at(lat, lon).map(|o| o.code.as_str())
}

/// The Data tab for a click, or None if the box is not about a country with figures.
pub fn for_click(item: Option<&str>, at: Option<(f64, f64)>, by_outline: bool, year: i32) -> Option<Facts> {
    let code = match_code(Countries::bundled()?, item, at, by_outline)?;
    facts(Owid::bundled()?, code, year)
}

/// The same country's figures for another year.
pub fn for_year(f: &Facts, year: i32) -> Option<Facts> {
    facts(Owid::bundled()?, &f.code, year)
}

/// A figure's values over the years.
fn series(c: &Country, s: Series) -> Vec<(i32, f64)> {
    let Some((first, last)) = c.years(s) else { return Vec::new() };
    (first..=last).filter_map(|y| c.year(s, y).map(|v| (y, v))).collect()
}

/// Electricity generated per person, in kWh, for the years both are known.
fn per_person(c: &Country) -> Vec<(i32, f64)> {
    series(c, Series::Generation)
        .into_iter()
        .filter_map(|(y, twh)| c.year(Series::Population, y).filter(|&p| p > 0.0).map(|p| (y, twh * 1e9 / p)))
        .collect()
}

/// The country's figures for a year: for each, the latest at or before it. None if it has none.
pub fn facts(owid: &Owid, code: &str, year: i32) -> Option<Facts> {
    let c = owid.country(code)?;
    let population = series(c, Series::Population);
    let mut rows = Vec::new();
    let mut add = |name: &str, points: Vec<(i32, f64)>, value: &dyn Fn(f64) -> String, unit: &str, source: String| {
        let Some(&(y, v)) = points.iter().rev().find(|p| p.0 <= year) else { return };
        let (lo, hi) = points.iter().fold((0.0f64, 0.0f64), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
        // Population is drawn in a unit that keeps the top of the line a whole number.
        let scale = if unit.is_empty() && hi >= 1e7 { 1e6 } else if unit.is_empty() && hi >= 1e4 { 1e3 } else { 1.0 };
        let unit = match scale {
            1e6 => "millions",
            1e3 => "thousands",
            _ if unit.is_empty() => "people",
            _ => unit,
        };
        let when = if y == year { String::new() } else { format!(" ({y})") };
        rows.push(Row {
            year: y,
            trend: Trend {
                title: format!("{name}: {}{when}", value(v)),
                points: points.iter().map(|&(t, v)| (t as f32, (v / scale) as f32)).collect(),
                max: nice(hi / scale),
                min: if lo < 0.0 { -nice(-lo / scale) } else { 0.0 },
                mark: Some(y as f32),
                caption: format!("In {unit}. Source: {source}."),
            },
        });
    };
    let source = |s: Series| move |y: i32| c.year_source(s, y).name().to_string();
    let latest = |p: &[(i32, f64)]| p.iter().rev().find(|q| q.0 <= year).map_or(year, |q| q.0);
    let src = source(Series::Population)(year);
    add("Population", population.clone(), &people, "", src);
    let pp = per_person(c);
    let src = format!("{} and {}", source(Series::Generation)(latest(&pp)), crate::owid::Source::Population.name());
    add("Electricity per person", pp, &|v| format!("{} kWh", group(v.round())), "kWh", src);
    for (name, s, unit, value) in [
        ("Renewable share", Series::RenewableShare, "%", (|v| format!("{v:.1}%")) as fn(f64) -> String),
        ("Carbon intensity", Series::CarbonIntensity, "g CO\u{2082}/kWh", |v| format!("{v:.0} g CO\u{2082}/kWh")),
        ("Net imports", Series::NetImports, "TWh", |v| format!("{v:+.1} TWh")),
    ] {
        let p = series(c, s);
        let src = source(s)(latest(&p));
        add(name, p, &value, unit, src);
    }
    if rows.is_empty() {
        return None;
    }
    let older = rows.iter().map(|r| r.year).filter(|&y| y < year).max();
    let note = older.map(|_| {
        let years: Vec<i32> = rows.iter().map(|r| r.year).collect();
        let newest = *years.iter().max().unwrap_or(&year);
        if years.iter().all(|&y| y == newest) {
            format!("There are no figures for {year} yet; these are for {newest}.")
        } else {
            format!("Not all figures for {year} are out yet; the latest are shown, with their year.")
        }
    });
    Some(Facts { code: code.to_string(), name: c.name.clone(), asked: year, note, rows })
}

/// A round number at or above a value, for the top of a line: 1 to 8 times a power of ten.
fn nice(v: f64) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let p = 10f64.powf(v.log10().floor());
    let m = [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0].into_iter().find(|&m| m * p >= v * 0.999_999).unwrap_or(10.0);
    (m * p) as f32
}

/// Digits in groups of three.
fn group(v: f64) -> String {
    let s = format!("{:.0}", v.abs());
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    if v < 0.0 { format!("-{out}") } else { out }
}

fn people(v: f64) -> String {
    if v >= 1e9 {
        format!("{:.2} billion", v / 1e9)
    } else if v >= 1e6 {
        format!("{:.1} million", v / 1e6)
    } else {
        group(v.round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> (&'static Countries, &'static Owid) {
        (Countries::bundled().unwrap(), Owid::bundled().unwrap())
    }

    #[test]
    fn a_country_is_matched_by_its_wikidata_item() {
        let (c, _) = data();
        assert_eq!(match_code(c, Some("Q142"), None, false), Some("FRA"));
        assert_eq!(match_code(c, Some("Q155"), Some((0.0, 0.0)), false), Some("BRA"));
        // A city's item is no country, and its point is not looked at.
        assert_eq!(match_code(c, Some("Q90"), Some((48.86, 2.35)), false), None);
    }

    #[test]
    fn a_country_is_matched_by_the_outline_under_the_click() {
        let (c, _) = data();
        assert_eq!(match_code(c, None, Some((48.86, 2.35)), true), Some("FRA"));
        assert_eq!(match_code(c, Some("Q90"), Some((-15.8, -47.9)), true), Some("BRA"));
        assert_eq!(match_code(c, None, Some((48.86, 2.35)), false), None);
        assert_eq!(match_code(c, None, Some((0.0, -30.0)), true), None, "open sea");
    }

    #[test]
    fn a_country_without_figures_has_no_tab() {
        let (c, o) = data();
        assert!(facts(o, "XXX", 2025).is_none());
        // Outlines exist for some places OWID has no figures for: none of them gets a tab.
        let without: Vec<&str> = c.all().iter().filter(|x| o.country(&x.code).is_none()).map(|x| x.code.as_str()).collect();
        assert!(!without.is_empty(), "the bundled outlines should include places without figures");
        for code in without {
            assert!(facts(o, code, 2025).is_none(), "{code}");
        }
        assert!(facts(o, "FRA", 1700).is_none(), "before the first figure");
        assert!(facts(o, "FRA", 2020).is_some());
    }

    #[test]
    fn a_year_beyond_the_figures_takes_the_latest_and_says_so() {
        let (_, o) = data();
        let f = facts(o, "FRA", 2100).unwrap();
        assert_eq!(f.rows.len(), 5, "{:?}", f.rows.iter().map(|r| &r.trend.title).collect::<Vec<_>>());
        let last = o.country("FRA").unwrap().years(Series::Population).unwrap().1;
        assert_eq!(f.rows[0].year, last);
        assert!(f.rows.iter().all(|r| r.year < 2100 && r.trend.title.contains(&format!("({})", r.year))));
        assert!(f.note.as_deref().is_some_and(|n| n.contains("2100")), "{:?}", f.note);
        let now = facts(o, "FRA", last).unwrap();
        assert!(now.rows[0].trend.title.starts_with("Population: ") && !now.rows[0].trend.title.contains('('));
    }

    #[test]
    fn figures_read_plainly() {
        let (_, o) = data();
        let f = facts(o, "FRA", 2023).unwrap();
        let t: Vec<&str> = f.rows.iter().map(|r| r.trend.title.as_str()).collect();
        assert!(t[0].contains("million"), "{t:?}");
        assert!(t[1].starts_with("Electricity per person: ") && t[1].ends_with(" kWh"), "{t:?}");
        assert!(f.rows.iter().all(|r| r.trend.caption.contains("Source: ")));
        // France exports: its line reaches below zero.
        assert!(f.rows[4].trend.min < 0.0, "{:?}", f.rows[4].trend);
        assert_eq!(nice(0.0), 0.0);
        assert_eq!(nice(68.2), 80.0);
        assert_eq!(nice(7350.0), 8000.0);
        assert_eq!(nice(1.6), 2.0);
        assert_eq!(nice(9.0), 10.0);
        assert_eq!(group(1234567.0), "1,234,567");
        assert_eq!(people(68_400_000.0), "68.4 million");
    }
}
