//! The bodies that can be viewed. Each is an immutable table entry: its shape, how its
//! imagery is tiled, what data exists for it, and how it turns. The body is chosen at startup
//! (`--body`) and can be changed while the program runs; the geodesy, tile addressing and sky
//! follow it.

use crate::geo::Grid;
use crate::sources::{self, TileSource};
use glam::{DMat3, DVec3};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Body {
    pub id: &'static str,
    pub name: &'static str,
    /// Equatorial semi-axis in metres, the polar one, and the squared first eccentricity.
    pub a: f64,
    pub b: f64,
    pub e2: f64,
    pub grid: Grid,
    /// Whether air scatters light: the blue glow and sky, and soft twilight.
    pub atmosphere: bool,
    /// Whether terrain, roads, labels, buildings and national aerial imagery exist. Without
    /// them the body is a smooth sphere carrying its imagery alone.
    pub surface_data: bool,
    /// The one imagery source, for a body without `surface_data`.
    pub imagery: &'static TileSource,
    /// Credit lines shown while the body is in view.
    pub credit: &'static str,
    /// From J2000 equatorial directions to the body-fixed frame at a Unix time.
    pub rotation: fn(f64) -> DMat3,
    /// Unit vector towards the Sun in the body-fixed frame at a Unix time.
    pub sun: fn(f64) -> DVec3,
    /// The planets and moons that show in its sky, and how big and bright they are.
    pub sky_bodies: fn(f64) -> Vec<crate::planets::Body>,
    /// The colour of the plain surface shown where no tile has arrived, RGBA.
    pub ground: Option<[u8; 4]>,
}

pub static EARTH: Body = Body {
    id: "earth",
    name: "Earth",
    a: 6_378_137.0,
    b: 6_356_752.314_245,
    e2: 6.694_379_990_14e-3,
    grid: Grid::Mercator,
    atmosphere: true,
    surface_data: true,
    imagery: &sources::EOX,
    credit: crate::CREDITS,
    rotation: crate::sky::earth_fixed,
    sun: |unix| crate::sky_time::Sun::at(unix).direction(),
    sky_bodies: crate::planets::bodies,
    ground: None,
};

/// The Moon as a sphere of the IAU mean radius, 1737.4 km (Archinal et al. 2011). Its
/// imagery is the LRO WAC mosaic from NASA Moon Trek; no elevation data is used.
pub static MOON: Body = Body {
    id: "moon",
    name: "Moon",
    a: 1_737_400.0,
    b: 1_737_400.0,
    e2: 0.0,
    grid: Grid::Equirect,
    atmosphere: false,
    surface_data: false,
    imagery: &sources::MOON_WAC,
    credit: "Moon: LROC WAC mosaic, NASA/GSFC/Arizona State University, via NASA Moon Trek https://trek.nasa.gov/moon/",
    rotation: crate::moon::fixed_frame,
    sun: crate::moon::sun_fixed,
    sky_bodies: crate::planets::bodies_from_moon,
    ground: Some([58, 58, 58, 255]),
};

/// Mars as the IAU 2015 spheroid, 3396.19 km equatorial and 3376.20 km polar radius. Its imagery is the Viking MDIM 2.1 colour mosaic from NASA Mars Trek. The air is not drawn.
pub static MARS: Body = Body {
    id: "mars",
    name: "Mars",
    a: 3_396_190.0,
    b: 3_376_200.0,
    e2: 1.0 - (3_376_200.0 / 3_396_190.0) * (3_376_200.0 / 3_396_190.0),
    grid: Grid::Equirect,
    atmosphere: false,
    surface_data: false,
    imagery: &sources::MARS_VIKING,
    credit: "Mars: Viking MDIM 2.1 colour mosaic, NASA/JPL/USGS, via NASA Mars Trek https://trek.nasa.gov/mars/",
    rotation: crate::planetary::mars_frame,
    sun: crate::planetary::sun_mars,
    sky_bodies: crate::planetary::sky_mars,
    ground: Some([150, 95, 70, 255]),
};

/// Mercury as a sphere of the IAU mean radius, 2439.4 km (Archinal et al. 2018).
pub static MERCURY: Body = Body {
    id: "mercury",
    name: "Mercury",
    a: 2_439_400.0,
    b: 2_439_400.0,
    e2: 0.0,
    grid: Grid::Equirect,
    atmosphere: false,
    surface_data: false,
    imagery: &sources::MERCURY_MDIS,
    credit: "Mercury: MESSENGER MDIS basemap, NASA/JHUAPL/Carnegie Institution of Washington/USGS, via NASA Mercury Trek https://trek.nasa.gov/mercury/",
    rotation: crate::planetary::mercury_frame,
    sun: crate::planetary::sun_mercury,
    sky_bodies: crate::planetary::sky_mercury,
    ground: Some([105, 100, 96, 255]),
};

