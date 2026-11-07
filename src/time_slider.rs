//! The date arithmetic behind the time sliders, and what is known about the dates they reach.
//! No window or GPU is involved.
//!
//! One bar cannot be centuries wide and minute exact, so there are three tracks: the year (1800
//! to 2050), the day of that year, and the time of day. Each moves only its own part of the
//! date. Times are Unix seconds (UTC, leap seconds ignored), negative before 1970.

use crate::eclipse::{self, Eclipse, LunarKind, SolarKind};
use crate::sky_time::{civil_from_days, days_from_civil};

pub const FIRST_YEAR: i64 = 1800;
pub const LAST_YEAR: i64 = 2050;
/// The last minute of the day that the time track reaches: 23:59.
const LAST_MINUTE: f64 = 1439.0;
/// Minutes moved by one notch of the wheel on the time track.
pub const WHEEL_MINUTES: i64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    Year,
    Day,
    Time,
}

pub fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_year(year: i64) -> i64 {
    if is_leap(year) {
        366
    } else {
        365
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Year, month, day and seconds into the UTC day. The seconds keep their fraction.
fn split(unix: f64) -> (i64, i64, i64, f64) {
    let days = (unix / 86_400.0).floor();
    let (y, m, d) = civil_from_days(days as i64);
    (y, m, d, unix - days * 86_400.0)
}

fn join(year: i64, month: i64, day: i64, secs: f64) -> f64 {
    days_from_civil(year, month, day) as f64 * 86_400.0 + secs
}

fn unit(pos: f64) -> f64 {
    if pos.is_finite() {
        pos.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Where the thumb of a track stands for a time, from 0 to 1. A year outside 1800 to 2050 sits
/// at the nearer end of the year track.
pub fn position(track: Track, unix: f64) -> f64 {
    let (y, m, d, secs) = split(unix);
    match track {
        Track::Year => unit((y - FIRST_YEAR) as f64 / (LAST_YEAR - FIRST_YEAR) as f64),
        Track::Day => {
            let index = days_from_civil(y, m, d) - days_from_civil(y, 1, 1);
            unit(index as f64 / (days_in_year(y) - 1) as f64)
        }
        Track::Time => unit(((secs / 60.0).floor()).min(LAST_MINUTE) / LAST_MINUTE),
    }
}

/// The time with the part that `track` stands for set from a thumb position; the rest is kept.
/// Years are whole, days whole, and the time of day is a whole minute (the seconds go).
/// 29 February becomes 28 February in a year without one.
pub fn set_position(track: Track, unix: f64, pos: f64) -> f64 {
    let pos = unit(pos);
    let (y, m, d, secs) = split(unix);
    match track {
        Track::Year => {
            let year = FIRST_YEAR + (pos * (LAST_YEAR - FIRST_YEAR) as f64).round() as i64;
            with_year(unix, year)
        }
        Track::Day => {
            let index = (pos * (days_in_year(y) - 1) as f64).round() as i64;
            (days_from_civil(y, 1, 1) + index) as f64 * 86_400.0 + secs
        }
        Track::Time => {
            let minute = (pos * LAST_MINUTE).round();
            join(y, m, d, minute * 60.0)
        }
    }
}

/// The same moment in another year.
fn with_year(unix: f64, year: i64) -> f64 {
    let (_, m, d, secs) = split(unix);
    join(year, m, d.min(days_in_month(year, m)), secs)
}

/// Moves a track by whole units (the wheel): a year, a day, or ten minutes, staying inside the
/// year range, the year, and the day.
pub fn step(track: Track, unix: f64, units: i64) -> f64 {
    let (y, m, d, secs) = split(unix);
    match track {
        Track::Year => with_year(unix, (y + units).clamp(FIRST_YEAR, LAST_YEAR)),
        Track::Day => {
            let index = days_from_civil(y, m, d) - days_from_civil(y, 1, 1);
            let index = (index + units).clamp(0, days_in_year(y) - 1);
            (days_from_civil(y, 1, 1) + index) as f64 * 86_400.0 + secs
        }
        Track::Time => {
            let minute = ((secs / 60.0).floor() as i64 + units * WHEEL_MINUTES).clamp(0, LAST_MINUTE as i64);
            join(y, m, d, minute as f64 * 60.0)
        }
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// What a track's thumb stands for, as shown beside it.
pub fn value_label(track: Track, unix: f64) -> String {
    let (y, m, d, secs) = split(unix);
    match track {
        Track::Year => format!("{y}"),
        Track::Day => format!("{d} {}", MONTHS[(m - 1).clamp(0, 11) as usize]),
        Track::Time => format!("{:02}:{:02}", (secs / 3600.0) as i64, (secs / 60.0) as i64 % 60),
    }
}

/// The labels at the two ends of a track.
pub fn end_labels(track: Track) -> (&'static str, &'static str) {
    match track {
        Track::Year => ("1800", "2050"),
        Track::Day => ("1 Jan", "31 Dec"),
        Track::Time => ("00:00", "23:59"),
    }
}

/// The start of the range the planet formulae are stated for, and the end of its last year.
pub fn range() -> (f64, f64) {
    (days_from_civil(FIRST_YEAR, 1, 1) as f64 * 86_400.0, days_from_civil(LAST_YEAR + 1, 1, 1) as f64 * 86_400.0)
}

/// True for a time the planet formulae (JPL's elements, valid 1800 to 2050) do not cover.
pub fn outside_range(unix: f64) -> bool {
    let (from, to) = range();
    !(from..to).contains(&unix)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Positions are known to be off.
    Warning,
    /// Less exact in one respect.
    Note,
}

/// What is less accurate at a time, most important first. Nothing is said about how much unless a
/// source gives it: the Sun's formula is stated to 0.01 degree for 1950 to 2050 only.
pub fn notices(unix: f64) -> Vec<(Level, &'static str)> {
    let mut out = Vec::new();
    if outside_range(unix) {
        out.push((Level::Warning, "Outside 1800-2050: planet positions are approximate."));
        out.push((Level::Note, "The Sun, the Moon and the Earth's turn are less exact there too."));
    } else if unix < days_from_civil(1950, 1, 1) as f64 * 86_400.0 {
        out.push((Level::Note, "Before 1950 the Sun is less exact: its formula is stated for 1950-2050."));
    }
    out
}

/// The text for the clock line when the time is outside the range, or none.
pub fn clock_mark(unix: f64) -> Option<&'static str> {
    outside_range(unix).then_some("approximate: outside 1800-2050")
}

/// The greatest moment of the first eclipse after `time`, searching up to a few years ahead.
pub fn next_eclipse(time: f64) -> Option<f64> {
    let from = time + 30.0;
    [200.0, 400.0, 800.0].iter().find_map(|days| eclipse::between(from, from + days * 86_400.0).first().map(Eclipse::time))
}

/// The greatest moment of the last eclipse before `time`.
pub fn previous_eclipse(time: f64) -> Option<f64> {
    let to = time - 30.0;
    [200.0, 400.0, 800.0].iter().find_map(|days| eclipse::between(to - days * 86_400.0, to).last().map(Eclipse::time))
}

/// Half a day: longer than any eclipse lasts from its first to its last contact.
const HALF_DAY: f64 = 43_200.0;

/// Finds out whether an eclipse is going on at a time, remembering the eclipses around the last
/// time asked about so that dragging a slider does not search again for every step.
#[derive(Default)]
pub struct EclipseWatch {
    /// The span the list of `known` covers.
    covered: Option<(f64, f64)>,
    known: Vec<Eclipse>,
}

fn clock(unix: f64) -> String {
    let (_, _, _, secs) = split(unix);
    format!("{:02}:{:02}", (secs / 3600.0) as i64, (secs / 60.0) as i64 % 60)
}

impl EclipseWatch {
    /// A line naming the eclipse in progress, or none. An eclipse is in progress from the first
    /// to the last contact of the shadow with the Earth, or of the Moon with the Earth's shadow.
    pub fn status(&mut self, time: f64) -> Option<String> {
        if !self.covered.is_some_and(|(a, b)| a <= time - HALF_DAY && time + HALF_DAY <= b) {
            let (a, b) = (time - 3.0 * 86_400.0, time + 3.0 * 86_400.0);
            self.known = eclipse::between(a, b);
            self.covered = Some((a, b));
        }
        self.known.iter().filter(|e| (e.time() - time).abs() < HALF_DAY).find_map(|e| match e {
            Eclipse::Solar(s) if eclipse::shadow_at(time).is_some() => {
                let kind = match s.kind {
                    SolarKind::Total => "Total",
                    SolarKind::Annular => "Annular",
                    SolarKind::Hybrid => "Hybrid",
                    SolarKind::Partial => "Partial",
                };
                Some(format!("{kind} solar eclipse, greatest {} UTC", clock(s.time)))
            }
            Eclipse::Lunar(l) => {
                let (umbral, penumbral) = eclipse::lunar_magnitudes(time);
                if penumbral <= 0.0 {
                    return None;
                }
                let kind = match l.kind {
                    LunarKind::Total => "Total",
                    LunarKind::Partial => "Partial",
                    LunarKind::Penumbral => "Penumbral",
                };
                let phase = if umbral >= 1.0 {
                    "total phase"
                } else if umbral > 0.0 {
                    "partial phase"
                } else {
                    "penumbral phase"
                };
                Some(format!("{kind} lunar eclipse, now in its {phase}"))
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky_time::{format_utc, parse_iso8601, Sun};

    fn at(text: &str) -> f64 {
        parse_iso8601(text).unwrap()
    }

    fn show(unix: f64) -> String {
        format_utc(unix, true)
    }

    #[test]
    fn every_track_round_trips_at_both_ends_and_inside() {
        for text in ["1800-01-01T00:00:00Z", "2050-12-31T23:59:00Z", "1850-03-21T12:00:00Z", "1969-12-31T23:59:00Z", "2026-06-21T16:00:00Z", "2024-02-29T05:30:00Z"] {
            let t = at(text);
            for track in [Track::Year, Track::Day, Track::Time] {
                let p = position(track, t);
                assert!((0.0..=1.0).contains(&p), "{text} {track:?} {p}");
                let back = set_position(track, t, p);
                // Setting the thumb where it already is changes nothing, except a leap day in a year track.
                assert_eq!(show(back), show(t), "{text} {track:?}");
            }
        }
    }

    #[test]
    fn the_ends_of_the_tracks_are_the_ends_of_the_range() {
        let t = at("2026-06-21T16:00:00Z");
        assert_eq!(show(set_position(Track::Year, t, 0.0)), "1800-06-21 16:00:00 UTC");
        assert_eq!(show(set_position(Track::Year, t, 1.0)), "2050-06-21 16:00:00 UTC");
        assert_eq!(show(set_position(Track::Day, t, 0.0)), "2026-01-01 16:00:00 UTC");
        assert_eq!(show(set_position(Track::Day, t, 1.0)), "2026-12-31 16:00:00 UTC");
        assert_eq!(show(set_position(Track::Time, t, 0.0)), "2026-06-21 00:00:00 UTC");
        assert_eq!(show(set_position(Track::Time, t, 1.0)), "2026-06-21 23:59:00 UTC");
        assert_eq!(position(Track::Year, at("1800-01-01T00:00:00Z")), 0.0);
        assert_eq!(position(Track::Year, at("2050-12-31T23:59:59Z")), 1.0);
        assert_eq!(position(Track::Day, at("2026-12-31T00:00:00Z")), 1.0);
        assert_eq!(position(Track::Day, at("2024-12-31T00:00:00Z")), 1.0, "a leap year has one more day");
        assert_eq!(position(Track::Time, at("2026-06-21T23:59:30Z")), 1.0);
    }

    #[test]
    fn a_track_moves_only_its_own_part() {
        let t = at("2026-06-21T16:45:30Z");
        assert_eq!(show(set_position(Track::Year, t, 0.5)), "1925-06-21 16:45:30 UTC");
        // The seconds survive a year or day change; the time track works in whole minutes.
        assert_eq!(show(set_position(Track::Day, t, 0.5)), "2026-07-02 16:45:30 UTC");
        assert_eq!(show(set_position(Track::Time, t, 720.0 / 1439.0)), "2026-06-21 12:00:00 UTC");
    }

    #[test]
    fn the_29th_of_february_becomes_the_28th_in_a_year_without_one() {
        let leap = at("2024-02-29T10:00:00Z");
        // 2025 is not a leap year; 2028 is.
        let to = |year: i64| set_position(Track::Year, leap, (year - FIRST_YEAR) as f64 / 250.0);
        assert_eq!(show(to(2025)), "2025-02-28 10:00:00 UTC");
        assert_eq!(show(to(2028)), "2028-02-29 10:00:00 UTC");
        assert_eq!(show(to(1900)), "1900-02-28 10:00:00 UTC", "1900 was not a leap year");
        assert_eq!(show(to(2000)), "2000-02-29 10:00:00 UTC", "2000 was");
        assert!(is_leap(2000) && !is_leap(1900) && !is_leap(2100) && is_leap(1904));
        // 31 December in a year of 366 days is the last day of the day track.
        assert_eq!(show(set_position(Track::Day, at("2028-05-05T00:00:00Z"), 1.0)), "2028-12-31 00:00:00 UTC");
        assert_eq!(show(set_position(Track::Day, at("2027-05-05T00:00:00Z"), 1.0)), "2027-12-31 00:00:00 UTC");
    }

    #[test]
    fn positions_outside_the_bar_and_odd_numbers_are_clamped() {
        let t = at("2026-06-21T16:00:00Z");
        for track in [Track::Year, Track::Day, Track::Time] {
            assert_eq!(set_position(track, t, -3.0), set_position(track, t, 0.0));
            assert_eq!(set_position(track, t, 7.0), set_position(track, t, 1.0));
            assert_eq!(set_position(track, t, f64::NAN), set_position(track, t, 0.0));
            assert_eq!(set_position(track, t, f64::INFINITY), set_position(track, t, 0.0));
        }
    }

    #[test]
    fn dates_before_1970_work_with_negative_unix_time() {
        let t = at("1850-03-21T12:00:00Z");
        assert!(t < 0.0);
        assert_eq!(show(t), "1850-03-21 12:00:00 UTC");
        assert_eq!(show(-1.0), "1969-12-31 23:59:59 UTC");
        assert_eq!(show(at("1800-01-01T00:00:00Z")), "1800-01-01 00:00:00 UTC");
        assert_eq!(value_label(Track::Time, t), "12:00");
        assert_eq!(value_label(Track::Day, t), "21 Mar");
        assert_eq!(value_label(Track::Year, t), "1850");
        // Half a minute before the epoch is still the last minute of 1969.
        assert_eq!(value_label(Track::Time, -30.0), "23:59");
        assert_eq!(show(set_position(Track::Time, t + 17.5, 0.0)), "1850-03-21 00:00:00 UTC");
        assert_eq!(show(set_position(Track::Year, t, 0.0)), "1800-03-21 12:00:00 UTC");
        assert!((position(Track::Year, t) - 0.2).abs() < 1e-12);
        // The first day of 1800 is before the planets' earliest instant by nothing.
        assert!(!outside_range(at("1800-01-01T00:00:00Z")));
        assert!(outside_range(at("1799-12-31T23:59:59Z")));
        assert!(outside_range(at("1700-01-01T12:00:00Z")) && outside_range(at("2051-01-01T00:00:00Z")));
        assert!(!outside_range(at("2050-12-31T23:59:59Z")));
    }

    #[test]
    fn steps_move_one_unit_and_stop_at_the_ends() {
        let t = at("2026-06-21T16:05:00Z");
        assert_eq!(show(step(Track::Year, t, 1)), "2027-06-21 16:05:00 UTC");
        assert_eq!(show(step(Track::Day, t, -1)), "2026-06-20 16:05:00 UTC");
        assert_eq!(show(step(Track::Time, t, 1)), "2026-06-21 16:15:00 UTC");
        assert_eq!(show(step(Track::Year, at("2050-02-01T00:00:00Z"), 1)), "2050-02-01 00:00:00 UTC");
        assert_eq!(show(step(Track::Year, at("1700-02-01T00:00:00Z"), 1)), "1800-02-01 00:00:00 UTC", "back into the range");
        assert_eq!(show(step(Track::Day, at("2026-12-31T08:00:00Z"), 1)), "2026-12-31 08:00:00 UTC", "a day step stays in the year");
        assert_eq!(show(step(Track::Day, at("2026-01-01T08:00:00Z"), -1)), "2026-01-01 08:00:00 UTC");
        assert_eq!(show(step(Track::Time, at("2026-06-21T23:55:00Z"), 1)), "2026-06-21 23:59:00 UTC");
        assert_eq!(show(step(Track::Time, at("2026-06-21T00:03:00Z"), -1)), "2026-06-21 00:00:00 UTC");
        assert_eq!(show(step(Track::Year, at("2024-02-29T00:00:00Z"), 1)), "2025-02-28 00:00:00 UTC");
    }

    #[test]
    fn dates_outside_the_range_are_marked_and_those_inside_are_not() {
        assert!(clock_mark(at("2026-06-21T16:00:00Z")).is_none());
        assert!(clock_mark(at("1700-01-01T12:00:00Z")).is_some());
        assert!(clock_mark(at("2100-01-01T12:00:00Z")).is_some());
        let warn = |t: &str| notices(at(t)).iter().any(|n| n.0 == Level::Warning);
        assert!(warn("1700-01-01T12:00:00Z") && warn("2060-01-01T00:00:00Z"));
        assert!(!warn("1850-01-01T00:00:00Z") && !warn("2026-06-21T16:00:00Z"));
        // Before 1950 only the milder note, about the Sun; nothing at all in 1950 to 2050.
        assert_eq!(notices(at("1850-01-01T00:00:00Z")).len(), 1);
        assert!(notices(at("1850-01-01T00:00:00Z"))[0].1.contains("1950"));
        assert!(notices(at("1950-01-01T00:00:00Z")).is_empty());
        assert!(notices(at("2026-06-21T16:00:00Z")).is_empty());
    }

    #[test]
    fn the_next_and_previous_eclipse_are_the_ones_on_either_side() {
        let total = at("2024-04-08T18:17:00Z");
        let day_before = at("2024-04-07T12:00:00Z");
        let next = next_eclipse(day_before).unwrap();
        assert!((next - total).abs() < 600.0, "{}", show(next));
        // From the greatest moment itself, next goes on to a later one and previous to an earlier one.
        let after = next_eclipse(next).unwrap();
        assert!(after > next + 86_400.0, "{}", show(after));
        let before = previous_eclipse(next).unwrap();
        assert!(before < next - 86_400.0, "{}", show(before));
        // Previous from just after the total eclipse is the total eclipse.
        let back = previous_eclipse(total + 3600.0).unwrap();
        assert!((back - next).abs() < 1.0);
        // The one before 2024-04-08 is the penumbral lunar eclipse of 2024-03-25, greatest at 07:12 UTC.
        assert!((before - at("2024-03-25T07:12:00Z")).abs() < 1800.0, "{}", show(before));
        // Each is strictly in the direction asked, from many starting points.
        for k in 0..12 {
            let t = at("1850-01-01T00:00:00Z") + k as f64 * 61.0 * 86_400.0;
            assert!(next_eclipse(t).unwrap() > t && previous_eclipse(t).unwrap() < t);
        }
    }

    #[test]
    fn the_eclipse_line_names_an_eclipse_only_while_it_lasts() {
        let mut watch = EclipseWatch::default();
        let greatest = next_eclipse(at("2024-04-07T00:00:00Z")).unwrap();
        let line = watch.status(greatest).unwrap();
        assert!(line.starts_with("Total solar eclipse"), "{line}");
        assert!(watch.status(greatest - 2.0 * 3600.0).is_some(), "the shadow touches the Earth two hours before");
        assert!(watch.status(greatest + 12.0 * 3600.0 - 1.0).is_none());
        assert!(watch.status(at("2024-05-20T12:00:00Z")).is_none());
        // Total lunar eclipse of 2022-11-08, greatest about 11:00 UTC.
        let line = watch.status(at("2022-11-08T10:59:00Z")).unwrap();
        assert!(line.starts_with("Total lunar eclipse") && line.contains("total phase"), "{line}");
        let line = watch.status(at("2022-11-08T08:50:00Z")).unwrap();
        assert!(line.contains("penumbral phase"), "{line}");
    }

    #[test]
    fn the_sun_in_1850_is_where_it_should_be() {
        // At the March equinox the Sun is over the equator, and at 12:00 UT it is over the meridian of Greenwich
        // give or take the equation of time, about -7 minutes on 21 March, which puts it near 2 degrees east.
        let t = at("1850-03-21T12:00:00Z");
        let sun = Sun::at(t);
        assert!(sun.declination.to_degrees().abs() < 1.0, "{}", sun.declination.to_degrees());
        assert!((sun.longitude.to_degrees() - 1.85).abs() < 1.0, "{}", sun.longitude.to_degrees());
        // The same calendar moment 176 years later agrees to a fraction of a degree.
        let now = Sun::at(at("2026-03-21T12:00:00Z"));
        assert!((now.longitude - sun.longitude).to_degrees().abs() < 0.5);
        assert!(sun.direction().is_finite());
        // And the time of the Sun's crossing the meridian a day later is a day later, not a year.
        let next = Sun::at(t + 86_400.0);
        assert!((next.longitude.to_degrees() - sun.longitude.to_degrees()).abs() < 1.0);
    }

    #[test]
    fn the_sky_and_the_shadow_stay_finite_before_1970_and_far_outside_the_range() {
        for text in ["1850-06-21T12:00:00Z", "1800-01-01T00:00:00Z", "1700-01-01T12:00:00Z", "2100-06-21T12:00:00Z"] {
            let t = at(text);
            assert!(t < 4.2e9);
            let bodies = crate::planets::bodies(t);
            assert!(bodies.len() >= 5 && bodies.iter().all(|b| b.dir.is_finite()), "{text}");
            let sm = crate::ephemeris::SunMoon::at(t);
            assert!(sm.sun.is_finite() && sm.moon.is_finite(), "{text}");
            if let Some(e) = eclipse::shadow_at(t) {
                assert!(e.axis.is_finite());
            }
            assert!(crate::ephemeris::delta_t_at(t).is_finite());
        }
    }
}
