//! The Moon's orientation in space and the Sun as seen in its body-fixed frame. No window
//! or GPU is involved.
//!
//! The rotation model is the IAU Working Group on Cartographic Coordinates and Rotational
//! Elements report: Archinal et al., Celestial Mechanics and Dynamical Astronomy 109 (2011)
//! 101-135, Table 2, unchanged for the Moon in the 2015 report (CMDA 130, 2018, article 22).
//! The numbers below were checked against NASA NAIF's `pck00011.tpc` (BODY301 and
//! BODY3_NUT_PREC_ANGLES). The body-fixed frame has its x axis through the mean Earth
//! direction at 0° longitude, z through the north pole, and longitude counted east.

use crate::sky::{equatorial, julian_centuries, precession};
use crate::sky_time::equatorial_of_date;
use glam::{DMat3, DVec3};
use std::f64::consts::PI;

/// Nutation and precession arguments E1..E13: epoch angle and rate per day, in degrees.
const ARGUMENTS: [(f64, f64); 13] = [
    (125.045, -0.052_992_1),
    (250.089, -0.105_984_2),
    (260.008, 13.012_000_9),
    (176.625, 13.340_715_4),
    (357.529, 0.985_600_3),
    (311.589, 26.405_708_4),
    (134.963, 13.064_993_0),
    (276.617, 0.328_714_6),
    (34.226, 1.748_487_7),
    (15.134, -0.158_976_3),
    (119.743, 0.003_609_6),
    (239.961, 0.164_357_3),
    (25.053, 12.959_008_8),
];

/// Coefficients of sin(E1..E13) for the pole's right ascension.
const RA_TERMS: [f64; 13] = [
    -3.8787, -0.1204, 0.0700, -0.0172, 0.0, 0.0072, 0.0, 0.0, 0.0, -0.0052, 0.0, 0.0, 0.0043,
];
/// Coefficients of cos(E1..E13) for the pole's declination.
const DEC_TERMS: [f64; 13] = [
    1.5419, 0.0239, -0.0278, 0.0068, 0.0, -0.0029, 0.0009, 0.0, 0.0, 0.0008, 0.0, 0.0, -0.0009,
];
/// Coefficients of sin(E1..E13) for the prime meridian angle.
const W_TERMS: [f64; 13] = [
    3.5610, 0.1208, -0.0642, 0.0158, 0.0252, -0.0066, -0.0047, -0.0046, 0.0028, 0.0052, 0.0040,
    0.0019, -0.0044,
];

/// Right ascension and declination of the north pole (ICRF, about J2000), and the prime
/// meridian angle W, all in radians. UT is taken as TDB; the difference is about a minute,
/// which turns the Moon by a few hundredths of a degree.
pub fn pole_and_meridian(unix: f64) -> (f64, f64, f64) {
    let d = unix / 86_400.0 + 2_440_587.5 - 2_451_545.0;
    let t = d / 36_525.0;
    let e: Vec<f64> = ARGUMENTS
        .iter()
        .map(|(a, rate)| (a + rate * d).to_radians())
        .collect();
    let sum = |terms: &[f64; 13], f: fn(f64) -> f64| -> f64 {
        terms.iter().zip(&e).map(|(c, e)| c * f(*e)).sum()
    };
    let ra = 269.9949 + 0.0031 * t + sum(&RA_TERMS, f64::sin);
    let dec = 66.5392 + 0.0130 * t + sum(&DEC_TERMS, f64::cos);
    let w = 38.3213 + 13.176_358_15 * d - 1.4e-12 * d * d + sum(&W_TERMS, f64::sin);
    (
        ra.to_radians(),
        dec.to_radians(),
        w.rem_euclid(360.0).to_radians(),
    )
}

/// From J2000 equatorial directions to the Moon-fixed frame: the IAU report's
/// R3(W) R1(90° - dec) R3(90° + ra), written for rotating vectors.
pub fn fixed_frame(unix: f64) -> DMat3 {
    let (ra, dec, w) = pole_and_meridian(unix);
    DMat3::from_rotation_z(-w)
        * DMat3::from_rotation_x(-(PI / 2.0 - dec))
        * DMat3::from_rotation_z(-(PI / 2.0 + ra))
}

/// The Sun's direction in J2000 equatorial coordinates, as seen from the Earth-Moon system.
/// Seen from the Moon it differs by under 0.15° (the Moon's distance over one au).
pub fn sun_j2000(unix: f64) -> DVec3 {
    let (ra, dec) = equatorial_of_date(unix);
    (precession(julian_centuries(unix)).transpose() * equatorial(ra, dec)).normalize()
}

