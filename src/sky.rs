//! The sky's contents: the star table, sidereal time and precession. No window or GPU is involved.
//!
//! Directions are unit vectors in the equatorial frame of J2000 (x towards the vernal equinox,
//! z towards the celestial pole), unless stated otherwise. `earth_fixed` turns them into the
//! Earth-fixed frame used everywhere else (x to 0° E on the equator, z north).

use bytemuck::{Pod, Zeroable};
use glam::{DMat3, DVec3};
use std::f64::consts::{PI, TAU};

/// Stars of magnitude 6 or brighter from the HYG database, made by tools/gen_stars.py.
/// Six bytes each: right ascension (u16), declination (i16), magnitude and colour index (u8).
const STAR_TABLE: &[u8] = include_bytes!("../data/stars.bin");
const STAR_BYTES: usize = 6;

/// One thing in the sky as the GPU draws it. A radius of zero makes a point; otherwise a disc.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct SkyPoint {
    pub dir: [f32; 3],
    pub mag: f32,
    pub colour: [f32; 3],
    /// Angular radius in radians.
    pub radius: f32,
}

/// Decodes the bundled star table.
pub fn star_points() -> Vec<SkyPoint> {
    decode_stars(STAR_TABLE)
}

pub fn decode_stars(bytes: &[u8]) -> Vec<SkyPoint> {
    bytes
        .chunks_exact(STAR_BYTES)
        .map(|c| {
            let ra = u16::from_le_bytes([c[0], c[1]]) as f64 / 65536.0 * TAU;
            let dec = i16::from_le_bytes([c[2], c[3]]) as f64 / 32767.0 * (PI / 2.0);
            let mag = c[4] as f32 / 20.0 - 1.5;
            let bv = c[5] as f32 / 50.0 - 0.5;
            let d = equatorial(ra, dec);
            SkyPoint {
                dir: [d.x as f32, d.y as f32, d.z as f32],
                mag,
                colour: bv_colour(bv),
                radius: 0.0,
            }
        })
        .collect()
}

/// The unit vector for a right ascension and declination in radians.
pub fn equatorial(ra: f64, dec: f64) -> DVec3 {
    DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin())
}

/// A star's apparent colour (linear RGB) from its B-V colour index, by interpolating between
/// the familiar blue-white, white, yellow, orange and red stars.
pub fn bv_colour(bv: f32) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 6] = [
        (-0.3, [0.55, 0.68, 1.0]),
        (0.0, [0.78, 0.86, 1.0]),
        (0.6, [1.0, 0.95, 0.85]),
        (1.0, [1.0, 0.80, 0.55]),
        (1.5, [1.0, 0.62, 0.38]),
        (2.0, [1.0, 0.48, 0.30]),
    ];
    let bv = bv.clamp(STOPS[0].0, STOPS[5].0);
    for w in STOPS.windows(2) {
        if bv <= w[1].0 {
            let f = (bv - w[0].0) / (w[1].0 - w[0].0);
            return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * f);
        }
    }
    STOPS[5].1
}

/// Julian centuries since J2000 for a Unix time (UT is taken as TT; the difference is about a minute).
pub fn julian_centuries(unix: f64) -> f64 {
    (unix / 86_400.0 + 2_440_587.5 - 2_451_545.0) / 36_525.0
}

/// Greenwich mean sidereal time in degrees, from the Astronomical Almanac's formula
/// GMST = 18.697374558 h + 24.06570982441908 h × (days since J2000), which is the same as
/// Meeus, "Astronomical Algorithms", eq. 12.4 without its negligible T² and T³ terms.
pub fn gmst_degrees(unix: f64) -> f64 {
    let d = unix / 86_400.0 + 2_440_587.5 - 2_451_545.0;
    (18.697_374_558 + 24.065_709_824_419_08 * d).rem_euclid(24.0) * 15.0
}

