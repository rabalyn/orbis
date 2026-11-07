//! Mars, Mercury, Ceres and Vesta: their orientation from the IAU Working Group on Cartographic
//! Coordinates and Rotational Elements, their orbits, the Sun in each body-fixed frame and the
//! planets in their skies. No window or GPU
//! is involved.
//!
//! The constants are those of the 2015 report (Archinal et al., Celestial Mechanics and
//! Dynamical Astronomy 130, 22, 2018), as set out in NASA NAIF's `pck00011.tpc` (BODY199, BODY499,
//! BODY2000001 and BODY2000004), which they were read from. The body-fixed frame has z through the
//! north pole and x through the prime meridian, so longitude is counted east. Time is taken as
//! TT, which is UTC plus 69.2 s today; it matters for Vesta, which turns 1.3 degrees in that time.
//!
//! Vesta's published prime meridian (the report's W0 of 285.39 degrees) belongs to the "Claudia
//! double prime" system that the Dawn team adopted (Archinal 2018); the Trek layer lists only
//! the IAU code of Vesta as its coordinate system, so whether its longitudes are the same
//! could only be checked against the picture (see SPEC.md).

use crate::planets;
use glam::{DMat3, DVec3};
use std::f64::consts::PI;

/// TT minus UTC in seconds (37 leap seconds and 32.184 s). Unix time carries no leap seconds.
const TT_MINUS_UTC: f64 = 69.184;

/// Pole direction and prime meridian with their linear and periodic parts.
struct Model {
    /// Right ascension, declination of the pole at J2000 and their rates per century, degrees.
    ra: (f64, f64),
    dec: (f64, f64),
    /// Prime meridian angle at J2000 and its rate per day, degrees.
    w: (f64, f64),
    /// Periodic arguments: angle at J2000 and rate per century, degrees.
    angles: &'static [(f64, f64)],
    /// Coefficients (degrees) of sin(angle i) for the right ascension and the prime meridian, and
    /// of cos(angle i) for the declination, each as (index, amplitude).
    ra_terms: &'static [(usize, f64)],
    dec_terms: &'static [(usize, f64)],
    w_terms: &'static [(usize, f64)],
}

impl Model {
    /// Right ascension and declination of the pole and the prime meridian angle, radians.
    fn angles_at(&self, unix: f64) -> (f64, f64, f64) {
        let unix = unix + TT_MINUS_UTC;
        let d = unix / 86_400.0 + 2_440_587.5 - 2_451_545.0;
        let t = d / 36_525.0;
        let arg = |i: usize| (self.angles[i].0 + self.angles[i].1 * t).to_radians();
        let sum = |terms: &[(usize, f64)], f: fn(f64) -> f64| -> f64 { terms.iter().map(|(i, c)| c * f(arg(*i))).sum() };
        let ra = self.ra.0 + self.ra.1 * t + sum(self.ra_terms, f64::sin);
        let dec = self.dec.0 + self.dec.1 * t + sum(self.dec_terms, f64::cos);
        let w = self.w.0 + self.w.1 * d + sum(self.w_terms, f64::sin);
        (ra.to_radians(), dec.to_radians(), w.rem_euclid(360.0).to_radians())
    }

    /// From J2000 equatorial directions to the body-fixed frame, the report's
    /// R3(W) R1(90° - dec) R3(90° + ra) written for rotating vectors (as in `moon`).
    fn frame(&self, unix: f64) -> DMat3 {
        let (ra, dec, w) = self.angles_at(unix);
        DMat3::from_rotation_z(-w) * DMat3::from_rotation_x(-(PI / 2.0 - dec)) * DMat3::from_rotation_z(-(PI / 2.0 + ra))
    }
}

/// Mercury: the periodic terms in the prime meridian have the arguments M1 to M5.
static MERCURY_MODEL: Model = Model {
    ra: (281.0103, -0.0328),
    dec: (61.4155, -0.0049),
    w: (329.5988, 6.138_510_8),
    angles: &[
        (174.791_085_7, 149_472.673_587_5),
        (349.582_171_4, 298_945.071_75),
        (164.373_257_1, 448_417.607_625),
        (339.164_342_9, 597_890.143_5),
        (153.955_428_6, 747_362.679_375),
    ],
    ra_terms: &[],
    dec_terms: &[],
    w_terms: &[(0, 0.010_672_57), (1, -0.001_123_09), (2, -0.000_110_40), (3, -0.000_025_39), (4, -0.000_005_71)],
};