/// Unit vector towards the Sun in the Moon-fixed frame.
pub fn sun_fixed(unix: f64) -> DVec3 {
    fixed_frame(unix) * sun_j2000(unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planets;
    use crate::sky_time::parse_iso8601;

    fn at(s: &str) -> f64 {
        parse_iso8601(s).unwrap()
    }

    fn lat_lon(v: DVec3) -> (f64, f64) {
        (v.z.asin().to_degrees(), v.y.atan2(v.x).to_degrees())
    }

    #[test]
    fn j2000_values_are_the_published_ones() {
        // At J2000 the report gives the pole as 269.9949 + (-3.8787 sin 125.045 ...) etc.
        let (ra, dec, w) = pole_and_meridian(at("2000-01-01T12:00:00Z"));
        assert!(
            (ra.to_degrees() - 266.855).abs() < 0.01,
            "{}",
            ra.to_degrees()
        );
        assert!(
            (dec.to_degrees() - 65.6411).abs() < 0.01,
            "{}",
            dec.to_degrees()
        );
        assert!(
            (w.to_degrees() - 41.1954).abs() < 0.01,
            "{}",
            w.to_degrees()
        );
    }

    #[test]
    fn the_pole_maps_to_the_z_axis() {
        for s in [
            "2000-01-01T12:00:00Z",
            "2026-03-01T00:00:00Z",
            "2040-09-09T09:09:00Z",
        ] {
            let (ra, dec, _) = pole_and_meridian(at(s));
            let pole = equatorial(ra, dec);
            let p = fixed_frame(at(s)) * pole;
            assert!((p - DVec3::Z).length() < 1e-12, "{p:?}");
        }
    }

    #[test]
    fn the_frame_turns_once_a_sidereal_month() {
        // The Moon is locked to the Earth: the frame turns 360 degrees in 27.32 days (13.1764 deg/day).
        let t0 = at("2026-03-01T00:00:00Z");
        let day = 86_400.0;
        let turn = |dt: f64| {
            let a = fixed_frame(t0) * DVec3::X;
            let b = fixed_frame(t0 + dt) * DVec3::X;
            (a.dot(b)).clamp(-1.0, 1.0).acos().to_degrees()
        };
        // The x axis (a vector near the equator) moves about 13 degrees a day, less the pole's tilt.
        assert!((turn(day) - 13.18).abs() < 1.0, "{}", turn(day));
    }

    #[test]
    fn the_earth_stays_near_the_middle_of_the_near_side() {
        // Libration keeps the Earth within about 8 degrees of longitude and 7 of latitude of
        // the centre of the disc (the mean Earth direction), whatever the date.
        for s in [
            "2000-01-01T12:00:00Z",
            "2013-05-05T05:05:00Z",
            "2024-02-29T18:00:00Z",
            "2026-03-01T00:00:00Z",
            "2026-09-17T21:30:00Z",
            "2031-12-24T07:00:00Z",
        ] {
            let t = at(s);
            let moon = planets::bodies(t)
                .into_iter()
                .find(|b| b.name == "Moon")
                .unwrap();
            let earth = fixed_frame(t) * (-moon.dir);
            let (lat, lon) = lat_lon(earth);
            assert!(
                lat.abs() < 8.5 && lon.abs() < 9.5,
                "{s}: sub-Earth point {lat:.2} {lon:.2}"
            );
        }
    }

    #[test]
    fn the_subsolar_point_follows_the_phase() {
        // Full Moon (2025-05-12 16:56 UTC): the Sun is over the near side's middle.
        let (lat, lon) = lat_lon(sun_fixed(at("2025-05-12T16:56:00Z")));
        assert!(lon.abs() < 12.0 && lat.abs() < 3.0, "{lat} {lon}");
        // New Moon (2025-05-27 03:02 UTC): the far side's middle is lit.
        let (_, lon) = lat_lon(sun_fixed(at("2025-05-27T03:02:00Z")));
        assert!(lon.abs() > 168.0, "{lon}");
        // First quarter (2025-06-03 03:41 UTC): the Sun stands over 90 degrees east, where
        // the morning terminator has just passed.
        let (_, lon) = lat_lon(sun_fixed(at("2025-06-03T03:41:00Z")));
        assert!((lon - 90.0).abs() < 15.0, "{lon}");
    }
}