/// Rotates J2000 equatorial directions to the mean equator and equinox of date: the IAU 1976
/// precession angles of Meeus, "Astronomical Algorithms", ch. 21 (eqs. 21.2 and 21.4).
pub fn precession(t: f64) -> DMat3 {
    let arcsec = PI / 648_000.0;
    let zeta = (2306.2181 * t + 0.30188 * t * t + 0.017998 * t * t * t) * arcsec;
    let z = (2306.2181 * t + 1.09468 * t * t + 0.018203 * t * t * t) * arcsec;
    let theta = (2004.3109 * t - 0.42665 * t * t - 0.041833 * t * t * t) * arcsec;
    DMat3::from_rotation_z(z) * DMat3::from_rotation_y(-theta) * DMat3::from_rotation_z(zeta)
}

/// From J2000 equatorial directions to the Earth-fixed frame at a given time.
pub fn earth_fixed(unix: f64) -> DMat3 {
    DMat3::from_rotation_z(-gmst_degrees(unix).to_radians()) * precession(julian_centuries(unix))
}

/// The faintest magnitude that can be seen from a viewpoint: a dark sky shows the faintest
/// naked-eye stars, the day sky hides all but Venus and the Moon, and above the air, nothing
/// hides anything. `daylight` is the share of the Sun's disc that is uncovered, 1 outside an eclipse. `sun_sin_altitude` is the sine of the Sun's altitude there; `None` means
/// flat light, which is always day.
pub fn limiting_magnitude(altitude: f64, sun_sin_altitude: Option<f64>, daylight: f64) -> f64 {
    const DARK: f64 = 6.5;
    const DAY: f64 = -4.5;
    let s = sun_sin_altitude.unwrap_or(1.0);
    let t = ((s + 0.2) / 0.3).clamp(0.0, 1.0);
    let day = t * t * (3.0 - 2.0 * t);
    // A covered Sun dims the sky in proportion: the sky gets 2.5 log10(1 / light) magnitudes
    // fainter, which over the eleven magnitudes between day and night is a share of the day.
    let day =
        day * (1.0 - (2.5 * (1.0 / daylight.max(1e-6)).log10() / (DARK - DAY)).clamp(0.0, 1.0));
    let air = (-altitude.max(0.0) / AIR_SCALE).exp();
    DARK + (DAY - DARK) * day * air
}

