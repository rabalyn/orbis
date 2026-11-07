//! The bodies that can be viewed. Each is an immutable table entry: its shape, how its
//! imagery is tiled, what data exists for it, and how it turns. The body is chosen once at
//! startup (`--body`), and the geodesy, tile addressing and sky follow it.

use crate::geo::Grid;
use crate::sources::{self, TileSource};
use glam::{DMat3, DVec3};
use std::sync::OnceLock;

pub struct Body {
    pub id: &'static str,
    #[allow(dead_code)]
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
};

static ALL: [&Body; 1] = [&EARTH];

static CURRENT: OnceLock<&'static Body> = OnceLock::new();

/// The body being viewed. Earth unless `select` chose another before the first use.
pub fn current() -> &'static Body {
    CURRENT.get().copied().unwrap_or(&EARTH)
}

pub fn by_id(id: &str) -> Option<&'static Body> {
    ALL.iter().copied().find(|b| b.id == id.to_ascii_lowercase())
}

pub fn ids() -> Vec<&'static str> {
    ALL.iter().map(|b| b.id).collect()
}

/// Chooses the body for the whole run. Only the first call has an effect.
pub fn select(body: &'static Body) {
    let _ = CURRENT.set(body);
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
}
