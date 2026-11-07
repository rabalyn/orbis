//! Positions of the Sun and Moon good to a few arcseconds, and ΔT, for eclipse work.
//!
//! The Sun (with the aberration that eclipse catalogs include) comes from the truncated VSOP87 series for the Earth, the Moon from the truncated ELP-2000/82
//! series; both as printed in Meeus, "Astronomical Algorithms" (2nd ed.): chapter 25 and appendix III
//! for the Sun, chapter 47 for the Moon, chapter 22 for nutation. Positions are geometric (no
//! aberration), referred to the true equator and equinox of date, which is the frame the Earth's
//! rotation angle belongs to. Times are Unix seconds in UT; ΔT turns them into dynamical time.

#![allow(clippy::approx_constant)]

use glam::{DMat3, DVec3};
use std::f64::consts::PI;

pub const AU_KM: f64 = 149_597_870.7;

/// TT − UT in seconds for a decimal year: the polynomial expressions of Espenak and Meeus,
/// "Five Millennium Canon of Solar Eclipses" (NASA/TP-2006-214141), valid from -1999 to +3000.
pub fn delta_t(y: f64) -> f64 {
    let poly = |c: &[f64], t: f64| c.iter().rev().fold(0.0, |acc, k| acc * t + k);
    match y {
        y if y < -500.0 => -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2),
        y if y < 500.0 => poly(&[10583.6, -1014.41, 33.78311, -5.952053, -0.1798452, 0.022174192, 0.0090316521], y / 100.0),
        y if y < 1600.0 => poly(&[1574.2, -556.01, 71.23472, 0.319781, -0.8503463, -0.005050998, 0.0083572073], (y - 1000.0) / 100.0),
        y if y < 1700.0 => poly(&[120.0, -0.9808, -0.01532, 1.0 / 7129.0], y - 1600.0),
        y if y < 1800.0 => poly(&[8.83, 0.1603, -0.0059285, 0.00013336, -1.0 / 1_174_000.0], y - 1700.0),
        y if y < 1860.0 => poly(
            &[13.72, -0.332447, 0.0068612, 0.0041116, -0.00037436, 0.0000121272, -0.0000001699, 0.000000000875],
            y - 1800.0,
        ),
        y if y < 1900.0 => poly(&[7.62, 0.5737, -0.251754, 0.01680668, -0.0004473624, 1.0 / 233_174.0], y - 1860.0),
        y if y < 1920.0 => poly(&[-2.79, 1.494119, -0.0598939, 0.0061966, -0.000197], y - 1900.0),
        y if y < 1941.0 => poly(&[21.20, 0.84493, -0.076100, 0.0020936], y - 1920.0),
        y if y < 1961.0 => poly(&[29.07, 0.407, -1.0 / 233.0, 1.0 / 2547.0], y - 1950.0),
        y if y < 1986.0 => poly(&[45.45, 1.067, -1.0 / 260.0, -1.0 / 718.0], y - 1975.0),
        y if y < 2005.0 => poly(&[63.86, 0.3345, -0.060374, 0.0017275, 0.000651814, 0.00002373599], y - 2000.0),
        y if y < 2050.0 => poly(&[62.92, 0.32217, 0.005589], y - 2000.0),
        y if y < 2150.0 => -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2) - 0.5628 * (2150.0 - y),
        y => -20.0 + 32.0 * ((y - 1820.0) / 100.0).powi(2),
    }
}

/// ΔT in seconds at a Unix time.
pub fn delta_t_at(unix: f64) -> f64 {
    delta_t(1970.0 + unix / (365.2425 * 86_400.0))
}

/// Julian centuries of dynamical time since J2000 for a Unix time in UT.
pub fn centuries_tt(unix: f64) -> f64 {
    ((unix + delta_t_at(unix)) / 86_400.0 + 2_440_587.5 - 2_451_545.0) / 36_525.0
}

/// The Earth's heliocentric L, B (radians) and R (au) from VSOP87D, truncated as in Meeus,
/// appendix III. Each term is amplitude (1e-8), phase, frequency; `tau` is in millennia.
fn earth_lbr(tau: f64) -> (f64, f64, f64) {
    let sum = |series: &[&[(f64, f64, f64)]]| -> f64 {
        let mut total = 0.0;
        let mut power = 1.0;
        for terms in series {
            let s: f64 = terms.iter().map(|&(a, b, c)| a * (b + c * tau).cos()).sum();
            total += s * power;
            power *= tau;
        }
        total * 1e-8
    };
    let l = sum(&[L0, L1, L2, L3, L4, L5]);
    let b = sum(&[B0, B1]);
    let r = sum(&[R0, R1, R2, R3, R4]);
    (l.rem_euclid(2.0 * PI), b, r)
}