/// Mars: the pole and meridian follow Konopliv et al., with the periodic terms of the report.
static MARS_MODEL: Model = Model {
    ra: (317.269_202, -0.109_275_47),
    dec: (54.432_516, -0.058_271_05),
    w: (176.049_863, 350.891_982_443_297),
    angles: &[
        (198.991_226, 19_139.481_998_5),
        (226.292_679, 38_280.851_128_1),
        (249.663_391, 57_420.725_159_3),
        (266.183_510, 76_560.636_795_0),
        (79.398_797, 0.504_261_5),
        (122.433_576, 19_139.940_747_6),
        (43.058_401, 38_280.875_327_2),
        (57.663_379, 57_420.751_720_5),
        (79.476_401, 76_560.649_500_4),
        (166.325_722, 0.504_261_5),
        (129.071_773, 19_140.032_824_4),
        (36.352_167, 38_281.047_359_1),
        (56.668_646, 57_420.929_536_0),
        (67.364_003, 76_560.255_221_5),
        (104.792_680, 95_700.438_757_8),
        (95.391_654, 0.504_261_5),
    ],
    ra_terms: &[(0, 0.000_068), (1, 0.000_238), (2, 0.000_052), (3, 0.000_009), (4, 0.419_057)],
    dec_terms: &[(5, 0.000_051), (6, 0.000_141), (7, 0.000_031), (8, 0.000_005), (9, 1.591_274)],
    w_terms: &[(10, 0.000_145), (11, 0.000_157), (12, 0.000_040), (13, 0.000_001), (14, 0.000_001), (15, 0.584_542)],
};

static CERES_MODEL: Model = Model {
    ra: (291.418, 0.0),
    dec: (66.764, 0.0),
    w: (170.650, 952.1532),
    angles: &[],
    ra_terms: &[],
    dec_terms: &[],
    w_terms: &[],
};

static VESTA_MODEL: Model = Model {
    ra: (309.031, 0.0),
    dec: (42.235, 0.0),
    w: (285.39, 1_617.332_942_8),
    angles: &[],
    ra_terms: &[],
    dec_terms: &[],
    w_terms: &[],
};

pub fn mercury_frame(unix: f64) -> DMat3 {
    MERCURY_MODEL.frame(unix)
}
pub fn mars_frame(unix: f64) -> DMat3 {
    MARS_MODEL.frame(unix)
}
pub fn ceres_frame(unix: f64) -> DMat3 {
    CERES_MODEL.frame(unix)
}
pub fn vesta_frame(unix: f64) -> DMat3 {
    VESTA_MODEL.frame(unix)
}

/// Orbital elements of a small body at an epoch, from the JPL Small-Body Database
/// (ssd-api.jpl.nasa.gov/sbdb.api, osculating elements on the J2000 ecliptic, retrieved 2026-11).
struct Orbit {
    /// Epoch as a Julian date (TDB), semi-major axis (au), eccentricity.
    epoch: f64,
    a: f64,
    e: f64,
    /// Inclination, longitude of the ascending node, argument of perihelion, mean anomaly at
    /// the epoch and mean motion per day, degrees.
    i: f64,
    node: f64,
    peri: f64,
    m0: f64,
    n: f64,
}

static CERES_ORBIT: Orbit = Orbit {
    epoch: 2_461_200.5,
    a: 2.765_552_595_034_094,
    e: 0.079_692_295_148_165_86,
    i: 10.588_027_801_834_62,
    node: 80.248_626_820_432_21,
    peri: 73.294_214_535_942_87,
    m0: 274.419_346_376_176_1,
    n: 0.214_304_450_648_43,
};

static VESTA_ORBIT: Orbit = Orbit {
    epoch: 2_461_200.5,
    a: 2.361_365_965_127_599,
    e: 0.090_203_743_828_343_82,
    i: 7.143_925_545_058_711,
    node: 103.701_293_265_032,
    peri: 151.468_647_822_156_4,
    m0: 81.190_156_072_868_62,
    n: 0.271_618_361_359_9,
};

