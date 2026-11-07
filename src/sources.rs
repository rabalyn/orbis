//! The public data services, and which of them can provide a given tile.

use crate::geo::TileKey;

/// Half the width of the Web Mercator square, in metres.
const MERCATOR_EDGE: f64 = 20_037_508.342_789_244;
/// Tiles this far beyond the base imagery still improve terrain and map lines.
const BASE_TILE_MAX_Z: u8 = 16;
/// National imagery starts here, just past the base imagery's real resolution.
const DETAIL_MIN_Z: u8 = 15;
pub const ABSOLUTE_MAX_Z: u8 = 19;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    Imagery,
    Terrain,
    Map,
}

#[derive(Debug, PartialEq)]
pub struct TileSource {
    /// Used in cache server URLs.
    pub id: &'static str,
    pub kind: Kind,
    /// With `{z}`, `{x}`, `{y}` or `{bbox}`. Empty when the URL must be looked up first.
    pub url: &'static str,
    /// Where tiles are stored, relative to the cache.
    pub cache: &'static str,
    pub content_type: &'static str,
    pub min_z: u8,
    pub max_z: u8,
    /// Approximate coverage as boxes of west, south, east, north in degrees; none means the
    /// world. They hug each area fairly closely, because a tile outside it costs a pointless
    /// request to that service.
    pub bounds: &'static [[f64; 4]],
    pub credit: &'static str,
    /// The most requests at once. Services rendering each image on demand get fewer.
    pub concurrency: usize,
}

const JPEG: &str = "image/jpeg";

const fn detail(
    id: &'static str,
    url: &'static str,
    cache: &'static str,
    max_z: u8,
    bounds: &'static [[f64; 4]],
    credit: &'static str,
    concurrency: usize,
) -> TileSource {
    TileSource {
        id,
        kind: Kind::Imagery,
        url,
        cache,
        content_type: JPEG,
        min_z: DETAIL_MIN_Z,
        max_z,
        bounds,
        credit,
        concurrency,
    }
}

pub static EOX: TileSource = TileSource {
    id: "eox",
    kind: Kind::Imagery,
    url: "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2024_3857/default/g/{z}/{y}/{x}.jpg",
    cache: "eox-s2cloudless-2024/{z}/{y}/{x}.jpg",
    content_type: JPEG,
    min_z: 0,
    max_z: 14,
    bounds: &[],
    credit: "",
    concurrency: 4,
};

pub static TERRAIN: TileSource = TileSource {
    id: "mapterhorn",
    kind: Kind::Terrain,
    url: "https://tiles.mapterhorn.com/{z}/{x}/{y}.webp",
    cache: "mapterhorn/{z}/{x}/{y}.webp",
    content_type: "image/webp",
    min_z: 0,
    max_z: 14,
    bounds: &[],
    credit: "",
    concurrency: 4,
};

pub static MAP: TileSource = TileSource {
    id: "openfreemap",
    kind: Kind::Map,
    url: "",
    cache: "openfreemap/{z}/{x}/{y}.pbf",
    content_type: "application/vnd.mapbox-vector-tile",
    min_z: 0,
    max_z: 14,
    bounds: &[],
    credit: "",
    concurrency: 4,
};

/// Host used to look up the map service's current URL.
pub const MAP_TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";

macro_rules! wms {
    ($base:literal, $layer:literal) => {
        concat!(
            $base,
            "?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=",
            $layer,
            "&STYLES=&CRS=EPSG:3857&BBOX={bbox}&WIDTH=256&HEIGHT=256&FORMAT=image/jpeg"
        )
    };
}