/// Nutation in longitude and obliquity (radians), and the mean obliquity: Meeus, ch. 22 (the short
/// series, good to 0.5 arcsecond) and eq. 22.2.
pub fn nutation(t: f64) -> (f64, f64, f64) {
    let arcsec = PI / 648_000.0;
    let omega = (125.04452 - 1934.136261 * t).to_radians();
    let ls = (280.4665 + 36000.7698 * t).to_radians();
    let lm = (218.3165 + 481267.8813 * t).to_radians();
    let dpsi = -17.20 * omega.sin() - 1.32 * (2.0 * ls).sin() - 0.23 * (2.0 * lm).sin() + 0.21 * (2.0 * omega).sin();
    let deps = 9.20 * omega.cos() + 0.57 * (2.0 * ls).cos() + 0.10 * (2.0 * lm).cos() - 0.09 * (2.0 * omega).cos();
    let eps0 = 23.0 * 3600.0 + 26.0 * 60.0 + 21.448 - 46.8150 * t - 0.00059 * t * t + 0.001813 * t * t * t;
    (dpsi * arcsec, deps * arcsec, eps0 * arcsec)
}

/// A geocentric ecliptic position: longitude and latitude (radians) and distance (km).
#[derive(Clone, Copy, Debug)]
pub struct Ecliptic {
    pub lon: f64,
    pub lat: f64,
    pub dist: f64,
}

/// The Sun, geometric, on the mean equinox of date (Meeus ch. 25, with the FK5 correction).
pub fn sun_mean_of_date(t: f64) -> Ecliptic {
    let arcsec = PI / 648_000.0;
    let (l, b, r) = earth_lbr(t / 10.0);
    let lprime = l - (1.397 * t + 0.00031 * t * t).to_radians();
    let lon = (l + PI - 0.09033 * arcsec).rem_euclid(2.0 * PI);
    let lat = -b + 0.03916 * arcsec * (lprime.cos() - lprime.sin());
    Ecliptic { lon, lat, dist: r * AU_KM }
}

/// The Moon, geometric, on the mean equinox of date: Meeus ch. 47 (about 10 arcseconds in longitude,
/// 4 in latitude, and 10 km in distance).
pub fn moon_mean_of_date(t: f64) -> Ecliptic {
    let (t2, t3, t4) = (t * t, t * t * t, t * t * t * t);
    let lp = 218.3164477 + 481267.88123421 * t - 0.0015786 * t2 + t3 / 538_841.0 - t4 / 65_194_000.0;
    let d = 297.8501921 + 445267.1114034 * t - 0.0018819 * t2 + t3 / 545_868.0 - t4 / 113_065_000.0;
    let m = 357.5291092 + 35999.0502909 * t - 0.0001536 * t2 + t3 / 24_490_000.0;
    let mp = 134.9633964 + 477198.8675055 * t + 0.0087414 * t2 + t3 / 69_699.0 - t4 / 14_712_000.0;
    let f = 93.2720950 + 483202.0175233 * t - 0.0036539 * t2 - t3 / 3_526_000.0 + t4 / 863_310_000.0;
    let a1 = (119.75 + 131.849 * t).to_radians();
    let a2 = (53.09 + 479264.290 * t).to_radians();
    let a3 = (313.45 + 481266.484 * t).to_radians();
    let e = 1.0 - 0.002516 * t - 0.0000074 * t2;
    let (dr, mr, mpr, fr) = (d.to_radians(), m.to_radians(), mp.to_radians(), f.to_radians());
    let lpr = lp.to_radians();

    let (mut sl, mut sr, mut sb) = (0.0, 0.0, 0.0);
    for &(cd, cm, cmp, cf, l, r) in LONGITUDE_DISTANCE {
        let arg = cd as f64 * dr + cm as f64 * mr + cmp as f64 * mpr + cf as f64 * fr;
        let w = e.powi((cm as i32).abs());
        sl += w * l * arg.sin();
        sr += w * r * arg.cos();
    }
    for &(cd, cm, cmp, cf, b) in LATITUDE {
        let arg = cd as f64 * dr + cm as f64 * mr + cmp as f64 * mpr + cf as f64 * fr;
        sb += e.powi((cm as i32).abs()) * b * arg.sin();
    }
    sl += 3958.0 * a1.sin() + 1962.0 * (lpr - fr).sin() + 318.0 * a2.sin();
    sb += -2235.0 * lpr.sin() + 382.0 * a3.sin() + 175.0 * (a1 - fr).sin() + 175.0 * (a1 + fr).sin()
        + 127.0 * (lpr - mpr).sin()
        - 115.0 * (lpr + mpr).sin();
    Ecliptic {
        lon: (lp + sl / 1e6).to_radians().rem_euclid(2.0 * PI),
        lat: (sb / 1e6).to_radians(),
        dist: 385_000.56 + sr / 1000.0,
    }
}