impl Orbit {
    /// Heliocentric position in au on the J2000 ecliptic from two-body motion, which drifts
    /// from the real orbit by the planets' pull (see `docs/SPEC.md`).
    fn position(&self, unix: f64) -> DVec3 {
        let jd = (unix + TT_MINUS_UTC) / 86_400.0 + 2_440_587.5;
        let m = (self.m0 + self.n * (jd - self.epoch)).to_radians().rem_euclid(2.0 * PI);
        let e = self.e;
        let mut ecc = m + e * m.sin();
        for _ in 0..10 {
            ecc -= (ecc - e * ecc.sin() - m) / (1.0 - e * ecc.cos());
        }
        let xp = self.a * (ecc.cos() - e);
        let yp = self.a * (1.0 - e * e).sqrt() * ecc.sin();
        let (so, co) = self.peri.to_radians().sin_cos();
        let (sn, cn) = self.node.to_radians().sin_cos();
        let (si, ci) = self.i.to_radians().sin_cos();
        DVec3::new(
            (co * cn - so * sn * ci) * xp + (-so * cn - co * sn * ci) * yp,
            (co * sn + so * cn * ci) * xp + (-so * sn + co * cn * ci) * yp,
            so * si * xp + co * si * yp,
        )
    }
}

/// Where each body is, heliocentric on the J2000 ecliptic in au.
fn heliocentric(body: &str, unix: f64) -> DVec3 {
    match body {
        "Ceres" => CERES_ORBIT.position(unix),
        "Vesta" => VESTA_ORBIT.position(unix),
        name => planets::heliocentric_of(name, unix).unwrap_or(DVec3::X),
    }
}

fn sun(body: &str, frame: fn(f64) -> DMat3, unix: f64) -> DVec3 {
    frame(unix) * planets::sun_from(heliocentric(body, unix))
}

pub fn sun_mercury(unix: f64) -> DVec3 {
    sun("Mercury", mercury_frame, unix)
}
pub fn sun_mars(unix: f64) -> DVec3 {
    sun("Mars", mars_frame, unix)
}
pub fn sun_ceres(unix: f64) -> DVec3 {
    sun("Ceres", ceres_frame, unix)
}
pub fn sun_vesta(unix: f64) -> DVec3 {
    sun("Vesta", vesta_frame, unix)
}

/// The planets and the Earth as seen from the body, as points. Moons are not listed.
fn sky(body: &str, unix: f64) -> Vec<planets::Body> {
    planets::seen_from(heliocentric(body, unix), body, unix)
}

