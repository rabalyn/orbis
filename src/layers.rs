//! Which kinds of map lines and names are shown. The map data keeps its class on every road
//! and its style on every label, so switching a kind only filters what is already decoded:
//! no request is made, and lines are drawn again from the tiles held in memory.
//!
//! Classes are those of the OpenMapTiles schema (https://openmaptiles.org/schema/): the
//! `transportation` layer's `class` (motorway to service, track, path, rail, transit, ferry) and
//! the `place` layer's `class` (country, state, city, town, village and smaller).

use crate::settings::Settings;
use crate::vector::{LabelStyle, RoadClass};

/// The kinds of line drawn on the tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineKinds {
    pub streets: bool,
    pub trails: bool,
    pub trains: bool,
    pub boundaries: bool,
}

impl LineKinds {
    pub fn of(s: &Settings) -> Self {
        Self { streets: s.streets, trails: s.trails, trains: s.trains, boundaries: s.boundaries }
    }

    pub fn road(&self, class: RoadClass) -> bool {
        match class {
            RoadClass::Path => self.trails,
            RoadClass::Rail => self.trains,
            _ => self.streets,
        }
    }
}

/// The kinds of name placed on the globe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameKinds {
    pub cities: bool,
    pub other: bool,
    /// Street names go with the streets, and need them shown.
    pub streets: bool,
}

impl NameKinds {
    pub fn of(s: &Settings) -> Self {
        Self { cities: s.cities, other: s.other_names, streets: s.streets }
    }

    pub fn any(&self) -> bool {
        self.cities || self.other
    }

    pub fn allows(&self, style: LabelStyle) -> bool {
        match style {
            LabelStyle::Country | LabelStyle::State | LabelStyle::City | LabelStyle::Town | LabelStyle::Village | LabelStyle::Local => {
                self.cities
            }
            LabelStyle::Sea | LabelStyle::Lake | LabelStyle::Peak => self.other,
            LabelStyle::Road => self.other && self.streets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_class_belongs_to_one_switch() {
        let none = LineKinds { streets: false, trails: false, trains: false, boundaries: false };
        let only = |f: fn(&mut LineKinds)| {
            let mut k = none;
            f(&mut k);
            RoadClass::ALL.iter().filter(|c| k.road(**c)).copied().collect::<Vec<_>>()
        };
        assert_eq!(only(|k| k.trails = true), vec![RoadClass::Path]);
        assert_eq!(only(|k| k.trains = true), vec![RoadClass::Rail]);
        let streets = only(|k| k.streets = true);
        assert_eq!(streets.len(), RoadClass::ALL.len() - 2);
        assert!(streets.contains(&RoadClass::Motorway) && streets.contains(&RoadClass::Service) && !streets.contains(&RoadClass::Path));
        assert!(only(|k| k.boundaries = true).is_empty(), "boundaries are not roads");
    }

    #[test]
    fn names_follow_their_switches() {
        let k = NameKinds { cities: true, other: false, streets: true };
        assert!(k.allows(LabelStyle::City) && k.allows(LabelStyle::Country) && k.allows(LabelStyle::Local));
        assert!(!k.allows(LabelStyle::Peak) && !k.allows(LabelStyle::Sea) && !k.allows(LabelStyle::Road));
        let k = NameKinds { cities: false, other: true, streets: false };
        assert!(k.allows(LabelStyle::Lake) && k.allows(LabelStyle::Peak) && !k.allows(LabelStyle::City));
        assert!(!k.allows(LabelStyle::Road), "street names need the streets");
        assert!(k.any() && !NameKinds { cities: false, other: false, streets: true }.any());
    }

    #[test]
    fn the_defaults_show_streets_but_not_trails_or_railways() {
        let s = Settings::default();
        let k = LineKinds::of(&s);
        assert!(RoadClass::ALL.iter().filter(|c| !matches!(c, RoadClass::Path | RoadClass::Rail)).all(|c| k.road(*c)) && k.boundaries);
        assert!(!k.road(RoadClass::Path) && !k.road(RoadClass::Rail));
        assert!(NameKinds::of(&s).allows(LabelStyle::City) && !NameKinds::of(&s).allows(LabelStyle::Road));
    }
}