/// A geocentric vector in km on the true equator of date, from an ecliptic position on the mean
/// equinox of date.
fn equatorial(p: Ecliptic, dpsi: f64, eps: f64) -> DVec3 {
    let lon = p.lon + dpsi;
    let (sl, cl) = lon.sin_cos();
    let (sb, cb) = p.lat.sin_cos();
    let (se, ce) = eps.sin_cos();
    DVec3::new(cb * cl, cb * sl * ce - sb * se, cb * sl * se + sb * ce) * p.dist
}

/// The Sun and Moon at a Unix time (UT), geocentric in km on the true equator of date, and the
/// apparent sidereal time at Greenwich (radians) for turning them Earth-fixed.
#[derive(Clone, Copy, Debug)]
pub struct SunMoon {
    pub sun: DVec3,
    pub moon: DVec3,
    pub sidereal: f64,
}

impl SunMoon {
    pub fn at(unix: f64) -> Self {
        let t = centuries_tt(unix);
        let (dpsi, deps, eps0) = nutation(t);
        let eps = eps0 + deps;
        let gmst = crate::sky::gmst_degrees(unix).to_radians();
        // The catalogs of eclipses are computed from the Sun's apparent place, which is displaced by
        // the aberration of 20.4898" / R (Meeus, eq. 25.10); that moves the greatest eclipse by 37 s.
        let mut sun = sun_mean_of_date(t);
        sun.lon -= 20.4898 * PI / 648_000.0 / (sun.dist / AU_KM);
        Self {
            sun: equatorial(sun, dpsi, eps),
            moon: equatorial(moon_mean_of_date(t), dpsi, eps),
            sidereal: (gmst + dpsi * eps.cos()).rem_euclid(2.0 * PI),
        }
    }

    /// From the true equator of date to the Earth-fixed frame.
    pub fn to_earth_fixed(self) -> DMat3 {
        DMat3::from_rotation_z(-self.sidereal)
    }
}

/// The Moon's mean radius in km (IAU).
const MOON_RADIUS_KM: f64 = 1737.4;
const SUN_RADIUS_KM: f64 = 696_000.0;

/// The Sun and Moon as seen from a point near the Earth: unit vectors in the Earth-fixed frame
/// and angular radii. The Moon's is topocentric, so it carries the parallax of up to a degree
/// that makes it appear over or beside the Sun depending on where the viewer stands.
#[derive(Clone, Copy, Debug)]
pub struct Topocentric {
    pub sun: DVec3,
    pub sun_radius: f64,
    pub moon: DVec3,
    pub moon_radius: f64,
}

impl Topocentric {
    /// Angle between the centres of the Sun and the Moon.
    pub fn separation(&self) -> f64 {
        self.sun.dot(self.moon).clamp(-1.0, 1.0).acos()
    }

    /// The fraction of the Sun's disc that the Moon covers.
    pub fn obscuration(&self) -> f64 {
        disc_overlap(self.sun_radius, self.moon_radius, self.separation())
    }
}