/// Height over which the air thins out for sky purposes, in metres.
pub const AIR_SCALE: f64 = 15_000.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky_time::parse_iso8601;

    fn at(s: &str) -> f64 {
        parse_iso8601(s).unwrap()
    }

    #[test]
    fn sidereal_time_matches_published_values() {
        // Meeus example 12.a: 1987 April 10, 0h UT, GMST = 13h 10m 46.3668s.
        let g = gmst_degrees(at("1987-04-10T00:00:00Z"));
        assert!((g - 13.179_546_33 * 15.0).abs() < 0.001, "{g}");
        // J2000.0 (2000 January 1, 12h): GMST = 18h 41m 50.548s.
        let g = gmst_degrees(at("2000-01-01T12:00:00Z"));
        assert!((g - 18.697_374_558 * 15.0).abs() < 1e-6, "{g}");
        // A sidereal day is 3m 56.555s shorter than a solar day.
        let a = gmst_degrees(at("2026-03-01T00:00:00Z"));
        let b = gmst_degrees(at("2026-03-02T00:00:00Z"));
        assert!(((b - a).rem_euclid(360.0) - 0.985_647).abs() < 1e-4);
    }

    #[test]
    fn precession_moves_the_equinox_by_the_known_rates() {
        // At RA 0, Dec 0: about 3.075 s (0.0128°) of right ascension and 20.04" of declination a year.
        let p = precession(0.25) * equatorial(0.0, 0.0);
        let (ra, dec) = (p.y.atan2(p.x).to_degrees(), p.z.asin().to_degrees());
        assert!((ra - 25.0 * 0.012_813).abs() < 0.002, "{ra}");
        assert!((dec - 25.0 * 20.04 / 3600.0).abs() < 0.002, "{dec}");
        assert!((precession(0.0) * DVec3::X - DVec3::X).length() < 1e-12);
    }

    #[test]
    fn the_star_table_holds_the_naked_eye_stars() {
        let stars = star_points();
        assert!((4500..6500).contains(&stars.len()), "{}", stars.len());
        assert!(
            stars.windows(2).all(|w| w[0].mag <= w[1].mag),
            "brightest first"
        );
        // Sirius: magnitude -1.44, RA 6h 45m, Dec -16.72°.
        let s = &stars[0];
        assert!((s.mag + 1.44).abs() < 0.06, "{}", s.mag);
        let d = DVec3::new(s.dir[0] as f64, s.dir[1] as f64, s.dir[2] as f64);
        assert!((d.y.atan2(d.x).to_degrees() - 101.287).abs() < 0.02);
        assert!((d.z.asin().to_degrees() + 16.716).abs() < 0.02);
        assert!(stars.iter().all(|s| s.mag <= 6.06));
        // Betelgeuse (about mag 0.4–0.5) is orange, Rigel (0.13) blue-white.
        let near = |ra: f64, dec: f64| {
            let t = equatorial(ra.to_radians(), dec.to_radians());
            stars
                .iter()
                .find(|s| {
                    (DVec3::new(s.dir[0] as f64, s.dir[1] as f64, s.dir[2] as f64) - t).length()
                        < 0.002
                })
                .unwrap()
        };
        let betelgeuse = near(88.793, 7.407);
        let rigel = near(78.634, -8.202);
        assert!(betelgeuse.colour[2] < 0.6 && rigel.colour[2] > 0.95);
    }

    #[test]
    fn stars_turn_with_sidereal_time() {
        // At J2000 noon, a star on the equator at RA 101.287° stands over longitude 101.287° - GMST.
        let t = at("2000-01-01T12:00:00Z");
        let v = earth_fixed(t) * equatorial(101.287f64.to_radians(), 0.0);
        let lon = v.y.atan2(v.x).to_degrees();
        assert!((lon - (101.287 - 280.4606)).abs() < 0.01, "{lon}");
        // Half a sidereal day later it is on the other side.
        let v2 = earth_fixed(t + 43_082.05) * equatorial(101.287f64.to_radians(), 0.0);
        assert!(v.dot(v2) < -0.999);
        // The pole star barely moves.
        let p = equatorial(0.0, 89.9f64.to_radians());
        assert!((earth_fixed(t) * p).z > 0.99);
    }

    #[test]
    fn colours_run_from_blue_to_red() {
        assert!(bv_colour(-0.3)[2] > bv_colour(-0.3)[0]);
        assert!(bv_colour(2.0)[0] > bv_colour(2.0)[2]);
        assert_eq!(bv_colour(9.0), bv_colour(2.0));
    }

    #[test]
    fn the_limit_follows_day_night_and_height() {
        let night = limiting_magnitude(0.0, Some(-0.5), 1.0);
        let noon = limiting_magnitude(0.0, Some(0.9), 1.0);
        let space = limiting_magnitude(400_000.0, Some(0.9), 1.0);
        assert!(night > 6.0 && noon < -4.0 && space > 6.4);
        let dusk = limiting_magnitude(0.0, Some(-0.1), 1.0);
        assert!(dusk < night && dusk > noon);
        assert!(
            limiting_magnitude(0.0, None, 1.0) < -4.0,
            "flat light is day"
        );
        // Totality darkens the day sky to near night; a partial eclipse hardly changes it.
        assert!(limiting_magnitude(0.0, Some(0.9), 1e-5) > 5.0);
        assert!(limiting_magnitude(0.0, Some(0.9), 0.5) < -3.0);
    }
}