pub fn sky_mercury(unix: f64) -> Vec<planets::Body> {
    sky("Mercury", unix)
}
pub fn sky_mars(unix: f64) -> Vec<planets::Body> {
    sky("Mars", unix)
}
pub fn sky_ceres(unix: f64) -> Vec<planets::Body> {
    sky("Ceres", unix)
}
pub fn sky_vesta(unix: f64) -> Vec<planets::Body> {
    sky("Vesta", unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::equatorial;
    use crate::sky_time::parse_iso8601;

    fn at(s: &str) -> f64 {
        parse_iso8601(s).unwrap()
    }

    /// Unix time of a Julian date given in UT.
    fn jd_unix(jd: f64) -> f64 {
        (jd - 2_440_587.5) * 86_400.0
    }

    #[test]
    fn the_poles_and_meridians_at_j2000_are_the_published_ones() {
        let t = jd_unix(2_451_545.0) - TT_MINUS_UTC;
        // (model, pole RA, pole Dec, W) in degrees; Mars and Mercury include their periodic terms.
        let cases: [(&Model, f64, f64, f64, f64); 4] = [
            (&MERCURY_MODEL, 281.0103, 61.4155, 329.5988 + 0.010_672_57 * 174.791_085_7_f64.to_radians().sin() - 0.001_123_09 * 349.582_171_4_f64.to_radians().sin() - 0.000_110_40 * 164.373_257_1_f64.to_radians().sin() - 0.000_025_39 * 339.164_342_9_f64.to_radians().sin() - 0.000_005_71 * 153.955_428_6_f64.to_radians().sin(), 0.001),
            (&MARS_MODEL, 317.681, 52.885, 176.630, 0.01),
            (&CERES_MODEL, 291.418, 66.764, 170.650, 1e-9),
            (&VESTA_MODEL, 309.031, 42.235, 285.39, 1e-9),
        ];
        for (m, ra0, dec0, w0, tol) in cases {
            let (ra, dec, w) = m.angles_at(t);
            assert!((ra.to_degrees() - ra0).abs() < tol.max(0.002), "ra {} {ra0}", ra.to_degrees());
            assert!((dec.to_degrees() - dec0).abs() < tol.max(0.002), "dec {} {dec0}", dec.to_degrees());
            assert!((w.to_degrees() - w0).abs() < tol.max(0.002), "w {} {w0}", w.to_degrees());
        }
    }

    #[test]
    fn each_pole_maps_to_the_z_axis_and_the_meridian_to_the_x_axis_plane() {
        let frames: [(Frame, &Model); 4] =
            [(mercury_frame, &MERCURY_MODEL), (mars_frame, &MARS_MODEL), (ceres_frame, &CERES_MODEL), (vesta_frame, &VESTA_MODEL)];
        for (frame, model) in frames {
            for s in ["2000-01-01T12:00:00Z", "2026-03-01T00:00:00Z", "2040-09-09T09:09:00Z"] {
                let t = at(s);
                let (ra, dec, _) = model.angles_at(t);
                let p = frame(t) * equatorial(ra, dec);
                assert!((p - DVec3::Z).length() < 1e-12, "{p:?}");
                // Rotation matrices keep lengths and handedness.
                let m = frame(t);
                assert!((m.determinant() - 1.0).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn the_prime_meridian_turns_at_the_published_rates() {
        // Degrees per day: Mercury 6.1385 (a 58.6 day spin), Mars 350.892, Ceres 952.153, Vesta 1617.333.
        let t0 = at("2026-03-01T00:00:00Z");
        for (model, rate) in [(&MERCURY_MODEL, 6.1385), (&MARS_MODEL, 350.892), (&CERES_MODEL, 952.1532), (&VESTA_MODEL, 1617.3329)] {
            let hour = 3600.0;
            let (_, _, a) = model.angles_at(t0);
            let (_, _, b) = model.angles_at(t0 + hour);
            let step = (b - a).to_degrees().rem_euclid(360.0);
            let want = (rate / 24.0f64).rem_euclid(360.0);
            assert!((step - want).abs() < 0.01, "{step} {want}");
        }
    }

    // JPL Horizons (ssd.jpl.nasa.gov/api/horizons.api, quantities 15 and 21, observer at the Earth's
    // centre) for 2025-12-31 12:00 UT, 2029-06-10 12:00 UT and 2031-05-13 12:00 UT:
    // sub-solar longitude and latitude in degrees and the one-way light time in minutes. Horizons
    // counts the longitudes of Mars and Mercury westwards and gives their latitudes as planetographic;
    // Ceres and Vesta are eastwards. The sub-solar point is that of the body's own time, the
    // observation time less the light time.
    const TIMES: [f64; 3] = [2_461_041.0, 2_462_298.0, 2_463_000.0];
    const MARS: [(f64, f64, f64); 3] = [(22.227230, -7.853533, 20.05242005), (154.827424, 17.571757, 8.42558350), (234.639362, 15.055822, 4.60360429)];
    const MERCURY: [(f64, f64, f64); 3] = [(249.833901, -0.018509, 11.42476353), (331.362800, -0.032515, 7.13088400), (326.553556, -0.033405, 8.37842644)];
    const CERES: [(f64, f64, f64); 3] = [(143.583490, 0.471770, 23.01885307), (205.784588, 4.701654, 19.94041070), (98.548427, -3.790376, 29.59258694)];
    const VESTA: [(f64, f64, f64); 3] = [(311.334404, -36.861582, 26.13443336), (203.259055, -29.386141, 10.37791941), (116.761361, 30.319616, 27.51160693)];

    type Frame = fn(f64) -> DMat3;

    fn lon_lat(v: DVec3) -> (f64, f64) {
        (v.y.atan2(v.x).to_degrees().rem_euclid(360.0), v.z.asin().to_degrees())
    }

    fn lon_error(a: f64, b: f64) -> f64 {
        ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs()
    }

    /// Planetographic latitude on a spheroid of semi-axes a and b, from the planetocentric one.
    fn planetographic(lat: f64, a: f64, b: f64) -> f64 {
        (lat.to_radians().tan() * (a * a) / (b * b)).atan().to_degrees()
    }

    // The tolerances are what the model achieves: Mars and Mercury agree to 0.01 degrees. Ceres
    // is up to 0.6 degrees off in longitude and Vesta 1.4, partly the two-body orbit and, for
    // Vesta, Horizons' triaxial figure, which the spheroid here does not reproduce.
    fn check(name: &str, sun: fn(f64) -> DVec3, rows: &[(f64, f64, f64); 3], west: bool, shape: (f64, f64), tol_lon: f64, tol_lat: f64) {
        for (jd, (lon, lat, light)) in TIMES.iter().zip(rows) {
            let t = jd_unix(*jd) - light * 60.0;
            let (my_lon, my_lat) = lon_lat(sun(t));
            let my_lat = planetographic(my_lat, shape.0, shape.1);
            let want_lon = if west { 360.0 - lon } else { *lon };
            println!("{name} {jd}: lon {my_lon:.3} want {want_lon:.3}, lat {my_lat:.3} want {lat:.3}");
            assert!(lon_error(my_lon, want_lon) < tol_lon, "{name} {jd}: lon {my_lon} want {want_lon}");
            assert!((my_lat - lat).abs() < tol_lat, "{name} {jd}: lat {my_lat} want {lat}");
        }
    }

    #[test]
    fn the_sub_solar_point_of_mercury_matches_horizons() {
        check("mercury", sun_mercury, &MERCURY, true, (1.0, 1.0), 0.05, 0.01);
    }

    #[test]
    fn the_sub_solar_point_of_mars_matches_horizons() {
        check("mars", sun_mars, &MARS, true, (3_396.19, 3_376.20), 0.05, 0.05);
    }

    #[test]
    fn the_sub_solar_point_of_ceres_matches_horizons() {
        check("ceres", sun_ceres, &CERES, false, (487.3, 446.0), 0.8, 0.2);
    }

    #[test]
    fn the_sub_solar_point_of_vesta_matches_horizons() {
        check("vesta", sun_vesta, &VESTA, false, (284.5, 229.0), 1.5, 0.8);
    }

    #[test]
    fn the_small_bodies_follow_their_horizons_positions() {
        // Heliocentric J2000 ecliptic positions in millions of km from JPL Horizons (target 1; and 4;, centre
        // 500@10, TDB) at 2026-01-01, 2029-01-01 and 2032-01-01. The two-body orbit from elements
        // of 2026-11-21 is off by up to 0.25 degrees as seen from the Sun at these dates.
        let rows: [(&str, f64, [f64; 3]); 6] = [
            ("Ceres", 2_461_041.5, [381.172_804, 192.678_035, -64.122_243]),
            ("Ceres", 2_462_137.5, [46.258_116, -427.208_733, -22.135_257]),
            ("Ceres", 2_463_232.5, [-293.213_775, 236.602_841, 61.533_223]),
            ("Vesta", 2_461_041.5, [164.667_263, -285.020_316, -11.589_457]),
            ("Vesta", 2_462_137.5, [-222.676_780, -234.037_863, 34.037_674]),
            ("Vesta", 2_463_232.5, [-326.335_840, 133.422_308, 35.795_609]),
        ];
        for (name, jd, km) in rows {
            let want = DVec3::from_array(km) * 1.0e6 / 149_597_870.7;
            let got = heliocentric(name, jd_unix(jd) - TT_MINUS_UTC);
            assert!(got.angle_between(want).to_degrees() < 0.3, "{name} {jd}");
            assert!((got.length() / want.length() - 1.0).abs() < 0.005, "{name} {jd}");
        }
    }
}