/// The fraction of a disc of radius `s` covered by a disc of radius `r` whose centre is `d` away
/// (the area of the lens between two circles).
pub fn disc_overlap(s: f64, r: f64, d: f64) -> f64 {
    if s <= 0.0 || d >= s + r {
        return 0.0;
    }
    if d <= (r - s).abs() {
        return if r >= s { 1.0 } else { r * r / (s * s) };
    }
    let ca = ((d * d + s * s - r * r) / (2.0 * d * s)).clamp(-1.0, 1.0);
    let cb = ((d * d + r * r - s * s) / (2.0 * d * r)).clamp(-1.0, 1.0);
    let k = (s + r - d) * (d + s - r) * (d - s + r) * (d + s + r);
    let area = s * s * ca.acos() + r * r * cb.acos() - 0.5 * k.max(0.0).sqrt();
    (area / (PI * s * s)).clamp(0.0, 1.0)
}

impl SunMoon {
    /// What an observer at `ecef` (Earth-fixed, metres) sees: the offset from the Earth's centre
    /// to the viewer is taken off both bodies' vectors.
    pub fn topocentric(self, ecef: DVec3) -> Topocentric {
        let to_fixed = self.to_earth_fixed();
        let me = ecef / 1000.0;
        let sun = to_fixed * self.sun - me;
        let moon = to_fixed * self.moon - me;
        Topocentric {
            sun: sun.normalize(),
            sun_radius: (SUN_RADIUS_KM / sun.length()).asin(),
            moon: moon.normalize(),
            moon_radius: (MOON_RADIUS_KM / moon.length()).asin(),
        }
    }
}

// Earth, VSOP87D, Meeus appendix III: amplitude (1e-8 rad or au), phase, frequency.
#[rustfmt::skip]
const L0: &[(f64, f64, f64)] = &[
    (175347046.0, 0.0, 0.0), (3341656.0, 4.6692568, 6283.0758500), (34894.0, 4.62610, 12566.15170),
    (3497.0, 2.7441, 5753.3849), (3418.0, 2.8289, 3.5231), (3136.0, 3.6277, 77713.7715),
    (2676.0, 4.4181, 7860.4194), (2343.0, 6.1352, 3930.2097), (1324.0, 0.7425, 11506.7698),
    (1273.0, 2.0371, 529.6910), (1199.0, 1.1096, 1577.3435), (990.0, 5.233, 5884.927),
    (902.0, 2.045, 26.298), (857.0, 3.508, 398.149), (780.0, 1.179, 5223.694),
    (753.0, 2.533, 5507.553), (505.0, 4.583, 18849.228), (492.0, 4.205, 775.523),
    (357.0, 2.920, 0.067), (317.0, 5.849, 11790.629), (284.0, 1.899, 796.298),
    (271.0, 0.315, 10977.079), (243.0, 0.345, 5486.778), (206.0, 4.806, 2544.314),
    (205.0, 1.869, 5573.143), (202.0, 2.458, 6069.777), (156.0, 0.833, 213.299),
    (132.0, 3.411, 2942.463), (126.0, 1.083, 20.775), (115.0, 0.645, 0.980),
    (103.0, 0.636, 4694.003), (102.0, 0.976, 15720.839), (102.0, 4.267, 7.114),
    (99.0, 6.21, 2146.17), (98.0, 0.68, 155.42), (86.0, 5.98, 161000.69),
    (85.0, 1.30, 6275.96), (85.0, 3.67, 71430.70), (80.0, 1.81, 17260.15),
    (79.0, 3.04, 12036.46), (75.0, 1.76, 5088.63), (74.0, 3.50, 3154.69),
    (74.0, 4.68, 801.82), (70.0, 0.83, 9437.76), (62.0, 3.98, 8827.39),
    (61.0, 1.82, 7084.90), (57.0, 2.78, 6286.60), (56.0, 4.39, 14143.50),
    (56.0, 3.47, 6279.55), (52.0, 0.19, 12139.55), (52.0, 1.33, 1748.02),
    (51.0, 0.28, 5856.48), (49.0, 0.49, 1194.45), (41.0, 5.37, 8429.24),
    (41.0, 2.40, 19651.05), (39.0, 6.17, 10447.39),
];
#[rustfmt::skip]
const L1: &[(f64, f64, f64)] = &[
    (628331966747.0, 0.0, 0.0), (206059.0, 2.678235, 6283.075850), (4303.0, 2.6351, 12566.1517),
    (425.0, 1.590, 3.523), (119.0, 5.796, 26.298), (109.0, 2.966, 1577.344),
    (93.0, 2.59, 18849.23), (72.0, 1.14, 529.69), (68.0, 1.87, 398.15),
    (67.0, 4.41, 5507.55), (59.0, 2.89, 5223.69), (56.0, 2.17, 155.42),
    (45.0, 0.40, 796.30), (36.0, 0.47, 775.52), (29.0, 2.65, 7.11),
    (21.0, 5.34, 0.98), (19.0, 1.85, 5486.78), (19.0, 4.97, 213.30),
    (17.0, 2.99, 6275.96), (16.0, 0.03, 2544.31), (16.0, 1.43, 2146.17),
    (15.0, 1.21, 10977.08), (12.0, 2.83, 1748.02), (12.0, 3.26, 5088.63),
    (12.0, 5.27, 1194.45), (12.0, 2.08, 4694.00), (11.0, 0.77, 553.57),
    (10.0, 1.30, 6286.60), (10.0, 4.24, 1349.87), (9.0, 2.70, 242.73),
    (9.0, 5.64, 951.72), (8.0, 5.30, 2352.87), (6.0, 2.65, 9437.76),
    (6.0, 4.67, 4690.48),
];
#[rustfmt::skip]
const L2: &[(f64, f64, f64)] = &[
    (52919.0, 0.0, 0.0), (8720.0, 1.0721, 6283.0758), (309.0, 0.867, 12566.152),
    (27.0, 0.05, 3.52), (16.0, 5.19, 26.30), (16.0, 3.68, 155.42),
    (10.0, 0.76, 18849.23), (9.0, 2.06, 77713.77), (7.0, 0.83, 775.52),
    (5.0, 4.66, 1577.34), (4.0, 1.03, 7.11), (4.0, 3.44, 5573.14),
    (3.0, 5.14, 796.30), (3.0, 6.05, 5507.55), (3.0, 1.19, 242.73),
    (3.0, 6.12, 529.69), (3.0, 0.31, 398.15), (3.0, 2.28, 553.57),
    (2.0, 4.38, 5223.69), (2.0, 3.75, 0.98),
];
const L3: &[(f64, f64, f64)] = &[
    (289.0, 5.844, 6283.076),
    (35.0, 0.0, 0.0),
    (17.0, 5.49, 12566.15),
    (3.0, 5.20, 155.42),
    (1.0, 4.72, 3.52),
    (1.0, 5.30, 18849.23),
    (1.0, 5.97, 242.73),
];
const L4: &[(f64, f64, f64)] = &[(114.0, 3.142, 0.0), (8.0, 4.13, 6283.08), (1.0, 3.84, 12566.15)];
const L5: &[(f64, f64, f64)] = &[(1.0, 3.14, 0.0)];
const B0: &[(f64, f64, f64)] =
    &[(280.0, 3.199, 84334.662), (102.0, 5.422, 5507.553), (80.0, 3.88, 5223.69), (44.0, 3.70, 2352.87), (32.0, 4.00, 1577.34)];