/// Detail imagery, best first. Smaller regions come before the larger ones around them.
pub static DETAIL: [TileSource; 8] = [
    detail(
        "berlin",
        wms!("https://gdi.berlin.de/services/wms/truedop_2024", "truedop_2024"),
        "berlin/{z}/{x}/{y}.jpg",
        19,
        &[[13.05, 52.33, 13.78, 52.68]],
        "Geoportal Berlin",
        2,
    ),
    detail(
        "nrw",
        wms!("https://www.wms.nrw.de/geobasis/wms_nw_dop", "nw_dop_rgb"),
        "nrw/{z}/{x}/{y}.jpg",
        19,
        &[[5.8, 50.3, 9.5, 52.6]],
        "Geobasis NRW",
        2,
    ),
    detail(
        "bavaria",
        wms!("https://geoservices.bayern.de/od/wms/dop/v1/dop40", "by_dop40c"),
        "bavaria/{z}/{x}/{y}.jpg",
        18,
        &[[9.9, 47.55, 13.0, 50.6], [13.0, 47.95, 13.85, 49.5], [8.9, 49.5, 9.9, 50.6], [10.1, 47.25, 10.9, 47.6], [10.9, 47.4, 13.1, 47.6]],
        "Bayerische Vermessungsverwaltung",
        2,
    ),
    detail(
        "basemap-at",
        "https://mapsneu.wien.gv.at/basemap/bmaporthofoto30cm/normal/google3857/{z}/{y}/{x}.jpeg",
        "basemap-at/{z}/{x}/{y}.jpg",
        19,
        &[[9.5, 46.4, 13.0, 47.75], [12.1, 46.4, 13.8, 47.8], [13.8, 47.0, 17.2, 49.02], [13.0, 47.6, 13.8, 48.8]],
        "Grundkarte: basemap.at",
        4,
    ),
    detail(
        "swisstopo",
        "https://wmts.geo.admin.ch/1.0.0/ch.swisstopo.swissimage/default/current/3857/{z}/{x}/{y}.jpeg",
        "swisstopo/{z}/{x}/{y}.jpg",
        19,
        &[[5.9, 45.8, 10.5, 47.81]],
        "© swisstopo",
        4,
    ),
    detail(
        "ign",
        "https://data.geopf.fr/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=ORTHOIMAGERY.ORTHOPHOTOS&STYLE=normal&FORMAT=image/jpeg&TILEMATRIXSET=PM&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}",
        "ign/{z}/{x}/{y}.jpg",
        19,
        &[[-5.2, 42.3, 6.8, 50.0], [-2.0, 50.0, 4.3, 51.1], [6.8, 47.4, 8.3, 49.1], [8.5, 41.3, 9.6, 43.1]],
        "© IGN",
        4,
    ),
    detail(
        "pdok",
        "https://service.pdok.nl/hwh/luchtfotorgb/wmts/v1_0/Actueel_orthoHR/EPSG:3857/{z}/{x}/{y}.jpeg",
        "pdok/{z}/{x}/{y}.jpg",
        19,
        &[[3.2, 50.7, 7.3, 53.6]],
        "PDOK Luchtfoto",
        4,
    ),
    detail(
        "usgs",
        "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}",
        "usgs/{z}/{x}/{y}.jpg",
        16,
        &[[-125.0, 24.0, -66.9, 49.5]],
        "USGS The National Map",
        4,
    ),
];

impl TileSource {
    fn covers(&self, k: TileKey) -> bool {
        if self.bounds.is_empty() {
            return true;
        }
        let (lat_n, lon_w) = k.lat_lon(0.0, 0.0);
        let (lat_s, lon_e) = k.lat_lon(1.0, 1.0);
        self.bounds.iter().any(|[west, south, east, north]| {
            lon_e.to_degrees() > *west
                && lon_w.to_degrees() < *east
                && lat_n.to_degrees() > *south
                && lat_s.to_degrees() < *north
        })
    }

    pub fn cache_name(&self, k: TileKey) -> String {
        fill(self.cache, k)
    }

    /// A tile's URL, from this source's own template or one looked up for it.
    pub fn url_from(template: &str, k: TileKey) -> String {
        let url = fill(template, k);
        if url.contains("{bbox}") {
            url.replace("{bbox}", &mercator_bbox(k))
        } else {
            url
        }
    }

    /// Whether a blank answer means "no data here", rather than a tile of plain ocean.
    pub fn blank_means_missing(&self) -> bool {
        !self.bounds.is_empty()
    }
}

