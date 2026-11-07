//! Positions of the planets and the Moon as seen from the Earth. No window or GPU is involved.
//!
//! Directions are unit vectors in the J2000 equatorial frame (see `sky`).

use crate::sky::{equatorial, julian_centuries, precession, SkyPoint};
use glam::DVec3;
use std::f64::consts::{PI, TAU};

pub struct Body {
    pub name: &'static str,
    pub dir: DVec3,
    /// Apparent visual magnitude.
    pub mag: f64,
    /// Angular radius in radians for the Moon; zero for the planets, which are drawn as points.
    pub radius: f64,
    pub colour: [f32; 3],
}

impl Body {
    pub fn point(&self) -> SkyPoint {
        SkyPoint {
            dir: [self.dir.x as f32, self.dir.y as f32, self.dir.z as f32],
            mag: self.mag as f32,
            colour: self.colour,
            radius: self.radius as f32,
        }
    }
}

/// Mean obliquity of the ecliptic at J2000.
const OBLIQUITY_J2000: f64 = 23.439_28 * PI / 180.0;

/// Orbital elements at J2000 and their rates per Julian century, from JPL's "Keplerian
/// elements for approximate positions of the major planets", Table 1 (valid 1800-2050):
/// semi-major axis (au), eccentricity, inclination, mean longitude, longitude of the
/// perihelion and of the ascending node (degrees).
struct Elements {
    name: &'static str,
    at_epoch: [f64; 6],
    rate: [f64; 6],
    /// Magnitude at unit distances and full phase, then the phase coefficients per degree,
    /// degree squared and degree cubed (Astronomical Almanac, as tabulated in Meeus ch. 41).
    magnitude: [f64; 4],
    colour: [f32; 3],
}

const EARTH_MOON: Elements = Elements {
    name: "Earth",
    at_epoch: [1.00000261, 0.01671123, -0.00001531, 100.46457166, 102.93768193, 0.0],
    rate: [0.00000562, -0.00004392, -0.01294668, 35999.37244981, 0.32327364, 0.0],
    magnitude: [0.0; 4],
    colour: [1.0; 3],
};

const PLANETS: [Elements; 5] = [
    Elements {
        name: "Mercury",
        at_epoch: [0.38709927, 0.20563593, 7.00497902, 252.25032350, 77.45779628, 48.33076593],
        rate: [0.00000037, 0.00001906, -0.00594749, 149472.67411175, 0.16047689, -0.12534081],
        magnitude: [-0.42, 0.0380, -0.000273, 0.000002],
        colour: [0.85, 0.80, 0.75],
    },
    Elements {
        name: "Venus",
        at_epoch: [0.72333566, 0.00677672, 3.39467605, 181.97909950, 131.60246718, 76.67984255],
        rate: [0.00000390, -0.00004107, -0.00078890, 58517.81538729, 0.00268329, -0.27769418],
        magnitude: [-4.40, 0.0009, 0.000239, -0.00000065],
        colour: [1.0, 0.97, 0.88],
    },
    Elements {
        name: "Mars",
        at_epoch: [1.52371034, 0.09339410, 1.84969142, -4.55343205, -23.94362959, 49.55953891],
        rate: [0.00001847, 0.00007882, -0.00813131, 19140.30268499, 0.44441088, -0.29257343],
        magnitude: [-1.52, 0.016, 0.0, 0.0],
        colour: [1.0, 0.55, 0.40],
    },
    Elements {
        name: "Jupiter",
        at_epoch: [5.20288700, 0.04838624, 1.30439695, 34.39644051, 14.72847983, 100.47390909],
        rate: [-0.00011607, -0.00013253, -0.00183714, 3034.74612775, 0.21252668, 0.20469106],
        magnitude: [-9.40, 0.005, 0.0, 0.0],
        colour: [1.0, 0.93, 0.80],
    },
    Elements {
        name: "Saturn",
        at_epoch: [9.53667594, 0.05386179, 2.48599187, 49.95424423, 92.59887831, 113.66242448],
        rate: [-0.00125060, -0.00050991, 0.00193609, 1222.49362201, -0.41897216, -0.28867794],
        // The rings, which brighten it, are left out.
        magnitude: [-8.88, 0.0, 0.0, 0.0],
        colour: [1.0, 0.90, 0.65],
    },
];