const B1: &[(f64, f64, f64)] = &[(9.0, 3.90, 5507.55), (6.0, 1.73, 5223.69)];
#[rustfmt::skip]
const R0: &[(f64, f64, f64)] = &[
    (100013989.0, 0.0, 0.0), (1670700.0, 3.0984635, 6283.0758500), (13956.0, 3.05525, 12566.15170),
    (3084.0, 5.1985, 77713.7715), (1628.0, 1.1739, 5753.3849), (1576.0, 2.8469, 7860.4194),
    (925.0, 5.453, 11506.770), (542.0, 4.564, 3930.210), (472.0, 3.661, 5884.927),
    (346.0, 0.964, 5507.553), (329.0, 5.900, 5223.694), (307.0, 0.299, 5573.143),
    (243.0, 4.273, 11790.629), (212.0, 5.847, 1577.344), (186.0, 5.022, 10977.079),
    (175.0, 3.012, 18849.228), (110.0, 5.055, 5486.778), (98.0, 0.89, 6069.78),
    (86.0, 5.69, 15720.84), (86.0, 1.27, 161000.69), (65.0, 0.27, 17260.15),
    (63.0, 0.92, 529.69), (57.0, 2.01, 83996.85), (56.0, 5.24, 71430.70),
    (49.0, 3.25, 2544.31), (47.0, 2.58, 775.52), (45.0, 5.54, 9437.76),
    (43.0, 6.01, 6275.96), (39.0, 5.36, 4694.00), (38.0, 2.39, 8827.39),
    (37.0, 0.83, 19651.05), (37.0, 4.90, 12139.55), (36.0, 1.67, 12036.46),
    (35.0, 1.84, 2942.46), (33.0, 0.24, 7084.90), (32.0, 0.18, 5088.63),
    (32.0, 1.78, 398.15), (28.0, 1.21, 6286.60), (28.0, 1.90, 6279.55),
    (26.0, 4.59, 10447.39),
];
const R1: &[(f64, f64, f64)] = &[
    (103019.0, 1.107490, 6283.075850),
    (1721.0, 1.0644, 12566.1517),
    (702.0, 3.142, 0.0),
    (32.0, 1.02, 18849.23),
    (31.0, 2.84, 5507.55),
    (25.0, 1.32, 5223.69),
    (18.0, 1.42, 1577.34),
    (10.0, 5.91, 10977.08),
    (9.0, 1.42, 6275.96),
    (9.0, 0.27, 5486.78),
];
const R2: &[(f64, f64, f64)] =
    &[(4359.0, 5.7846, 6283.0758), (124.0, 5.579, 12566.152), (12.0, 3.14, 0.0), (9.0, 3.63, 77713.77), (6.0, 1.87, 5573.14), (3.0, 5.47, 18849.23)];