/// Ceres as the spheroid with the radii of NAIF's pck00011.tpc: 487.3 km equatorial and 446.0 km polar.
pub static CERES: Body = Body {
    id: "ceres",
    name: "Ceres",
    a: 487_300.0,
    b: 446_000.0,
    e2: 1.0 - (446_000.0 / 487_300.0) * (446_000.0 / 487_300.0),
    grid: Grid::Equirect,
    atmosphere: false,
    surface_data: false,
    imagery: &sources::CERES_FC,
    credit: "Ceres: Dawn FC HAMO colour mosaic, NASA/JPL-Caltech/UCLA/MPS/DLR/IDA, via NASA Ceres Trek https://trek.nasa.gov/ceres/",
    rotation: crate::planetary::ceres_frame,
    sun: crate::planetary::sun_ceres,
    sky_bodies: crate::planetary::sky_ceres,
    ground: Some([85, 83, 80, 255]),
};

/// Vesta as a spheroid. The published figure is a triaxial ellipsoid of 289, 280 and 229 km (pck00011.tpc); the two equatorial axes are averaged to 284.5 km.
pub static VESTA: Body = Body {
    id: "vesta",
    name: "Vesta",
    a: 284_500.0,
    b: 229_000.0,
    e2: 1.0 - (229_000.0 / 284_500.0) * (229_000.0 / 284_500.0),
    grid: Grid::Equirect,
    atmosphere: false,
    surface_data: false,
    imagery: &sources::VESTA_FC,
    credit: "Vesta: Dawn FC HAMO mosaic, NASA/JPL-Caltech/UCLA/MPS/DLR/IDA, via NASA Vesta Trek https://trek.nasa.gov/vesta/",
    rotation: crate::planetary::vesta_frame,
    sun: crate::planetary::sun_vesta,
    sky_bodies: crate::planetary::sky_vesta,
    ground: Some([100, 97, 92, 255]),
};

static ALL: [&Body; 6] = [&EARTH, &MOON, &MARS, &MERCURY, &CERES, &VESTA];

/// The index in [`ALL`] of the body being viewed.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The body being viewed. Earth unless `select` chose another.
pub fn current() -> &'static Body {
    ALL[CURRENT.load(Ordering::SeqCst)]
}

pub fn by_id(id: &str) -> Option<&'static Body> {
    ALL.iter().copied().find(|b| b.id == id.to_ascii_lowercase())
}

/// Every body that can be chosen, Earth first.
pub fn all() -> &'static [&'static Body] {
    &ALL
}

pub fn ids() -> Vec<&'static str> {
    ALL.iter().map(|b| b.id).collect()
}

/// Chooses the body to view. Whoever changes it while tiles are in use must drop the ones
/// made for the old body; `service_lib::invalidate` marks those still being made.
pub fn select(body: &'static Body) {
    let i = ALL.iter().position(|b| b.id == body.id).unwrap_or(0);
    CURRENT.store(i, Ordering::SeqCst);
}

/// The finest zoom level of the body's imagery.
pub fn max_zoom() -> u8 {
    if current().surface_data {
        sources::ABSOLUTE_MAX_Z
    } else {
        current().imagery.max_z
    }
}

/// A distance or size meant for Earth, scaled to the body in view. Exact for Earth.
pub fn scaled(metres_on_earth: f64) -> f64 {
    metres_on_earth * current().a / EARTH.a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earth_is_the_default_and_wgs84() {
        assert_eq!(current().id, "earth");
        assert_eq!(scaled(2.2e7), 2.2e7);
        let b = &EARTH;
        let e2 = 1.0 - (b.b / b.a).powi(2);
        assert!((e2 - b.e2).abs() < 1e-12);
    }

    #[test]
    fn bodies_are_found_by_name() {
        assert!(by_id("Earth").is_some());
        assert!(by_id("pluto").is_none());
        for b in ALL {
            assert!(b.a >= b.b && b.b > 0.0);
        }
    }

    #[test]
    fn the_new_bodies_have_published_radii_and_their_own_imagery() {
        for (id, a, b) in [("mars", 3_396_190.0, 3_376_200.0), ("mercury", 2_439_400.0, 2_439_400.0), ("ceres", 487_300.0, 446_000.0)] {
            let body = by_id(id).unwrap();
            assert_eq!((body.a, body.b), (a, b));
            assert!((body.e2 - (1.0 - (b / a) * (b / a))).abs() < 1e-12);
        }
        let vesta = by_id("vesta").unwrap();
        assert!(vesta.a > 280_000.0 && vesta.a < 289_000.0 && vesta.b == 229_000.0);
        let mut caches = std::collections::HashSet::new();
        for b in ALL {
            if !b.surface_data {
                assert!(!b.credit.is_empty() && b.ground.is_some());
                assert_eq!(b.imagery.grid, Grid::Equirect);
                assert!(caches.insert(b.imagery.cache), "{}", b.id);
                assert!(sources::by_id(b.imagery.id).is_some());
            }
        }
    }

    #[test]
    fn the_sun_and_the_sky_follow_each_body() {
        let t = 1_800_000_000.0;
        for b in ALL {
            let sun = (b.sun)(t);
            assert!((sun.length() - 1.0).abs() < 1e-9, "{}", b.id);
            let names: Vec<_> = (b.sky_bodies)(t).iter().map(|p| p.name).collect();
            assert!(!names.contains(&b.name) || b.id == "moon", "{}: {names:?}", b.id);
            assert!(names.len() >= 5, "{}: {names:?}", b.id);
        }
    }
}
