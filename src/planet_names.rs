//! The IAU names of features on the bodies other than Earth, from the USGS Gazetteer of
//! Planetary Nomenclature (`data/names.tsv`, made by `tools/gen_names.py`): labels for the
//! globe and the map, and places for the search box.
//!
//! A feature is named once it would be a fair part of the view: the camera may be up to
//! [`FEATURE_REACH`] times the feature's diameter away, regions a little further. The label's
//! priority holds the diameter, so that larger features win when labels compete for room.

use crate::search::Place;
use crate::vector::{Label, LabelStyle};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

const TABLE: &str = include_str!("../data/names.tsv");
/// How many diameters away a crater, a mountain or a valley is still named.
const FEATURE_REACH: f64 = 60.0;
/// The same for maria, plains and the other wide regions, named from further off.
const REGION_REACH: f64 = 100.0;
/// A label's priority counts the diameter in steps of this many metres.
const PRIORITY_STEP: f64 = 100.0;
const MAX_RESULTS: usize = 8;

struct Entry {
    label: Label,
    diameter: f64,
}

fn table() -> &'static HashMap<&'static str, (Arc<Vec<Label>>, Vec<Entry>)> {
    static T: OnceLock<HashMap<&'static str, (Arc<Vec<Label>>, Vec<Entry>)>> = OnceLock::new();
    T.get_or_init(|| {
        let mut by_body: HashMap<&'static str, Vec<Entry>> = HashMap::new();
        for line in TABLE.lines() {
            if let Some((body, entry)) = parse(line) {
                by_body.entry(body).or_default().push(entry);
            }
        }
        by_body
            .into_iter()
            .map(|(body, entries)| {
                (
                    body,
                    (
                        Arc::new(entries.iter().map(|e| e.label.clone()).collect()),
                        entries,
                    ),
                )
            })
            .collect()
    })
}

fn parse(line: &'static str) -> Option<(&'static str, Entry)> {
    let mut f = line.split('\t');
    let (body, name, lat, lon, diameter, kind) = (
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
    );
    let (lat, lon, diameter): (f64, f64, f64) =
        (lat.parse().ok()?, lon.parse().ok()?, diameter.parse().ok()?);
    if !(lat.is_finite() && lon.is_finite() && diameter.is_finite() && diameter > 0.0) {
        return None;
    }
    let diameter = diameter * 1000.0;
    let style = if kind == "R" {
        LabelStyle::Region
    } else {
        LabelStyle::Feature
    };
    let priority = -((diameter / PRIORITY_STEP)
        .round()
        .clamp(1.0, i32::MAX as f64 / 4.0) as i32);
    // The Gazetteer gives planetocentric latitudes; the globe is drawn in geodetic ones.
    let lat = crate::geo::from_planetocentric(lat.to_radians(), body);
    let label = Label {
        text: name.to_string(),
        style,
        priority,
        lat,
        lon: lon.to_radians(),
        along: None,
    };
    Some((body, Entry { label, diameter }))
}

/// The names on a body, largest first; none for Earth, whose names come with its map.
pub fn labels(body: &str) -> Arc<Vec<Label>> {
    table().get(body).map(|t| t.0.clone()).unwrap_or_default()
}

/// How far from the camera a feature's name may be shown, in metres.
pub fn reach(label: &Label) -> f64 {
    diameter(label)
        * if label.style == LabelStyle::Region {
            REGION_REACH
        } else {
            FEATURE_REACH
        }
}

/// The diameter of a named feature, in metres, from its label.
pub fn diameter(label: &Label) -> f64 {
    -(label.priority as f64) * PRIORITY_STEP
}

/// A named feature near a place.
#[derive(Clone, Debug, PartialEq)]
pub struct Near {
    pub name: String,
    pub region: bool,
    /// Degrees.
    pub lat: f64,
    pub lon: f64,
    /// Metres.
    pub diameter: f64,
    pub distance: f64,
    /// The place lies within the feature.
    pub inside: bool,
}

/// The named features a place lies in, smallest first, then the nearest others; at most `n`.
/// Positions in radians.
pub fn nearest(body: &str, lat: f64, lon: f64, n: usize) -> Vec<Near> {
    let Some((_, entries)) = table().get(body) else {
        return Vec::new();
    };
    let radius = crate::body::all()
        .iter()
        .find(|b| b.id == body)
        .map_or(1.0, |b| b.a);
    let mut all: Vec<Near> = entries
        .iter()
        .map(|e| {
            let (la, lo) = (e.label.lat, e.label.lon);
            let h = ((la - lat) / 2.0).sin().powi(2)
                + lat.cos() * la.cos() * ((lo - lon) / 2.0).sin().powi(2);
            let distance = 2.0 * h.sqrt().min(1.0).asin() * radius;
            Near {
                name: e.label.text.clone(),
                region: e.label.style == LabelStyle::Region,
                lat: la.to_degrees(),
                lon: lo.to_degrees(),
                diameter: e.diameter,
                distance,
                inside: distance <= e.diameter / 2.0,
            }
        })
        .collect();
    all.sort_by(|a, b| match (a.inside, b.inside) {
        (true, true) => a.diameter.total_cmp(&b.diameter),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => a.distance.total_cmp(&b.distance),
    });
    // A place inside a large region lies inside many; past the first few the nearest serve better.
    let inside: Vec<Near> = all
        .iter()
        .filter(|x| x.inside)
        .take(n / 2)
        .cloned()
        .collect();
    let mut out = inside.clone();
    out.extend(all.into_iter().filter(|x| !x.inside).take(n - inside.len()));
    out
}

