//! The simulated clock and the position of the Sun. No window or GPU is involved.
//!
//! Times are seconds since the Unix epoch (UTC, leap seconds ignored), as f64.

use glam::DVec3;
use std::time::{SystemTime, UNIX_EPOCH};
use winit::keyboard::KeyCode;

/// Sun directions closer than this (degrees) look the same, so no redraw is needed.
const REDRAW_ANGLE_DEG: f64 = 0.05;
/// The Sun moves 360° per day of clock time.
const SUN_DEG_PER_SECOND: f64 = 360.0 / 86_400.0;
/// From this speed on, the picture changes fast enough to redraw every frame.
const CONTINUOUS_RATE: f64 = 600.0;
/// Speeds the faster and slower keys step through, in clock seconds per real second.
const RATES: [f64; 9] = [1.0, 10.0, 60.0, 600.0, 3600.0, 21_600.0, 86_400.0, 604_800.0, 2_592_000.0];

pub fn wall_clock_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Where the Sun stands: the direction to it in the Earth-fixed frame, and the point
/// on the ground below it.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    /// Declination in radians.
    pub declination: f64,
    /// Longitude of the subsolar point, east positive, in -π..π.
    pub longitude: f64,
}

impl Sun {
    /// The low-precision solar coordinates of the Astronomical Almanac (also given in
    /// Meeus, "Astronomical Algorithms", ch. 25 in a longer form), good to about 0.01°
    /// of arc from 1950 to 2050. The subsolar longitude is the right ascension minus the
    /// Greenwich mean sidereal time (Almanac, "Sidereal time" low-precision formula).
    pub fn at(unix: f64) -> Self {
        let n = unix / 86_400.0 + 2_440_587.5 - 2_451_545.0;
        let mean_longitude = (280.460 + 0.985_647_4 * n).rem_euclid(360.0);
        let anomaly = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
        let ecliptic_longitude =
            (mean_longitude + 1.915 * anomaly.sin() + 0.020 * (2.0 * anomaly).sin()).to_radians();
        let obliquity = (23.439 - 0.000_000_4 * n).to_radians();
        let right_ascension = (obliquity.cos() * ecliptic_longitude.sin()).atan2(ecliptic_longitude.cos());
        let declination = (obliquity.sin() * ecliptic_longitude.sin()).asin();
        let gmst = (18.697_374_558 + 24.065_709_824_419_08 * n).rem_euclid(24.0) * 15.0;
        let longitude = wrap_pi(right_ascension - gmst.to_radians());
        Self { declination, longitude }
    }

    /// Unit vector towards the Sun in the Earth-fixed frame (x to 0° E on the equator, z north).
    pub fn direction(&self) -> DVec3 {
        let (sd, cd) = self.declination.sin_cos();
        DVec3::new(cd * self.longitude.cos(), cd * self.longitude.sin(), sd)
    }
}

fn wrap_pi(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// A control for the clock, independent of how it was typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockKey {
    PauseResume,
    Faster,
    Slower,
    /// One hour, or a day when large.
    StepBack { large: bool },
    StepForward { large: bool },
    /// Back to the real time, running at normal speed.
    Now,
    /// Switches between sunlight and flat, always-lit imagery; the scene handles it.
    ToggleSun,
}

/// The simulated clock. It is advanced with a real-time reading, so tests control time.
#[derive(Clone, Debug)]
pub struct Clock {
    time: f64,
    rate: f64,
    running: bool,
    last_real: f64,
}

