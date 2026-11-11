//! Solar and lunar eclipses, computed from the Sun and Moon positions of `ephemeris`.
//!
//! Solar eclipses use the geometry of Besselian elements (Explanatory Supplement to the Astronomical
//! Almanac, ch. 11; Espenak and Meeus, "Five Millennium Canon of Solar Eclipses", NASA/TP-2006-214141),
//! evaluated directly from the positions at any instant instead of from tabulated polynomials. The
//! shadow axis, the cone radii and the Earth are all treated in the Earth-fixed frame, in Earth radii.
//! Lunar eclipses use the Earth's shadow cone at the Moon's distance, with the Earth's radius enlarged by 1/85
//! for the atmosphere (Danjon's rule, see Meeus, "Astronomical Algorithms", ch. 54).
//!
//! Times are Unix seconds (UT); angles are in radians.

use crate::ephemeris::{self, SunMoon};
use crate::geo;
use glam::DVec3;
use std::f64::consts::PI;

/// WGS84 semi-axes in metres; eclipses are only worked out for the Earth.
const A: f64 = 6_378_137.0;
const B: f64 = 6_356_752.314_245;
/// The Earth's equatorial radius, the unit of the Besselian elements.
pub const RE: f64 = A / 1000.0;
const SUN_RADIUS: f64 = 696_000.0;
/// Moon radius over Earth radius, as Espenak and Meeus use it for the penumbra and the umbra.
const K_PENUMBRA: f64 = 0.272_507_6;
const K_UMBRA: f64 = 0.272_281;
/// The Earth's radius is taken as larger by this fraction for the shadow, for the air above it:
/// Danjon's 1/85, which is also the rule that reproduces the catalogs' magnitudes.
const ATMOSPHERE: f64 = 1.0 / 85.0;
const SYNODIC_MONTH: f64 = 29.530_588_861;
/// Polar over equatorial radius.
const C: f64 = B / A;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolarKind {
    Total,
    Annular,
    Hybrid,
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LunarKind {
    Total,
    Partial,
    Penumbral,
}

#[derive(Clone, Copy, Debug)]
pub struct Solar {
    pub kind: SolarKind,
    /// Greatest eclipse: when the shadow axis passes closest to the Earth's centre.
    pub time: f64,
    /// Fraction of the Sun's diameter covered at the greatest eclipse point.
    pub magnitude: f64,
    /// Least distance of the axis from the Earth's centre in Earth radii, positive if north of it.
    pub gamma: f64,
    pub lat: f64,
    pub lon: f64,
    /// Whether the axis touches the Earth.
    pub central: bool,
    /// Seconds of totality or annularity at the greatest eclipse point.
    pub duration: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct Lunar {
    pub kind: LunarKind,
    pub time: f64,
    /// How far into the umbra the Moon's far edge goes, as a fraction of the Moon's diameter (1 and more: total).
    pub umbral_magnitude: f64,
    pub penumbral_magnitude: f64,
    /// Distance of the Moon from the shadow axis in Earth radii.
    pub gamma: f64,
    /// Seconds from the first to the last contact with the penumbra, the umbra, and totality.
    pub penumbral_duration: f64,
    pub partial_duration: Option<f64>,
    pub total_duration: Option<f64>,
    /// The point on the Earth below the Moon at the greatest eclipse.
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Eclipse {
    Solar(Solar),
    Lunar(Lunar),
}

impl Eclipse {
    pub fn time(&self) -> f64 {
        match self {
            Eclipse::Solar(s) => s.time,
            Eclipse::Lunar(l) => l.time,
        }
    }
}

/// The shadow of the Moon at one instant. Lengths are in Earth radii.
#[derive(Clone, Copy, Debug)]
pub struct Elements {
    /// Unit vector of the shadow axis towards the Sun, Earth-fixed.
    pub axis: DVec3,
    /// Where the axis crosses the plane through the Earth's centre at right angles to it.
    pub foot: DVec3,
    /// Declination of the axis.
    pub declination: f64,
    /// Distances along the axis from that plane, towards the Sun, of the penumbra's and the umbra's apex.
    pub penumbra_apex: f64,
    pub umbra_apex: f64,
    pub tan_penumbra: f64,
    pub tan_umbra: f64,
}

impl Elements {
    pub fn at(time: f64) -> Self {
        let sm = SunMoon::at(time);
        let to_fixed = sm.to_earth_fixed();
        let moon = sm.moon;
        let to_sun = sm.sun - moon;
        let distance = to_sun.length();
        let g = to_sun / distance;
        let sin_penumbra = (SUN_RADIUS + K_PENUMBRA * RE) / distance;
        let sin_umbra = (SUN_RADIUS - K_UMBRA * RE) / distance;
        let along = moon.dot(g);
        // East and north unit vectors of the plane, in the equatorial frame.
        let a = g.y.atan2(g.x);
        let east = DVec3::new(-a.sin(), a.cos(), 0.0);
        let north = g.cross(east);
        let foot = (east * moon.dot(east) + north * moon.dot(north)) / RE;
        Self {
            axis: to_fixed * g,
            foot: to_fixed * foot,
            declination: g.z.asin(),
            penumbra_apex: (along + K_PENUMBRA * RE / sin_penumbra) / RE,
            umbra_apex: (along - K_UMBRA * RE / sin_umbra) / RE,
            tan_penumbra: sin_penumbra / (1.0 - sin_penumbra * sin_penumbra).sqrt(),
            tan_umbra: sin_umbra / (1.0 - sin_umbra * sin_umbra).sqrt(),
        }
    }

    /// Distance of the axis from the Earth's centre.
    pub fn gamma(&self) -> f64 {
        self.foot.length()
    }

    /// Radius of the penumbra in the plane through the Earth's centre.
    pub fn penumbra_radius(&self) -> f64 {
        self.penumbra_apex * self.tan_penumbra
    }

    /// Where the axis enters the Earth's surface on the Sun's side, Earth-fixed in Earth radii.
    pub fn axis_hit(&self) -> Option<DVec3> {
        let scale = |v: DVec3| DVec3::new(v.x, v.y, v.z / C);
        let (p, d) = (scale(self.foot), scale(self.axis));
        let (a, b, c) = (d.length_squared(), p.dot(d), p.length_squared() - 1.0);
        let disc = b * b - a * c;
        (disc >= 0.0).then(|| {
            let s = (-b + disc.sqrt()) / a;
            self.foot + self.axis * s
        })
    }

    /// At a point `p` on the Earth: its distance from the axis, the penumbra's radius there, and the
    /// umbra's radius there, negative where the umbra reaches (a total eclipse) and positive where
    /// only the antumbra does (an annular one).
    pub fn local(&self, p: DVec3) -> (f64, f64, f64) {
        let zeta = p.dot(self.axis);
        let q = p - self.foot;
        let m = (q - self.axis * q.dot(self.axis)).length();
        (
            (m),
            (self.penumbra_apex - zeta) * self.tan_penumbra,
            (self.umbra_apex - zeta) * self.tan_umbra,
        )
    }

    /// The fraction of the Sun's diameter covered at a point on the Earth.
    pub fn magnitude_at(&self, p: DVec3) -> f64 {
        let (m, l1, l2) = self.local(p);
        (l1 - m) / (l1 + l2)
    }

    /// Seconds of totality or annularity for someone standing still at `p` around now; zero where
    /// the point is outside the umbra and antumbra.
    fn central_phase_at(&self, p: DVec3) -> f64 {
        let (m, _, l2) = self.local(p);
        l2.abs() - m
    }
}

/// Radius of the Earth's silhouette, seen along the axis, in the direction of the point (x, y) of
/// the fundamental plane: the Earth is an ellipsoid, flatter than a sphere by the polar radius.
fn silhouette_radius(declination: f64, angle: f64) -> f64 {
    let minor = (declination.sin().powi(2) + C * C * declination.cos().powi(2)).sqrt();
    1.0 / (angle.cos().powi(2) + (angle.sin() / minor).powi(2)).sqrt()
}

/// The fundamental plane's east/north coordinates of the axis foot, from the Earth-fixed frame.
/// Used to find the direction in which the axis lies from the Earth's centre.
fn plane_angle(e: &Elements) -> f64 {
    let east = DVec3::Z.cross(e.axis);
    let east = if east.length() < 1e-9 {
        DVec3::X
    } else {
        east.normalize()
    };
    let north = e.axis.cross(east);
    e.foot.dot(north).atan2(e.foot.dot(east))
}

/// The time when `f` is smallest within `centre ± half`, for a function with one minimum there.
fn minimise(centre: f64, half: f64, f: impl Fn(f64) -> f64) -> f64 {
    let (mut lo, mut hi) = (centre - half, centre + half);
    for _ in 0..60 {
        let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if f(a) < f(b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    (lo + hi) / 2.0
}

/// The time between `inside` (where `f` is positive) and `outside` (where it is not) at which it changes.
fn bisect(mut inside: f64, mut outside: f64, f: impl Fn(f64) -> f64) -> f64 {
    for _ in 0..50 {
        let mid = (inside + outside) / 2.0;
        if f(mid) > 0.0 {
            inside = mid;
        } else {
            outside = mid;
        }
    }
    (inside + outside) / 2.0
}

fn wrap_pi(a: f64) -> f64 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

/// The time near `guess` when the Moon's ecliptic longitude is `target` ahead of the Sun's.
fn syzygy(guess: f64, target: f64) -> f64 {
    const RATE: f64 = 12.19 * PI / 180.0 / 86_400.0;
    let mut t = guess;
    for _ in 0..8 {
        let c = ephemeris::centuries_tt(t);
        let f = wrap_pi(
            ephemeris::moon_mean_of_date(c).lon - ephemeris::sun_mean_of_date(c).lon - target,
        );
        t -= f / RATE;
    }
    t
}

fn jd_to_unix(jd: f64) -> f64 {
    (jd - 2_440_587.5) * 86_400.0
}

/// Every eclipse whose greatest phase falls between two Unix times, in order.
pub fn between(start: f64, end: f64) -> Vec<Eclipse> {
    let k_of = |unix: f64| ((unix / 86_400.0 + 2_440_587.5) - 2_451_550.097_66) / SYNODIC_MONTH;
    let (first, last) = (k_of(start).floor() as i64 - 1, k_of(end).ceil() as i64 + 1);
    let mut found = Vec::new();
    for k in first..=last {
        let mean = jd_to_unix(2_451_550.097_66 + SYNODIC_MONTH * k as f64);
        let new_moon = syzygy(mean - ephemeris::delta_t_at(mean), 0.0);
        if let Some(s) = solar_near(new_moon) {
            found.push(Eclipse::Solar(s));
        }
        let full_moon = syzygy(new_moon + SYNODIC_MONTH * 43_200.0, PI);
        if let Some(l) = lunar_near(full_moon) {
            found.push(Eclipse::Lunar(l));
        }
    }
    found.retain(|e| e.time() >= start && e.time() <= end);
    found.sort_by(|a, b| a.time().total_cmp(&b.time()));
    found
}

/// The eclipse nearest in time to `unix`.
pub fn nearest(unix: f64) -> Option<Eclipse> {
    let year = 365.25 * 86_400.0;
    between(unix - 0.6 * year, unix + 0.6 * year)
        .into_iter()
        .min_by(|a, b| (a.time() - unix).abs().total_cmp(&(b.time() - unix).abs()))
}

fn solar_near(new_moon: f64) -> Option<Solar> {
    let time = minimise(new_moon, 6.0 * 3600.0, |t| Elements::at(t).gamma());
    let e = Elements::at(time);
    let gamma = e.gamma();
    let reach = silhouette_radius(e.declination, plane_angle(&e));
    if !e.touches_earth() {
        return None;
    }
    let north = e.foot.dot(DVec3::Z) >= 0.0;
    let signed_gamma = if north { gamma } else { -gamma };
    let hit = e.axis_hit();
    // The greatest eclipse point: where the axis meets the Earth, or the nearest limb if it misses.
    let point = hit.unwrap_or_else(|| {
        let p = e.foot.normalize() * reach;
        let scale = (p.x * p.x + p.y * p.y + p.z * p.z / (C * C)).sqrt();
        p / scale
    });
    let (_, l1, l2) = e.local(point);
    // On the axis the magnitude is the ratio of the Moon's and the Sun's diameters, as the catalogs
    // give it for central eclipses; elsewhere it is the fraction of the Sun's diameter covered.
    let magnitude = if hit.is_some() {
        (l1 - l2) / (l1 + l2)
    } else {
        e.magnitude_at(point)
    };
    let (lat, lon, _) = geo::ecef_to_geodetic(point * A);
    let mut kind = SolarKind::Partial;
    let mut duration = None;
    if hit.is_some() {
        let types = central_types(time);
        kind = match (types.0, types.1) {
            (true, true) => SolarKind::Hybrid,
            (true, false) => SolarKind::Total,
            _ => SolarKind::Annular,
        };
        let phase = |t: f64| Elements::at(t).central_phase_at(point);
        let before = bisect(time, time - 900.0, phase);
        let after = bisect(time, time + 900.0, phase);
        duration = Some(after - before).filter(|d| *d > 0.0 && *d < 1500.0);
    } else if gamma < reach + l2.abs() {
        // The umbra touches the limb without the axis reaching the Earth.
        kind = if l2 < 0.0 {
            SolarKind::Total
        } else {
            SolarKind::Annular
        };
    }
    Some(Solar {
        kind,
        time,
        magnitude,
        gamma: signed_gamma,
        lat,
        lon,
        central: hit.is_some(),
        duration,
    })
}

/// Whether the central line has total and annular stretches: (total somewhere, annular somewhere).
fn central_types(time: f64) -> (bool, bool) {
    let (mut total, mut annular) = (false, false);
    for p in central_line(time, 60.0) {
        if p.total {
            total = true;
        } else {
            annular = true;
        }
    }
    (total, annular)
}

/// A point of the central line.
#[derive(Clone, Copy, Debug)]
pub struct PathPoint {
    pub lat: f64,
    pub lon: f64,
    /// Total rather than annular here.
    pub total: bool,
}

/// The central line of the eclipse whose greatest phase is at `time`, one point every `step`
/// seconds from where the axis first meets the Earth to where it leaves it, ends included.
pub fn central_line(time: f64, step: f64) -> Vec<PathPoint> {
    let point_at = |t: f64| {
        let e = Elements::at(t);
        e.axis_hit().map(|p| {
            let (lat, lon, _) = geo::ecef_to_geodetic(p * A);
            let (_, _, l2) = e.local(p);
            PathPoint {
                lat,
                lon,
                total: l2 < 0.0,
            }
        })
    };
    let hits = |t: f64| if point_at(t).is_some() { 1.0 } else { -1.0 };
    if hits(time) < 0.0 {
        return Vec::new();
    }
    let reach = 4.0 * 3600.0;
    let mut out = Vec::new();
    // Walk outwards from the greatest eclipse in both directions, then join.
    for sign in [-1.0, 1.0] {
        let mut side = Vec::new();
        let mut t = time;
        loop {
            let next = t + sign * step;
            if (next - time).abs() > reach {
                break;
            }
            if hits(next) < 0.0 {
                let edge = bisect(t, next, hits);
                side.extend(point_at(edge));
                break;
            }
            side.extend(point_at(next));
            t = next;
        }
        if sign < 0.0 {
            side.reverse();
            out.extend(side);
            out.extend(point_at(time));
        } else {
            out.extend(side);
        }
    }
    out
}

/// What the Earth's shadow does to the Moon at one instant, as the Moon's distance from the
/// axis and the radii of the cones at the Moon, all in km.
struct LunarShadow {
    distance: f64,
    umbra: f64,
    penumbra: f64,
    moon: f64,
}

fn lunar_shadow(time: f64) -> LunarShadow {
    let sm = SunMoon::at(time);
    let sun_distance = sm.sun.length();
    let toward_sun = sm.sun / sun_distance;
    let behind = -sm.moon.dot(toward_sun);
    // A Moon on the Sun's side of the Earth is in no shadow of the Earth.
    let off_axis = if behind > 0.0 {
        (sm.moon + toward_sun * behind).length()
    } else {
        f64::MAX / 4.0
    };
    let tan = |s: f64| s / (1.0 - s * s).sqrt();
    let earth = RE * (1.0 + ATMOSPHERE);
    let umbra = earth - behind * tan((SUN_RADIUS - RE) / sun_distance);
    let penumbra = earth + behind * tan((SUN_RADIUS + RE) / sun_distance);
    LunarShadow {
        distance: off_axis,
        umbra,
        penumbra,
        moon: K_PENUMBRA * RE,
    }
}

/// Umbral and penumbral magnitudes at an instant: how far the Moon's disc is inside each shadow,
/// in Moon diameters. Positive means some of the disc is in the shadow.
pub fn lunar_magnitudes(time: f64) -> (f64, f64) {
    let s = lunar_shadow(time);
    (
        (s.umbra + s.moon - s.distance) / (2.0 * s.moon),
        (s.penumbra + s.moon - s.distance) / (2.0 * s.moon),
    )
}

fn lunar_near(full_moon: f64) -> Option<Lunar> {
    let time = minimise(full_moon, 6.0 * 3600.0, |t| lunar_shadow(t).distance);
    let (umbral, penumbral) = lunar_magnitudes(time);
    if penumbral <= 0.0 {
        return None;
    }
    let kind = if umbral >= 1.0 {
        LunarKind::Total
    } else if umbral > 0.0 {
        LunarKind::Partial
    } else {
        LunarKind::Penumbral
    };
    // Contacts: where the Moon's edge meets a cone's edge.
    let span = |limit: &dyn Fn(&LunarShadow) -> f64| {
        let f = |t: f64| {
            let s = lunar_shadow(t);
            limit(&s) - s.distance
        };
        let before = bisect(time, time - 5.0 * 3600.0, f);
        let after = bisect(time, time + 5.0 * 3600.0, f);
        after - before
    };
    let penumbral_duration = span(&|s| s.penumbra + s.moon);
    let partial_duration = (umbral > 0.0).then(|| span(&|s| s.umbra + s.moon));
    let total_duration = (umbral >= 1.0).then(|| span(&|s| s.umbra - s.moon));
    let sm = SunMoon::at(time);
    let below = sm.to_earth_fixed() * sm.moon.normalize();
    let (lat, lon, _) = geo::ecef_to_geodetic(below * A);
    Some(Lunar {
        kind,
        time,
        umbral_magnitude: umbral,
        penumbral_magnitude: penumbral,
        gamma: lunar_shadow(time).distance / RE,
        penumbral_duration,
        partial_duration,
        total_duration,
        lat,
        lon,
    })
}

/// The colour the Moon's disc is multiplied by during a lunar eclipse, or `None` outside the Earth's
/// shadow. The penumbra only dims it a little; in the umbra it turns dark copper, the light that the
/// Earth's air bends round the shadow.
pub fn moon_tint(time: f64) -> Option<[f32; 3]> {
    let (umbral, penumbral) = lunar_magnitudes(time);
    if penumbral <= 0.0 {
        return None;
    }
    let pen = (penumbral as f32).clamp(0.0, 1.0);
    let um = (umbral as f32).clamp(0.0, 1.0);
    const COPPER: [f32; 3] = [0.30, 0.075, 0.03];
    Some(std::array::from_fn(|i| {
        (1.0 - 0.25 * pen) * (1.0 + (COPPER[i] - 1.0) * um)
    }))
}

impl Elements {
    /// Whether the penumbra reaches the Earth: any part of it can be eclipsed.
    pub fn touches_earth(&self) -> bool {
        self.gamma()
            < silhouette_radius(self.declination, plane_angle(self)) + self.penumbra_radius()
    }
}

/// The Moon's shadow at an instant, if the penumbra falls on the Earth.
pub fn shadow_at(time: f64) -> Option<Elements> {
    Some(Elements::at(time)).filter(Elements::touches_earth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky_time::parse_iso8601;

    fn at(text: &str) -> f64 {
        parse_iso8601(text).unwrap()
    }

    fn solar(text: &str) -> Solar {
        let t = at(text);
        match between(t - 86_400.0, t + 86_400.0).first() {
            Some(Eclipse::Solar(s)) => *s,
            other => panic!("no solar eclipse on {text}: {other:?}"),
        }
    }

    fn lunar(text: &str) -> Lunar {
        let t = at(text);
        match between(t - 86_400.0, t + 86_400.0).first() {
            Some(Eclipse::Lunar(l)) => *l,
            other => panic!("no lunar eclipse on {text}: {other:?}"),
        }
    }

    /// Great-circle distance in km.
    fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
        let (la, lb) = (a.0.to_radians(), b.0.to_radians());
        let d = (la.sin() * lb.sin() + la.cos() * lb.cos() * (a.1 - b.1).to_radians().cos())
            .clamp(-1.0, 1.0);
        d.acos() * 6371.0
    }

    // The published values below are from the NASA/GSFC catalogs of Fred Espenak (Five Millennium
    // Catalog of Solar Eclipses, and the Lunar Eclipse catalog for 2021-2100): the time of greatest
    // eclipse in UT, the gamma, the magnitude, and for solar eclipses the greatest eclipse point.
    // Tolerances: 1 minute for solar times, 2 minutes for lunar; 0.01 in gamma and solar magnitude,
    // 0.02 in lunar magnitude; 150 km for the greatest eclipse point.

    #[test]
    fn total_eclipse_of_2017() {
        let s = solar("2017-08-21T00:00:00Z");
        assert_eq!(s.kind, SolarKind::Total);
        assert!(
            (s.time - at("2017-08-21T18:25:32Z")).abs() < 60.0,
            "{}",
            s.time - at("2017-08-21T18:25:32Z")
        );
        assert!((s.gamma - 0.4367).abs() < 0.003, "{}", s.gamma);
        assert!((s.magnitude - 1.0306).abs() < 0.003, "{}", s.magnitude);
        assert!(km((s.lat.to_degrees(), s.lon.to_degrees()), (37.0, -87.7)) < 150.0);
        assert!(
            (s.duration.unwrap() - 160.0).abs() < 10.0,
            "{:?}",
            s.duration
        );
    }

    #[test]
    fn total_eclipse_of_2024() {
        let s = solar("2024-04-08T00:00:00Z");
        assert_eq!(s.kind, SolarKind::Total);
        assert!(
            (s.time - at("2024-04-08T18:17:16Z")).abs() < 60.0,
            "{}",
            s.time - at("2024-04-08T18:17:16Z")
        );
        assert!((s.gamma - 0.3431).abs() < 0.003, "{}", s.gamma);
        assert!((s.magnitude - 1.0566).abs() < 0.003, "{}", s.magnitude);
        assert!(km((s.lat.to_degrees(), s.lon.to_degrees()), (25.3, -104.1)) < 150.0);
        assert!(
            (s.duration.unwrap() - 268.0).abs() < 10.0,
            "{:?}",
            s.duration
        );
    }

    #[test]
    fn annular_eclipse_of_2023() {
        let s = solar("2023-10-14T00:00:00Z");
        assert_eq!(s.kind, SolarKind::Annular);
        assert!(
            (s.time - at("2023-10-14T17:59:32Z")).abs() < 120.0,
            "{}",
            s.time - at("2023-10-14T17:59:32Z")
        );
        assert!((s.gamma - 0.3753).abs() < 0.003, "{}", s.gamma);
        assert!((s.magnitude - 0.952).abs() < 0.003, "{}", s.magnitude);
        assert!(
            (s.duration.unwrap() - 317.0).abs() < 15.0,
            "{:?}",
            s.duration
        );
    }

    #[test]
    fn hybrid_eclipse_of_april_2023() {
        let s = solar("2023-04-20T00:00:00Z");
        assert_eq!(s.kind, SolarKind::Hybrid);
        assert!(
            (s.time - at("2023-04-20T04:16:44Z")).abs() < 60.0,
            "{}",
            s.time - at("2023-04-20T04:16:44Z")
        );
        assert!((s.magnitude - 1.0132).abs() < 0.003, "{}", s.magnitude);
    }

    #[test]
    fn total_eclipse_of_2026() {
        let s = solar("2026-08-12T00:00:00Z");
        assert_eq!(s.kind, SolarKind::Total);
        assert!(
            (s.time - at("2026-08-12T17:46:06Z")).abs() < 60.0,
            "{}",
            s.time - at("2026-08-12T17:46:06Z")
        );
        assert!((s.gamma - 0.8977).abs() < 0.003, "{}", s.gamma);
        assert!((s.magnitude - 1.0386).abs() < 0.003, "{}", s.magnitude);
        assert!(km((s.lat.to_degrees(), s.lon.to_degrees()), (65.2, -25.2)) < 150.0);
    }

    #[test]
    fn partial_eclipse_of_2025() {
        let s = solar("2025-03-29T00:00:00Z");
        assert_eq!(s.kind, SolarKind::Partial);
        assert!(
            (s.time - at("2025-03-29T10:47:30Z")).abs() < 120.0,
            "{}",
            s.time - at("2025-03-29T10:47:30Z")
        );
        assert!((s.magnitude - 0.9376).abs() < 0.005, "{}", s.magnitude);
        assert!(!s.central);
    }

    #[test]
    fn lunar_eclipses_of_2022_and_2025() {
        let l = lunar("2022-11-08T00:00:00Z");
        assert_eq!(l.kind, LunarKind::Total);
        assert!(
            (l.time - at("2022-11-08T11:00:22Z")).abs() < 120.0,
            "{}",
            l.time - at("2022-11-08T11:00:22Z")
        );
        assert!(
            (l.umbral_magnitude - 1.3589).abs() < 0.02,
            "{}",
            l.umbral_magnitude
        );
        assert!(
            (l.total_duration.unwrap() - 85.0 * 60.0).abs() < 180.0,
            "{:?}",
            l.total_duration
        );
        let l = lunar("2025-03-14T00:00:00Z");
        assert_eq!(l.kind, LunarKind::Total);
        assert!(
            (l.time - at("2025-03-14T06:58:46Z")).abs() < 120.0,
            "{}",
            l.time - at("2025-03-14T06:58:46Z")
        );
        assert!(
            (l.umbral_magnitude - 1.1804).abs() < 0.02,
            "{}",
            l.umbral_magnitude
        );
        assert!(
            (l.total_duration.unwrap() - 65.0 * 60.0).abs() < 180.0,
            "{:?}",
            l.total_duration
        );
    }

    #[test]
    fn partial_and_penumbral_lunar_eclipses_of_2024() {
        let l = lunar("2024-03-25T00:00:00Z");
        assert_eq!(l.kind, LunarKind::Penumbral);
        assert!(
            (l.penumbral_magnitude - 0.9576).abs() < 0.02,
            "{}",
            l.penumbral_magnitude
        );
        let l = lunar("2024-09-18T00:00:00Z");
        assert_eq!(l.kind, LunarKind::Partial);
        assert!(
            (l.umbral_magnitude - 0.0869).abs() < 0.02,
            "{}",
            l.umbral_magnitude
        );
    }

    /// The eclipses of the year, as the NASA catalogs list them: no more, no fewer.
    fn kinds(year_start: &str, year_end: &str) -> Vec<String> {
        between(at(year_start), at(year_end))
            .iter()
            .map(|e| match e {
                Eclipse::Solar(s) => format!("S {:?}", s.kind),
                Eclipse::Lunar(l) => format!("L {:?}", l.kind),
            })
            .collect()
    }

    #[test]
    fn the_scan_finds_exactly_the_eclipses_of_2024() {
        // Mar 25 penumbral, Apr 8 total, Sep 18 partial, Oct 2 annular.
        assert_eq!(
            kinds("2024-01-01T00:00:00Z", "2025-01-01T00:00:00Z"),
            ["L Penumbral", "S Total", "L Partial", "S Annular"]
        );
    }

    #[test]
    fn the_scan_finds_exactly_the_eclipses_of_other_years() {
        // 2023: Apr 20 hybrid, May 5 penumbral, Oct 14 annular, Oct 28 partial.
        assert_eq!(
            kinds("2023-01-01T00:00:00Z", "2024-01-01T00:00:00Z"),
            ["S Hybrid", "L Penumbral", "S Annular", "L Partial"]
        );
        // 2025: Mar 14 total, Mar 29 partial, Sep 7 total, Sep 21 partial.
        assert_eq!(
            kinds("2025-01-01T00:00:00Z", "2026-01-01T00:00:00Z"),
            ["L Total", "S Partial", "L Total", "S Partial"]
        );
        // 2026: Feb 17 annular, Mar 3 total, Aug 12 total, Aug 28 partial.
        assert_eq!(
            kinds("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z"),
            ["S Annular", "L Total", "S Total", "L Partial"]
        );
        // 2018: three partial solar eclipses and two lunar ones (Jan 31 and Jul 27 total), five in all.
        assert_eq!(
            kinds("2018-01-01T00:00:00Z", "2019-01-01T00:00:00Z").len(),
            5
        );
    }

    // Rows of the central line table of the 2024 April 8 eclipse in the NASA/GSFC eclipse bulletin
    // (eclipse.gsfc.nasa.gov/SEpath/SEpath2001/SE2024Apr08Tpath.html): UT, latitude and longitude in
    // degrees and minutes, and the duration of totality there in seconds.
    const NASA_2024_LINE: [(&str, f64, f64, f64); 8] = [
        ("17:30", 11.0 + 26.4 / 60.0, -(117.0 + 12.6 / 60.0), 242.7),
        ("17:50", 17.0 + 24.5 / 60.0, -(111.0 + 25.8 / 60.0), 259.1),
        ("18:10", 23.0 + 11.9 / 60.0, -(106.0 + 6.8 / 60.0), 267.2),
        ("18:20", 26.0 + 3.3 / 60.0, -(103.0 + 23.6 / 60.0), 268.2),
        ("18:40", 31.0 + 43.2 / 60.0, -(97.0 + 22.2 / 60.0), 263.7),
        ("19:00", 37.0 + 19.7 / 60.0, -(89.0 + 46.6 / 60.0), 250.0),
        ("19:20", 42.0 + 47.8 / 60.0, -(78.0 + 59.2 / 60.0), 225.9),
        ("19:40", 47.0 + 40.1 / 60.0, -(60.0 + 44.7 / 60.0), 187.7),
    ];

    #[test]
    fn central_line_of_2024_follows_the_nasa_table() {
        let t = solar("2024-04-08T00:00:00Z").time;
        let line = central_line(t, 20.0);
        assert!(line.len() > 300, "{}", line.len());
        let mut worst: f64 = 0.0;
        for (time, lat, lon, _seconds) in NASA_2024_LINE {
            // How far the table's point is from the computed line.
            let nearest = line
                .iter()
                .map(|p| km((lat, lon), (p.lat.to_degrees(), p.lon.to_degrees())))
                .fold(f64::MAX, f64::min);
            eprintln!("{time}: {nearest:.1} km from the line");
            worst = worst.max(nearest);
            assert!(nearest < 8.0, "{time}: {nearest} km from the line");
        }
        // About 3 km in practice: the table is one point per two minutes, and rounds to 0.1 minute of arc.
        eprintln!("worst distance from the line: {worst:.1} km");
    }

    #[test]
    fn shadow_darkens_only_near_the_axis() {
        let s = solar("2024-04-08T00:00:00Z");
        let e = Elements::at(s.time);
        let hit = e.axis_hit().unwrap();
        assert!(e.magnitude_at(hit) > 1.0, "{}", e.magnitude_at(hit));
        let away = (hit + DVec3::new(0.0, 0.0, 0.2)).normalize();
        assert!(e.magnitude_at(away) < 0.9);
        assert!(moon_tint(s.time).is_none());
    }

    #[test]
    fn the_moon_goes_dark_in_total_lunar_eclipse() {
        let l = lunar("2025-03-14T00:00:00Z");
        let tint = moon_tint(l.time).unwrap();
        assert!(tint[0] < 0.4 && tint[2] < tint[0] / 5.0, "{tint:?}");
        assert!(moon_tint(l.time - 6.0 * 3600.0).is_none());
    }
}