/// Heliocentric position in au on the J2000 ecliptic (JPL's procedure for Table 1).
fn heliocentric(el: &Elements, t: f64) -> DVec3 {
    let v: [f64; 6] = std::array::from_fn(|i| el.at_epoch[i] + el.rate[i] * t);
    let (a, e) = (v[0], v[1]);
    let (inc, l, peri, node) = (v[2].to_radians(), v[3], v[4], v[5].to_radians());
    let omega = peri.to_radians() - node;
    let m = (l - peri).to_radians().rem_euclid(TAU);
    let mut ecc = m + e * m.sin();
    for _ in 0..8 {
        ecc -= (ecc - e * ecc.sin() - m) / (1.0 - e * ecc.cos());
    }
    let xp = a * (ecc.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * ecc.sin();
    let (so, co) = omega.sin_cos();
    let (sn, cn) = node.sin_cos();
    let (si, ci) = inc.sin_cos();
    DVec3::new(
        (co * cn - so * sn * ci) * xp + (-so * cn - co * sn * ci) * yp,
        (co * sn + so * cn * ci) * xp + (-so * sn + co * cn * ci) * yp,
        so * si * xp + co * si * yp,
    )
}

fn ecliptic_to_equatorial(v: DVec3) -> DVec3 {
    let (s, c) = OBLIQUITY_J2000.sin_cos();
    DVec3::new(v.x, c * v.y - s * v.z, s * v.y + c * v.z)
}

/// The planets as seen from the Earth (light travel time is ignored, which is under 20 arcseconds
/// for the inner planets and under half an arcminute for the others).
fn planets(t: f64) -> Vec<Body> {
    let earth = heliocentric(&EARTH_MOON, t);
    let sun_distance = earth.length();
    PLANETS
        .iter()
        .map(|el| {
            let p = heliocentric(el, t);
            let geo = p - earth;
            let (r, delta) = (p.length(), geo.length());
            // The phase angle at the planet, between the Sun and the Earth.
            let cos_i = ((r * r + delta * delta - sun_distance * sun_distance) / (2.0 * r * delta)).clamp(-1.0, 1.0);
            let i = cos_i.acos().to_degrees();
            let [m0, k1, k2, k3] = el.magnitude;
            let mag = m0 + 5.0 * (r * delta).log10() + k1 * i + k2 * i * i + k3 * i * i * i;
            Body {
                name: el.name,
                dir: ecliptic_to_equatorial(geo).normalize(),
                mag,
                radius: 0.0,
                colour: el.colour,
            }
        })
        .collect()
}

/// The Moon, from the Astronomical Almanac's low-precision formulae (about 0.3° in longitude and
/// latitude, and 0.003° in parallax). The result is geocentric: up to 1° off for an observer
/// who sees the Moon near the horizon.
fn moon(t: f64) -> Body {
    let d = |deg: f64, rate: f64| (deg + rate * t).to_radians();
    let lon = 218.32 + 481_267.881 * t
        + 6.29 * d(135.0, 477_198.87).sin()
        - 1.27 * d(259.3, -413_335.36).sin()
        + 0.66 * d(235.7, 890_534.22).sin()
        + 0.21 * d(269.9, 954_397.74).sin()
        - 0.19 * d(357.5, 35_999.05).sin()
        - 0.11 * d(186.5, 966_404.03).sin();
    let lat = 5.13 * d(93.3, 483_202.02).sin() + 0.28 * d(228.2, 960_400.89).sin()
        - 0.28 * d(318.3, 6_003.15).sin()
        - 0.17 * d(217.6, -407_332.21).sin();
    let parallax = 0.9508
        + 0.0518 * d(135.0, 477_198.87).cos()
        + 0.0095 * d(259.3, -413_335.36).cos()
        + 0.0078 * d(235.7, 890_534.22).cos()
        + 0.0028 * d(269.9, 954_397.74).cos();
    let (lon, lat) = (lon.to_radians(), lat.to_radians());
    // Direction cosines on the equator of date; 0.9175 and 0.3978 are cos and sin of the obliquity.
    let l = lat.cos() * lon.cos();
    let m = 0.9175 * lat.cos() * lon.sin() - 0.3978 * lat.sin();
    let n = 0.3978 * lat.cos() * lon.sin() + 0.9175 * lat.sin();
    let of_date = equatorial(m.atan2(l), n.asin());
    Body {
        name: "Moon",
        dir: (precession(t).transpose() * of_date).normalize(),
        mag: -12.7,
        radius: (0.272_508 * parallax.to_radians().sin()).asin(),
        colour: [1.0, 0.98, 0.93],
    }
}

/// The five bright planets and the Moon at a Unix time.
pub fn bodies(unix: f64) -> Vec<Body> {
    let t = julian_centuries(unix);
    let mut all = planets(t);
    all.push(moon(t));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::earth_fixed;
    use crate::sky_time::{parse_iso8601, Sun};

    fn at(s: &str) -> f64 {
        parse_iso8601(s).unwrap()
    }

    fn find(list: &[Body], name: &str) -> DVec3 {
        list.iter().find(|b| b.name == name).unwrap().dir
    }

    fn separation(a: DVec3, b: DVec3) -> f64 {
        a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// Right ascension and declination of date, in degrees.
    fn of_date(dir: DVec3, unix: f64) -> (f64, f64) {
        let v = precession(julian_centuries(unix)) * dir;
        (v.y.atan2(v.x).to_degrees().rem_euclid(360.0), v.z.asin().to_degrees())
    }

    #[test]
    fn venus_matches_meeus_example_33a() {
        // 1992 December 20, 0h: apparent RA 21h 04m 41.5s, Dec -18° 53' 17"; we expect arcminutes.
        let unix = at("1992-12-20T00:00:00Z");
        let (ra, dec) = of_date(find(&bodies(unix), "Venus"), unix);
        assert!((ra - 316.1727).abs() < 0.1, "{ra}");
        assert!((dec + 18.888).abs() < 0.1, "{dec}");
    }

    #[test]
    fn jupiter_and_saturn_meet_at_the_great_conjunction() {
        // 2020 December 21, about 18h UT: 0.1° apart in Capricornus near RA 20h 12m, Dec -20.5° (the sky position is checked loosely).
        let unix = at("2020-12-21T18:00:00Z");
        let all = bodies(unix);
        let (j, s) = (find(&all, "Jupiter"), find(&all, "Saturn"));
        assert!(separation(j, s) < 0.3, "{}", separation(j, s));
        let (ra, dec) = of_date(j, unix);
        assert!((ra - 302.8).abs() < 0.5 && (dec + 20.5).abs() < 0.5, "{ra} {dec}");
    }

    #[test]
    fn inner_planets_stay_near_the_sun() {
        let mut venus_max: f64 = 0.0;
        let mut mercury_max: f64 = 0.0;
        for day in 0..730 {
            let unix = at("2025-01-01T00:00:00Z") + day as f64 * 86_400.0;
            let sun = earth_fixed(unix).transpose() * Sun::at(unix).direction();
            let all = bodies(unix);
            venus_max = venus_max.max(separation(find(&all, "Venus"), sun));
            mercury_max = mercury_max.max(separation(find(&all, "Mercury"), sun));
        }
        // Greatest elongations are 47.8° and 28.3°.
        assert!((45.0..48.5).contains(&venus_max), "{venus_max}");
        assert!((24.0..28.8).contains(&mercury_max), "{mercury_max}");
    }

    #[test]
    fn mars_brightens_at_opposition() {
        // Mars was at opposition on 2025 January 16 at magnitude -1.4.
        let unix = at("2025-01-16T00:00:00Z");
        let all = bodies(unix);
        let mars = all.iter().find(|b| b.name == "Mars").unwrap();
        assert!((mars.mag + 1.4).abs() < 0.3, "{}", mars.mag);
        let sun = earth_fixed(unix).transpose() * Sun::at(unix).direction();
        assert!(separation(mars.dir, sun) > 170.0);
        let jupiter = all.iter().find(|b| b.name == "Jupiter").unwrap();
        assert!((jupiter.mag + 2.5).abs() < 0.4, "{}", jupiter.mag);
        let venus = bodies(at("2025-02-10T00:00:00Z")).into_iter().find(|b| b.name == "Venus").unwrap();
        assert!(venus.mag < -4.0, "Venus is brilliant in February 2025: {}", venus.mag);
    }

    #[test]
    fn moon_matches_meeus_example_47a() {
        // 1992 April 12, 0h TD: apparent RA 134.688°, Dec +13.768°, distance 368 410 km (parallax 0.9928°).
        let unix = at("1992-04-12T00:00:00Z");
        let m = moon(julian_centuries(unix));
        let (ra, dec) = of_date(m.dir, unix);
        assert!((ra - 134.688).abs() < 0.4, "{ra}");
        assert!((dec - 13.768).abs() < 0.4, "{dec}");
        let radius = m.radius.to_degrees();
        assert!((radius - 0.2725 * 0.9928).abs() < 0.01, "{radius}");
    }

    #[test]
    fn moon_stands_with_the_sun_at_eclipses() {
        let sun = |unix: f64| earth_fixed(unix).transpose() * Sun::at(unix).direction();
        // Total solar eclipse of 2024 April 8, 18:17 UT: Moon and Sun together.
        let t = at("2024-04-08T18:17:00Z");
        assert!(separation(find(&bodies(t), "Moon"), sun(t)) < 0.8);
        // Total lunar eclipse of 2019 January 21, 05:12 UT: Moon opposite the Sun.
        let t = at("2019-01-21T05:12:00Z");
        assert!(separation(find(&bodies(t), "Moon"), sun(t)) > 179.0 - 0.8);
        // And first quarter is about 90° away.
        let t = at("2026-01-26T04:47:00Z");
        let quarter = separation(find(&bodies(t), "Moon"), sun(t));
        assert!((quarter - 90.0).abs() < 8.0, "{quarter}");
    }
}