/// Names on a body that match what was typed: whole names first, then names that start with
/// it, then the rest that hold it; larger features first within each.
pub fn search(body: &str, query: &str) -> Vec<Place> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let Some((_, entries)) = table().get(body) else {
        return Vec::new();
    };
    let mut found: Vec<(u8, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let name = fold(&e.label.text);
            let rank = if name == q {
                0
            } else if name.starts_with(&q) {
                1
            } else if name.contains(&q) {
                2
            } else {
                return None;
            };
            Some((rank, i))
        })
        .collect();
    found.sort();
    let radius = crate::geo::radius();
    found
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, i)| {
            let e = &entries[i];
            let half = (e.diameter / 2.0 / radius).min(1.5);
            let (lat, lon) = (e.label.lat, e.label.lon);
            let across = half / lat.cos().abs().max(0.05);
            Place {
                name: e.label.text.clone(),
                lat,
                lon,
                bbox: Some([
                    (lat - half).max(-1.57),
                    lon - across,
                    (lat + half).min(1.57),
                    lon + across,
                ]),
            }
        })
        .collect()
}

/// Lower case without accents, so that "tycho" finds Tycho and "lacus" finds Lacūs.
pub(crate) fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ā' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'ē' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'ō' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'ū' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_body_but_earth_has_names_largest_first() {
        assert!(labels("earth").is_empty());
        for body in ["moon", "mars", "mercury", "ceres", "vesta"] {
            let l = labels(body);
            assert!(l.len() > 50, "{body}: {}", l.len());
            assert!(
                l.windows(2).all(|w| w[0].priority <= w[1].priority),
                "{body}"
            );
        }
        assert!(labels("moon").len() > 9000);
    }

    #[test]
    fn big_features_are_named_from_further_away() {
        let mars = labels("mars");
        let olympus = mars.iter().find(|l| l.text == "Olympus Mons").unwrap();
        assert_eq!(olympus.style, LabelStyle::Feature);
        // About 600 km across: named from tens of thousands of kilometres away.
        assert!(
            (3.0e7..4.0e7).contains(&reach(olympus)),
            "{}",
            reach(olympus)
        );
        let centric = crate::geo::to_planetocentric(olympus.lat, "mars").to_degrees();
        assert!((centric - 18.65).abs() < 0.01 && (olympus.lon.to_degrees() + 133.8).abs() < 0.01);
        let moon = labels("moon");
        let imbrium = moon.iter().find(|l| l.text == "Mare Imbrium").unwrap();
        assert_eq!(imbrium.style, LabelStyle::Region);
        let shorty = moon.iter().find(|l| l.text == "Shorty").unwrap();
        assert!(reach(shorty) < 10_000.0 && reach(imbrium) > 1.0e8);
    }

    #[test]
    fn a_place_lists_the_features_it_lies_in_then_the_nearest() {
        let near = nearest(
            "moon",
            (-43.31f64).to_radians(),
            (-11.36f64).to_radians(),
            6,
        );
        assert_eq!(near.len(), 6);
        assert!(near[0].inside, "{near:?}");
        assert!(near.iter().any(|x| x.name == "Tycho" && x.inside));
        let outside: Vec<_> = near.iter().filter(|x| !x.inside).collect();
        assert!(outside.windows(2).all(|w| w[0].distance <= w[1].distance));
        assert!(nearest("earth", 0.0, 0.0, 6).is_empty());
    }

    #[test]
    fn search_finds_whole_names_first_and_ignores_accents() {
        let found = search("moon", "tycho");
        assert_eq!(found[0].name, "Tycho");
        assert!(
            found.len() > 1 && found.len() <= MAX_RESULTS,
            "Tycho A and others follow"
        );
        let b = found[0].bbox.unwrap();
        assert!(b[0] < found[0].lat && found[0].lat < b[2]);
        assert!(search("mars", "VALLES MARINERIS")
            .iter()
            .any(|p| p.name == "Valles Marineris"));
        assert!(search("mars", "  ").is_empty() && search("earth", "Paris").is_empty());
        assert_eq!(fold("Lacūs Mortis"), "lacus mortis");
    }
}