fn fill(template: &str, k: TileKey) -> String {
    template
        .replace("{z}", &k.z.to_string())
        .replace("{x}", &k.x.to_string())
        .replace("{y}", &k.y.to_string())
}

fn mercator_bbox(k: TileKey) -> String {
    let size = 2.0 * MERCATOR_EDGE / (1u64 << k.z) as f64;
    let west = -MERCATOR_EDGE + k.x as f64 * size;
    let north = MERCATOR_EDGE - k.y as f64 * size;
    format!("{:.3},{:.3},{:.3},{:.3}", west, north - size, west + size, north)
}

/// All sources a cache server can serve.
pub fn all() -> impl Iterator<Item = &'static TileSource> {
    [&EOX, &TERRAIN, &MAP].into_iter().chain(DETAIL.iter())
}

pub fn by_id(id: &str) -> Option<&'static TileSource> {
    all().find(|s| s.id == id)
}

/// Which imagery could serve a tile, best first, with the source tile to ask for. A tile
/// beyond a source's detail takes a part of a coarser image.
pub fn imagery_for(k: TileKey) -> Vec<(&'static TileSource, TileKey)> {
    let mut out: Vec<_> = DETAIL
        .iter()
        .filter(|s| k.z >= s.min_z && s.covers(k))
        .map(|s| (s, k.ancestor(k.z.min(s.max_z))))
        .collect();
    out.push((&EOX, k.ancestor(k.z.min(EOX.max_z))));
    out
}

/// How far a tile may be split at this place, given the imagery available there.
pub fn max_tile_z(k: TileKey) -> u8 {
    if k.z < BASE_TILE_MAX_Z {
        return BASE_TILE_MAX_Z;
    }
    DETAIL
        .iter()
        .filter(|s| s.covers(k))
        .map(|s| s.max_z)
        .max()
        .unwrap_or(BASE_TILE_MAX_Z)
        .max(BASE_TILE_MAX_Z)
}