impl Clock {
    pub fn new(time: f64, real: f64) -> Self {
        Self { time, rate: 1.0, running: true, last_real: real }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    #[cfg(test)]
    pub fn rate(&self) -> f64 {
        self.rate
    }

    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn pause(&mut self) {
        self.running = false;
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
    }

    /// Brings the clock up to a reading of real time, in seconds from any fixed start.
    pub fn advance(&mut self, real: f64) {
        if self.running {
            self.time += (real - self.last_real) * self.rate;
        }
        self.last_real = real;
    }

    pub fn apply(&mut self, key: ClockKey, wall_now: f64) {
        match key {
            ClockKey::PauseResume => self.running = !self.running,
            ClockKey::Faster => self.rate = next_rate(self.rate, true),
            ClockKey::Slower => self.rate = next_rate(self.rate, false),
            ClockKey::StepBack { large } => self.time -= step_size(large),
            ClockKey::StepForward { large } => self.time += step_size(large),
            ClockKey::ToggleSun => {}
            ClockKey::Now => {
                self.time = wall_now;
                self.rate = 1.0;
                self.running = true;
            }
        }
    }

    /// True when running fast enough that the picture changes every frame.
    pub fn is_fast(&self) -> bool {
        self.running && self.rate.abs() >= CONTINUOUS_RATE
    }

    /// How many real seconds until the Sun has moved enough from where `drawn` was drawn
    /// to be worth a redraw. `None` when the clock is stopped and nothing will change.
    pub fn redraw_in(&self, drawn: f64) -> Option<f64> {
        let limit = REDRAW_ANGLE_DEG / SUN_DEG_PER_SECOND;
        let moved = (self.time - drawn).abs();
        if moved >= limit {
            return Some(0.0);
        }
        if !self.running || self.rate == 0.0 {
            return None;
        }
        Some((limit - moved) / self.rate.abs())
    }

    /// The date and time in UTC for the on-screen display, with a note on the speed.
    /// Seconds are only shown when they change the display more often than once a minute.
    pub fn label(&self) -> String {
        let seconds = !self.running || self.rate != 1.0;
        let mut text = format_utc(self.time, seconds);
        if !self.running {
            text.push_str("  paused");
        } else if self.rate != 1.0 {
            text.push_str(&format!("  x{}", self.rate));
        }
        text
    }
}

/// The control a key press stands for. Space pauses, - and = slow down and speed up,
/// comma and period step back and forward (a day with Shift), T goes back to now, S toggles sunlight.
pub fn key_for(code: KeyCode, shift: bool) -> Option<ClockKey> {
    Some(match code {
        KeyCode::Space => ClockKey::PauseResume,
        KeyCode::Equal | KeyCode::NumpadAdd => ClockKey::Faster,
        KeyCode::Minus | KeyCode::NumpadSubtract => ClockKey::Slower,
        KeyCode::Comma => ClockKey::StepBack { large: shift },
        KeyCode::Period => ClockKey::StepForward { large: shift },
        KeyCode::KeyT => ClockKey::Now,
        KeyCode::KeyS => ClockKey::ToggleSun,
        _ => return None,
    })
}

fn step_size(large: bool) -> f64 {
    if large {
        86_400.0
    } else {
        3600.0
    }
}

fn next_rate(rate: f64, faster: bool) -> f64 {
    let at = RATES.iter().position(|r| *r >= rate.abs() - 1e-9).unwrap_or(0);
    let at = if faster { (at + 1).min(RATES.len() - 1) } else { at.saturating_sub(1) };
    RATES[at]
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian), after H. Hinnant's
/// public-domain "days_from_civil".
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Formats as `2026-06-21 12:00 UTC`, with seconds when asked.
pub fn format_utc(unix: f64, seconds: bool) -> String {
    let whole = unix.floor() as i64;
    let (days, rest) = (whole.div_euclid(86_400), whole.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rest / 3600, rest / 60 % 60, rest % 60);
    if seconds {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
    } else {
        format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
    }
}

/// Reads an ISO 8601 date and time such as `2026-06-21T12:00:00Z`, `2026-06-21 12:00`
/// or `2026-06-21`. Without an offset the time is UTC.
pub fn parse_iso8601(text: &str) -> Option<f64> {
    let text = text.trim();
    let (date, rest) = match text.find(['T', 't', ' ']) {
        Some(i) => (&text[..i], Some(&text[i + 1..])),
        None => (text, None),
    };
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Rejects dates such as 31 February.
    let days = days_from_civil(y, m, d);
    if civil_from_days(days) != (y, m, d) {
        return None;
    }
    let mut seconds = 0.0;
    let mut offset = 0.0;
    if let Some(rest) = rest {
        let (clock, zone) = match rest.find(['Z', 'z', '+', '-']) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let mut f = clock.split(':');
        let h: f64 = f.next()?.parse().ok()?;
        let min: f64 = f.next()?.parse().ok()?;
        let s: f64 = match f.next() {
            Some(s) => s.parse().ok()?,
            None => 0.0,
        };
        if f.next().is_some() || !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&min) || !(0.0..61.0).contains(&s) {
            return None;
        }
        seconds = h * 3600.0 + min * 60.0 + s;
        if let Some(sign) = zone.chars().next().filter(|c| *c == '+' || *c == '-') {
            let z = zone[1..].replace(':', "");
            if z.len() != 4 && z.len() != 2 {
                return None;
            }
            let zh: f64 = z[..2].parse().ok()?;
            let zm: f64 = if z.len() == 4 { z[2..].parse().ok()? } else { 0.0 };
            offset = (zh * 3600.0 + zm * 60.0) * if sign == '+' { 1.0 } else { -1.0 };
        } else if !zone.is_empty() && !zone.eq_ignore_ascii_case("z") {
            return None;
        }
    }
    Some(days as f64 * 86_400.0 + seconds - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> f64 {
        parse_iso8601(text).unwrap()
    }

    #[test]
    fn parses_dates_and_offsets() {
        assert_eq!(at("1970-01-01T00:00:00Z"), 0.0);
        assert_eq!(at("2000-01-01T12:00:00Z"), 946_728_000.0);
        assert_eq!(at("2000-01-01"), 946_684_800.0);
        assert_eq!(at("2000-01-01 12:00"), 946_728_000.0);
        assert_eq!(at("2000-01-01T14:00:00+02:00"), 946_728_000.0);
        assert_eq!(at("2000-01-01T07:30-0430"), 946_728_000.0);
        assert_eq!(at("2000-01-01T12:00:00.5Z"), 946_728_000.5);
        assert_eq!(at("2024-02-29"), 1_709_164_800.0);
        for bad in ["", "x", "2025-02-29", "2025-13-01", "2025-01-01T25:00", "2025-01-01T12", "2025-01-01T12:00Q", "2025-01"] {
            assert!(parse_iso8601(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn formats_what_it_parses() {
        let t = at("2026-06-21T08:24:09Z");
        assert_eq!(format_utc(t, true), "2026-06-21 08:24:09 UTC");
        assert_eq!(format_utc(t, false), "2026-06-21 08:24 UTC");
        assert_eq!(format_utc(-1.0, true), "1969-12-31 23:59:59 UTC");
    }

    #[test]
    fn sun_at_the_year_2000_epoch() {
        // Astronomical Almanac 2000: on 1 January at 12 UT the Sun's declination is -23.0°.
        let s = Sun::at(at("2000-01-01T12:00:00Z"));
        assert!((s.declination.to_degrees() + 23.0).abs() < 0.1, "{}", s.declination.to_degrees());
        // Equation of time is about -3.2 minutes then: the Sun is 0.8° east of Greenwich.
        assert!((s.longitude.to_degrees() - 0.8).abs() < 0.2, "{}", s.longitude.to_degrees());
    }

    #[test]
    fn solstices_and_equinoxes_within_a_few_hundredths_of_a_degree() {
        // Approximate instants of the 2026 solstices and equinoxes (UTC); the declination there
        // changes by 0.017° an hour at most, so being off by an hour or two is harmless.
        let cases = [
            ("2026-03-20T14:46:00Z", 0.0),
            ("2026-06-21T08:24:00Z", 23.44),
            ("2026-09-23T00:05:00Z", 0.0),
            ("2026-12-21T20:50:00Z", -23.44),
        ];
        for (when, dec) in cases {
            let d = Sun::at(at(when)).declination.to_degrees();
            assert!((d - dec).abs() < 0.06, "{when}: {d}");
        }
    }

    #[test]
    fn subsolar_longitude_at_noon_utc_follows_the_equation_of_time() {
        // At 12:00 UTC the Sun is over Greenwich plus the equation of time: about -1.7 min on
        // 21 June (Sun 0.4° east of Greenwich), +1.9 min on 21 December (0.5° west), and
        // +16.4 min on 3 November (4.1° west).
        for (when, lon) in [("2026-06-21T12:00:00Z", 0.43), ("2026-12-21T12:00:00Z", -0.47), ("2026-11-03T12:00:00Z", -4.1)] {
            let l = Sun::at(at(when)).longitude.to_degrees();
            assert!((l - lon).abs() < 0.3, "{when}: {l}");
        }
        // Six hours later it is a quarter turn to the west.
        let l = Sun::at(at("2026-06-21T18:00:00Z")).longitude.to_degrees();
        assert!((l + 90.0 - 0.43).abs() < 0.3, "{l}");
    }

    #[test]
    fn direction_is_a_unit_vector_over_the_subsolar_point() {
        let s = Sun::at(at("2026-06-21T12:00:00Z"));
        let d = s.direction();
        assert!((d.length() - 1.0).abs() < 1e-12);
        assert!((d.z.asin() - s.declination).abs() < 1e-12);
        assert!(d.x > 0.9 && d.y.abs() < 0.02);
    }

    #[test]
    fn clock_runs_pauses_and_speeds_up() {
        let mut c = Clock::new(1000.0, 50.0);
        c.advance(60.0);
        assert_eq!(c.time(), 1010.0);
        c.apply(ClockKey::PauseResume, 0.0);
        c.advance(100.0);
        assert_eq!(c.time(), 1010.0, "a paused clock stands still");
        c.apply(ClockKey::PauseResume, 0.0);
        c.advance(101.0);
        assert_eq!(c.time(), 1011.0, "time while paused is not made up");
        c.apply(ClockKey::Faster, 0.0);
        assert_eq!(c.rate(), 10.0);
        c.advance(102.0);
        assert_eq!(c.time(), 1021.0);
        for _ in 0..30 {
            c.apply(ClockKey::Faster, 0.0);
        }
        assert_eq!(c.rate(), 2_592_000.0, "speed tops out");
        for _ in 0..30 {
            c.apply(ClockKey::Slower, 0.0);
        }
        assert_eq!(c.rate(), 1.0, "and bottoms out at real speed");
    }

    #[test]
    fn clock_steps_and_resets() {
        let mut c = Clock::new(10_000.0, 0.0);
        c.pause();
        c.apply(ClockKey::StepForward { large: false }, 0.0);
        assert_eq!(c.time(), 13_600.0);
        c.apply(ClockKey::StepBack { large: true }, 0.0);
        assert_eq!(c.time(), 13_600.0 - 86_400.0);
        assert!(!c.is_running(), "stepping leaves a paused clock paused");
        c.set_rate(60.0);
        c.apply(ClockKey::Now, 5_000_000.0);
        assert_eq!((c.time(), c.rate(), c.is_running()), (5_000_000.0, 1.0, true));
    }

    #[test]
    fn redraws_are_rare_at_real_speed_and_constant_when_fast() {
        let mut c = Clock::new(0.0, 0.0);
        let wait = c.redraw_in(0.0).unwrap();
        assert!((5.0..30.0).contains(&wait), "{wait}");
        c.advance(wait + 0.1);
        assert_eq!(c.redraw_in(0.0), Some(0.0));
        assert!(!c.is_fast());
        c.pause();
        assert_eq!(c.redraw_in(c.time()), None, "nothing changes while paused");
        let mut f = Clock::new(0.0, 0.0);
        f.set_rate(3600.0);
        assert!(f.is_fast());
        assert!(f.redraw_in(0.0).unwrap() < 0.05);
    }

    #[test]
    fn keys_map_to_controls() {
        assert_eq!(key_for(KeyCode::Space, false), Some(ClockKey::PauseResume));
        assert_eq!(key_for(KeyCode::Period, true), Some(ClockKey::StepForward { large: true }));
        assert_eq!(key_for(KeyCode::Comma, false), Some(ClockKey::StepBack { large: false }));
        assert_eq!(key_for(KeyCode::KeyS, false), Some(ClockKey::ToggleSun));
        assert_eq!(key_for(KeyCode::KeyT, false), Some(ClockKey::Now));
        assert_eq!(key_for(KeyCode::KeyX, false), None);
    }

    #[test]
    fn label_shows_seconds_only_when_they_matter() {
        let mut c = Clock::new(at("2026-06-21T12:00:07Z"), 0.0);
        assert_eq!(c.label(), "2026-06-21 12:00 UTC");
        c.apply(ClockKey::Faster, 0.0);
        assert_eq!(c.label(), "2026-06-21 12:00:07 UTC  x10");
        c.apply(ClockKey::PauseResume, 0.0);
        assert_eq!(c.label(), "2026-06-21 12:00:07 UTC  paused");
    }
}