const R3: &[(f64, f64, f64)] = &[(145.0, 4.273, 6283.076), (7.0, 3.92, 12566.15)];
const R4: &[(f64, f64, f64)] = &[(4.0, 2.56, 6283.08)];

// Moon, Meeus table 47.A: multiples of D, M, M', F, then the longitude sum (1e-6 degree) and
// the distance sum (1e-3 km).
#[rustfmt::skip]
const LONGITUDE_DISTANCE: &[(i8, i8, i8, i8, f64, f64)] = &[
    (0, 0, 1, 0, 6288774.0, -20905355.0), (2, 0, -1, 0, 1274027.0, -3699111.0), (2, 0, 0, 0, 658314.0, -2955968.0),
    (0, 0, 2, 0, 213618.0, -569925.0), (0, 1, 0, 0, -185116.0, 48888.0), (0, 0, 0, 2, -114332.0, -3149.0),
    (2, 0, -2, 0, 58793.0, 246158.0), (2, -1, -1, 0, 57066.0, -152138.0), (2, 0, 1, 0, 53322.0, -170733.0),
    (2, -1, 0, 0, 45758.0, -204586.0), (0, 1, -1, 0, -40923.0, -129620.0), (1, 0, 0, 0, -34720.0, 108743.0),
    (0, 1, 1, 0, -30383.0, 104755.0), (2, 0, 0, -2, 15327.0, 10321.0), (0, 0, 1, 2, -12528.0, 0.0),
    (0, 0, 1, -2, 10980.0, 79661.0), (4, 0, -1, 0, 10675.0, -34782.0), (0, 0, 3, 0, 10034.0, -23210.0),
    (4, 0, -2, 0, 8548.0, -21636.0), (2, 1, -1, 0, -7888.0, 24208.0), (2, 1, 0, 0, -6766.0, 30824.0),
    (1, 0, -1, 0, -5163.0, -8379.0), (1, 1, 0, 0, 4987.0, -16675.0), (2, -1, 1, 0, 4036.0, -12831.0),
    (2, 0, 2, 0, 3994.0, -10445.0), (4, 0, 0, 0, 3861.0, -11650.0), (2, 0, -3, 0, 3665.0, 14403.0),
    (0, 1, -2, 0, -2689.0, -7003.0), (2, 0, -1, 2, -2602.0, 0.0), (2, -1, -2, 0, 2390.0, 10056.0),
    (1, 0, 1, 0, -2348.0, 6322.0), (2, -2, 0, 0, 2236.0, -9884.0), (0, 1, 2, 0, -2120.0, 5751.0),
    (0, 2, 0, 0, -2069.0, 0.0), (2, -2, -1, 0, 2048.0, -4950.0), (2, 0, 1, -2, -1773.0, 4130.0),
    (2, 0, 0, 2, -1595.0, 0.0), (4, -1, -1, 0, 1215.0, -3958.0), (0, 0, 2, 2, -1110.0, 0.0),
    (3, 0, -1, 0, -892.0, 3258.0), (2, 1, 1, 0, -810.0, 2616.0), (4, -1, -2, 0, 759.0, -1897.0),
    (0, 2, -1, 0, -713.0, -2117.0), (2, 2, -1, 0, -700.0, 2354.0), (2, 1, -2, 0, 691.0, 0.0),
    (2, -1, 0, -2, 596.0, 0.0), (4, 0, 1, 0, 549.0, -1423.0), (0, 0, 4, 0, 537.0, -1117.0),
    (4, -1, 0, 0, 520.0, -1571.0), (1, 0, -2, 0, -487.0, -1739.0), (2, 1, 0, -2, -399.0, 0.0),
    (0, 0, 2, -2, -381.0, -4421.0), (1, 1, 1, 0, 351.0, 0.0), (3, 0, -2, 0, -340.0, 0.0),
    (4, 0, -3, 0, 330.0, 0.0), (2, -1, 2, 0, 327.0, 0.0), (0, 2, 1, 0, -323.0, 1165.0),
    (1, 1, -1, 0, 299.0, 0.0), (2, 0, 3, 0, 294.0, 0.0), (2, 0, -1, -2, 0.0, 8752.0),
];