#[cfg(test)]
fn key_at(lat: f64, lon: f64, z: u8) -> TileKey {
    TileKey::containing(lat.to_radians(), lon.to_radians(), z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(k: TileKey) -> Vec<&'static str> {
        imagery_for(k).iter().map(|s| s.0.id).collect()
    }

    #[test]
    fn far_views_use_only_the_base() {
        assert_eq!(ids(key_at(47.27, 11.4, 10)), ["eox"]);
        assert_eq!(ids(key_at(47.27, 11.4, 14)), ["eox"]);
    }

    #[test]
    fn close_views_prefer_detail_with_the_base_as_fallback() {
        assert_eq!(ids(key_at(47.27, 11.4, 16)), ["basemap-at", "eox"]);
        assert_eq!(ids(key_at(48.14, 11.58, 16)), ["bavaria", "eox"]);
        assert_eq!(ids(key_at(46.02, 7.75, 16)), ["swisstopo", "eox"]);
        assert_eq!(ids(key_at(48.86, 2.35, 16)), ["ign", "eox"]);
        assert_eq!(ids(key_at(52.37, 4.90, 16)), ["pdok", "eox"]);
        assert_eq!(ids(key_at(51.23, 6.78, 16)), ["nrw", "pdok", "eox"]);
        assert_eq!(ids(key_at(52.52, 13.40, 16)), ["berlin", "eox"]);
        assert_eq!(ids(key_at(35.68, 139.76, 16)), ["eox"]);
        assert_eq!(ids(key_at(40.75, -73.98, 16)), ["usgs", "eox"]);
    }

    #[test]
    fn well_known_places_do_not_trouble_their_neighbours_services() {
        // Whichever is first gets asked for every tile, so a wrong one costs a useless request
        // each time. A neighbour further down the list is asked only if the first has nothing.
        for (name, lat, lon, expected) in [
            ("Innsbruck", 47.27, 11.40, "basemap-at"),
            ("Salzburg", 47.80, 13.04, "basemap-at"),
            ("Vienna", 48.21, 16.37, "basemap-at"),
            ("Munich", 48.14, 11.58, "bavaria"),
            ("Nuremberg", 49.45, 11.08, "bavaria"),
            ("Garmisch", 47.49, 11.10, "bavaria"),
            ("Zurich", 47.37, 8.54, "swisstopo"),
            ("Strasbourg", 48.57, 7.75, "ign"),
            ("Lille", 50.63, 3.06, "ign"),
        ] {
            assert_eq!(ids(key_at(lat, lon, 18))[0], expected, "{name}");
        }
        for (name, lat, lon) in [("Frankfurt", 50.11, 8.68), ("Hamburg", 53.55, 9.99), ("Cologne south", 49.9, 7.0)] {
            assert_eq!(ids(key_at(lat, lon, 18)), ["eox"], "{name} has no service yet");
        }
    }

    #[test]
    fn beyond_a_sources_detail_a_part_of_its_finest_image_is_used() {
        let k = key_at(40.75, -73.98, 19);
        let found = imagery_for(k);
        assert_eq!(found[0].1, k.ancestor(16));
        assert_eq!(found.last().unwrap().1, k.ancestor(14));
    }

    #[test]
    fn tiles_split_further_only_where_detail_exists() {
        assert_eq!(max_tile_z(key_at(47.27, 11.4, 16)), 19);
        assert_eq!(max_tile_z(key_at(48.14, 11.58, 16)), 18);
        assert_eq!(max_tile_z(key_at(35.68, 139.76, 16)), 16);
        assert_eq!(max_tile_z(key_at(40.75, -73.98, 16)), 16);
        assert_eq!(max_tile_z(key_at(35.68, 139.76, 3)), 16);
        for s in all() {
            assert!(s.max_z <= ABSOLUTE_MAX_Z);
        }
    }

    #[test]
    fn urls_follow_each_services_scheme() {
        let k = TileKey::new(18, 139_000, 91_000);
        assert_eq!(
            TileSource::url_from(TERRAIN.url, k),
            "https://tiles.mapterhorn.com/18/139000/91000.webp"
        );
        assert!(TileSource::url_from(EOX.url, k).ends_with("/g/18/91000/139000.jpg"));
        let ign = by_id("ign").unwrap();
        assert!(TileSource::url_from(ign.url, k).contains("TILEMATRIX=18&TILEROW=91000&TILECOL=139000"));
    }

    #[test]
    fn wms_requests_use_the_tile_as_their_box() {
        // The whole world, then its north-east quarter.
        assert_eq!(
            mercator_bbox(TileKey::new(0, 0, 0)),
            "-20037508.343,-20037508.343,20037508.343,20037508.343"
        );
        assert_eq!(
            mercator_bbox(TileKey::new(1, 1, 0)),
            "0.000,0.000,20037508.343,20037508.343"
        );
        let url = TileSource::url_from(by_id("bavaria").unwrap().url, TileKey::new(1, 1, 0));
        assert!(url.contains("BBOX=0.000,0.000,20037508.343,20037508.343&WIDTH=256"));
    }

    #[test]
    fn identifiers_and_cache_folders_are_unique() {
        let sources: Vec<_> = all().collect();
        for (i, a) in sources.iter().enumerate() {
            for b in &sources[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.cache, b.cache);
            }
            // Identifiers appear in URLs.
            assert!(a.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
            assert_eq!(by_id(a.id), Some(*a));
        }
        assert_eq!(by_id("../../etc"), None);
    }

    #[test]
    fn existing_cached_base_imagery_stays_valid() {
        let k = TileKey::new(3, 2, 1);
        assert_eq!(EOX.cache_name(k), "eox-s2cloudless-2024/3/1/2.jpg");
    }

    #[test]
    fn every_detail_source_credits_its_data() {
        for s in &DETAIL {
            assert!(!s.credit.is_empty(), "{}", s.id);
            assert!(s.blank_means_missing());
        }
        assert!(!EOX.blank_means_missing());
    }
}
