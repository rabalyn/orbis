//! A list of dated events with a place, for the panel opened with E. The eclipse list is the first
//! source; other feeds (earthquakes, eruptions and the like) add theirs with `EventList::set_source`.

use crate::eclipse::{Eclipse, LunarKind, SolarKind};
use crate::places::Picker;
use crate::sky_time::format_utc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    SolarEclipse,
    LunarEclipse,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub source: &'static str,
    pub kind: Kind,
    /// Unix seconds, UT.
    pub time: f64,
    pub lat: f64,
    pub lon: f64,
    /// How far the camera stands from the place when it flies there, in metres.
    pub distance: f64,
    /// Choosing the event sets the clock to its time and pauses it.
    pub set_clock: bool,
    /// What the row says after the date.
    pub title: String,
    pub detail: String,
    /// What the notice says when the event is shown.
    pub note: String,
}

impl Event {
    pub fn row(&self) -> String {
        format!("{}  {}  {}", format_utc(self.time, false).trim_end_matches(" UTC"), self.title, self.detail)
    }

    pub fn from_eclipse(e: &Eclipse) -> Event {
        const SOLAR_DISTANCE: f64 = 9.0e6;
        const LUNAR_DISTANCE: f64 = 2.6e7;
        match e {
            Eclipse::Solar(s) => {
                let name = match s.kind {
                    SolarKind::Total => "Total solar",
                    SolarKind::Annular => "Annular solar",
                    SolarKind::Hybrid => "Hybrid solar",
                    SolarKind::Partial => "Partial solar",
                };
                let when = format_utc(s.time, true);
                let note = format!(
                    "Greatest eclipse {when}, gamma {:.3}{}",
                    s.gamma,
                    if s.central { "" } else { ". The shadow's axis misses the Earth." }
                );
                let mut detail = format!("mag {:.3}", s.magnitude);
                if let Some(d) = s.duration {
                    detail.push_str(&format!("  {}m{:02}s", (d / 60.0).floor(), (d % 60.0).round().min(59.0)));
                }
                Event {
                    source: "eclipse",
                    kind: Kind::SolarEclipse,
                    time: s.time,
                    lat: s.lat,
                    lon: s.lon,
                    distance: SOLAR_DISTANCE,
                    set_clock: true,
                    title: name.to_string(),
                    detail,
                    note,
                }
            }
            Eclipse::Lunar(l) => {
                let name = match l.kind {
                    LunarKind::Total => "Total lunar",
                    LunarKind::Partial => "Partial lunar",
                    LunarKind::Penumbral => "Penumbral lunar",
                };
                let hours = |s: f64| format!("{}h{:02}m", (s / 3600.0).floor(), ((s % 3600.0) / 60.0).round().min(59.0));
                let mut detail = match l.kind {
                    LunarKind::Penumbral => format!("mag {:.2}", l.penumbral_magnitude),
                    _ => format!("mag {:.2}", l.umbral_magnitude),
                };
                if let Some(d) = l.total_duration.or(l.partial_duration) {
                    detail.push_str(&format!("  {}", hours(d)));
                }
                let mut note = format!("Greatest eclipse {}, gamma {:.3}. In the penumbra for {}", format_utc(l.time, true), l.gamma, hours(l.penumbral_duration));
                if let Some(d) = l.partial_duration {
                    note.push_str(&format!(", partly in the umbra for {}", hours(d)));
                }
                if let Some(d) = l.total_duration {
                    note.push_str(&format!(", wholly for {}", hours(d)));
                }
                note.push('.');
                Event {
                    source: "eclipse",
                    kind: Kind::LunarEclipse,
                    time: l.time,
                    lat: l.lat,
                    lon: l.lon,
                    distance: LUNAR_DISTANCE,
                    set_clock: true,
                    title: name.to_string(),
                    detail,
                    note,
                }
            }
        }
    }
}

/// The events of all sources in time order, with the panel's choice.
#[derive(Default)]
pub struct EventList {
    pub picker: Picker,
    events: Vec<Event>,
}

impl EventList {
    /// Replaces what one source has listed.
    pub fn set_source(&mut self, source: &'static str, events: Vec<Event>) {
        self.events.retain(|e| e.source != source);
        self.events.extend(events);
        self.events.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.picker.clamp(self.events.len());
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn get(&self, i: usize) -> Option<&Event> {
        self.events.get(i)
    }

    /// The row of the event nearest in time to `time`.
    pub fn nearest_to(&self, time: f64) -> Option<usize> {
        (0..self.events.len()).min_by(|&a, &b| (self.events[a].time - time).abs().total_cmp(&(self.events[b].time - time).abs()))
    }

    pub fn panel_text(&self, title: &str) -> String {
        self.picker.panel_text(
            title,
            self.events.len(),
            |i| self.events[i].row(),
            "Nothing listed for these dates.",
            "Enter: show it   Up/Down: choose   E or Esc: close",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(source: &'static str, time: f64) -> Event {
        Event { source, kind: Kind::SolarEclipse, time, lat: 0.0, lon: 0.0, distance: 1.0, set_clock: true, title: "T".into(), detail: String::new(), note: String::new() }
    }

    #[test]
    fn sources_replace_only_their_own_events_and_stay_in_time_order() {
        let mut list = EventList::default();
        list.set_source("a", vec![event("a", 30.0), event("a", 10.0)]);
        list.set_source("b", vec![event("b", 20.0)]);
        assert_eq!((0..3).map(|i| list.get(i).unwrap().time).collect::<Vec<_>>(), [10.0, 20.0, 30.0]);
        list.set_source("a", vec![event("a", 5.0)]);
        assert_eq!((0..list.len()).map(|i| list.get(i).unwrap().time).collect::<Vec<_>>(), [5.0, 20.0]);
        assert_eq!(list.nearest_to(18.0), Some(1));
    }

    #[test]
    fn rows_name_the_date_and_the_kind() {
        let e = Eclipse::Solar(crate::eclipse::Solar {
            kind: SolarKind::Total,
            time: 1_712_600_000.0,
            magnitude: 1.0566,
            gamma: 0.34,
            lat: 0.4,
            lon: -1.8,
            central: true,
            duration: Some(268.0),
        });
        let row = Event::from_eclipse(&e).row();
        assert_eq!(row, "2024-04-08 18:13  Total solar  mag 1.057  4m28s");
    }
}