// Moon, Meeus table 47.B: the latitude sum (1e-6 degree).
#[rustfmt::skip]
const LATITUDE: &[(i8, i8, i8, i8, f64)] = &[
    (0, 0, 0, 1, 5128122.0), (0, 0, 1, 1, 280602.0), (0, 0, 1, -1, 277693.0), (2, 0, 0, -1, 173237.0),
    (2, 0, -1, 1, 55413.0), (2, 0, -1, -1, 46271.0), (2, 0, 0, 1, 32573.0), (0, 0, 2, 1, 17198.0),
    (2, 0, 1, -1, 9266.0), (0, 0, 2, -1, 8822.0), (2, -1, 0, -1, 8216.0), (2, 0, -2, -1, 4324.0),
    (2, 0, 1, 1, 4200.0), (2, 1, 0, -1, -3359.0), (2, -1, -1, 1, 2463.0), (2, -1, 0, 1, 2211.0),
    (2, -1, -1, -1, 2065.0), (0, 1, -1, -1, -1870.0), (4, 0, -1, -1, 1828.0), (0, 1, 0, 1, -1794.0),
    (0, 0, 0, 3, -1749.0), (0, 1, -1, 1, -1565.0), (1, 0, 0, 1, -1491.0), (0, 1, 1, 1, -1475.0),
    (0, 1, 1, -1, -1410.0), (0, 1, 0, -1, -1344.0), (1, 0, 0, -1, -1335.0), (0, 0, 3, 1, 1107.0),
    (4, 0, 0, -1, 1021.0), (4, 0, -1, 1, 833.0), (0, 0, 1, -3, 777.0), (4, 0, -2, 1, 671.0),
    (2, 0, 0, -3, 607.0), (2, 0, 2, -1, 596.0), (2, -1, 1, -1, 491.0), (2, 0, -2, 1, -451.0),
    (0, 0, 3, -1, 439.0), (2, 0, 2, 1, 422.0), (2, 0, -3, -1, 421.0), (2, 1, -1, 1, -366.0),
    (2, 1, 0, 1, -351.0), (4, 0, 0, 1, 331.0), (2, -1, 1, 1, 315.0), (2, -2, 0, -1, 302.0),
    (0, 0, 1, 3, -283.0), (2, 1, 1, -1, -229.0), (1, 1, 0, -1, 223.0), (1, 1, 0, 1, 223.0),
    (0, 1, -2, -1, -220.0), (2, 1, -1, -1, -220.0), (1, 0, 1, 1, -185.0), (2, -1, -2, -1, 181.0),
    (0, 1, 2, 1, -177.0), (4, 0, -2, -1, 176.0), (4, -1, -1, -1, 166.0), (1, 0, 1, -1, -164.0),
    (4, 0, 1, -1, 132.0), (1, 0, -1, -1, -119.0), (4, -1, 0, -1, 115.0), (2, -2, 0, 1, 107.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    // Meeus example 47.a: 1992 April 12, 0h TD (JDE 2448724.5).
    #[test]
    fn moon_matches_the_worked_example() {
        let t = (2_448_724.5 - 2_451_545.0) / 36_525.0;
        let m = moon_mean_of_date(t);
        assert!((m.lon.to_degrees() - 133.162655).abs() < 0.0005, "{}", m.lon.to_degrees());
        assert!((m.lat.to_degrees() + 3.229126).abs() < 0.0005, "{}", m.lat.to_degrees());
        assert!((m.dist - 368_409.7).abs() < 1.0, "{}", m.dist);
    }

    // Meeus example 25.b: 1992 October 13, 0h TD. The geometric longitude is 199.907372 degrees
    // (his L = 19.907372 plus 180) and R = 0.99760775 au.
    #[test]
    fn sun_matches_the_worked_example() {
        let t = (2_448_908.5 - 2_451_545.0) / 36_525.0;
        let s = sun_mean_of_date(t);
        assert!((s.lon.to_degrees() - 199.907372).abs() < 0.0005, "{}", s.lon.to_degrees());
        assert!((s.dist / AU_KM - 0.99760775).abs() < 0.00001, "{}", s.dist / AU_KM);
    }

    #[test]
    fn delta_t_follows_the_published_values() {
        // 63.86 s in 2000. The prediction for 2024 is some 4 s above the measured 69.2 s, which
        // moves a shadow by about 2 km.
        assert!((delta_t(2000.0) - 63.86).abs() < 0.01);
        assert!((delta_t(2024.0) - 69.2).abs() < 6.0);
        assert!((delta_t(1900.0) + 2.79).abs() < 0.01);
        // The pieces meet.
        for y in [1600.0, 1700.0, 1800.0, 1860.0, 1900.0, 1920.0, 1941.0, 1961.0, 1986.0, 2005.0, 2050.0] {
            assert!((delta_t(y - 1e-9) - delta_t(y)).abs() < 2.0, "{y}");
        }
    }
}

#[cfg(test)]
mod topocentric_tests {
    use super::*;
    use crate::geo::geodetic_to_ecef;
    use crate::sky_time::parse_iso8601;

    fn separation_deg(lat: f64, lon: f64, time: &str) -> f64 {
        let ecef = geodetic_to_ecef(lat.to_radians(), lon.to_radians(), 200.0);
        SunMoon::at(parse_iso8601(time).unwrap()).topocentric(ecef).separation().to_degrees()
    }

    #[test]
    fn moon_covers_the_sun_over_dallas_in_2024() {
        let t = "2024-04-08T18:42:00Z";
        let near = separation_deg(32.78, -96.8, t);
        assert!(near < 0.1, "{near}");
        // From Honolulu or London, or from Dallas at another time, the Moon is well away from the Sun.
        assert!(separation_deg(21.3, -157.8, t) > 0.5, "Honolulu");
        assert!(separation_deg(51.5, 0.0, t) > 0.5, "London");
        assert!(separation_deg(32.78, -96.8, "2024-04-08T21:00:00Z") > 0.5);
    }

    #[test]
    fn parallax_moves_the_moon_by_up_to_a_degree() {
        // Geocentric against topocentric, for a place where the Moon is low in the sky.
        let time = parse_iso8601("2024-04-08T18:42:00Z").unwrap();
        let sm = SunMoon::at(time);
        let geo = (sm.to_earth_fixed() * sm.moon).normalize();
        let far = sm.topocentric(geodetic_to_ecef(0.0, 0.0, 0.0)).moon;
        assert!(geo.dot(far).clamp(-1.0, 1.0).acos().to_degrees() > 0.3);
    }

    #[test]
    fn disc_overlap_limits() {
        assert_eq!(disc_overlap(1.0, 1.0, 2.5), 0.0);
        assert_eq!(disc_overlap(1.0, 1.1, 0.05), 1.0);
        assert!((disc_overlap(1.0, 0.5, 0.2) - 0.25).abs() < 1e-12);
        let half = disc_overlap(1.0, 1.0, 1.0);
        assert!((half - 0.391).abs() < 0.001, "{half}");
    }

    #[test]
    fn totality_covers_the_whole_disc() {
        let ecef = geodetic_to_ecef(32.78_f64.to_radians(), (-96.8_f64).to_radians(), 200.0);
        let sky = SunMoon::at(parse_iso8601("2024-04-08T18:42:00Z").unwrap()).topocentric(ecef);
        assert!(sky.obscuration() > 0.99, "{}", sky.obscuration());
    }
}
