//! The public data services, and which of them can provide a given tile.

use crate::geo::{Grid, TileKey};

/// Half the width of the Web Mercator square, in metres.
const MERCATOR_EDGE: f64 = 20_037_508.342_789_244;
/// Tiles this far beyond the base imagery still improve terrain and map lines.
const BASE_TILE_MAX_Z: u8 = 16;
/// National imagery starts here, two levels before the base imagery reaches its real resolution,
/// so that the change from Sentinel-2 to aerial photos, which differ in colour, lies far out in a
/// tilted view rather than across the middle of it. All detail services serve these levels.
const DETAIL_MIN_Z: u8 = 13;
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
    /// The width of its images. A 512-pixel image holds the detail of four 256-pixel ones.
    pub pixels: u32,
    /// How its tiles are laid out over the body.
    pub grid: Grid,
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
        pixels: 256,
        grid: Grid::Mercator,
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
    pixels: 256,
    grid: Grid::Mercator,
};

/// NASA Moon Trek's WMTS layer of the LRO Wide Angle Camera global mosaic (100 m per pixel),
/// read from its capabilities document: 256-pixel JPEG tiles, an equirectangular matrix set
/// with 2 by 1 tiles at level 0 and levels 0 to 8, laid out as {z}/{row}/{column}.
pub static MOON_WAC: TileSource = TileSource {
    id: "moon-wac",
    kind: Kind::Imagery,
    url: "https://trek.nasa.gov/tiles/Moon/EQ/LRO_WAC_Mosaic_Global_303ppd_v02/1.0.0/default/default028mm/{z}/{y}/{x}.jpg",
    cache: "nasa-trek-moon-wac/{z}/{y}/{x}.jpg",
    content_type: JPEG,
    min_z: 0,
    max_z: 8,
    bounds: &[],
    credit: "",
    concurrency: 2,
    pixels: 256,
    grid: Grid::Equirect,
};

/// NASA Mars Trek's layer of the Viking MDIM 2.1 colour mosaic (232 m per pixel), read from its capabilities document: 256-pixel JPEG tiles, plate carrée matrix set with 2 by 1 tiles at level 0 and levels 0 to 7.
pub static MARS_VIKING: TileSource = TileSource {
    id: "mars-viking",
    kind: Kind::Imagery,
    url: "https://trek.nasa.gov/tiles/Mars/EQ/Mars_Viking_MDIM21_ClrMosaic_global_232m/1.0.0/default/default028mm/{z}/{y}/{x}.jpg",
    cache: "nasa-trek-mars-viking/{z}/{y}/{x}.jpg",
    content_type: JPEG,
    min_z: 0,
    max_z: 7,
    bounds: &[],
    credit: "",
    concurrency: 2,
    pixels: 256,
    grid: Grid::Equirect,
};

/// NASA Mercury Trek's layer of the MESSENGER MDIS basemap (166 m per pixel), levels 0 to 7 of the same plate carrée matrix set.
pub static MERCURY_MDIS: TileSource = TileSource {
    id: "mercury-mdis",
    kind: Kind::Imagery,
    url: "https://trek.nasa.gov/tiles/Mercury/EQ/Mercury_MESSENGER_MDIS_Basemap_BDR_Mosaic_Global_166m/1.0.0/default/default028mm/{z}/{y}/{x}.jpg",
    cache: "nasa-trek-mercury-mdis/{z}/{y}/{x}.jpg",
    content_type: JPEG,
    min_z: 0,
    max_z: 7,
    bounds: &[],
    credit: "",
    concurrency: 2,
    pixels: 256,
    grid: Grid::Equirect,
};

/// NASA Ceres Trek's layer of the Dawn Framing Camera HAMO colour-shaded mosaic (60 pixels per degree), levels 0 to 5.
pub static CERES_FC: TileSource = TileSource {
    id: "ceres-fc",
    kind: Kind::Imagery,
    url: "https://trek.nasa.gov/tiles/Ceres/EQ/Ceres_Dawn_FC_HAMO_ClrShade_DLR_Global_60ppd_Oct2016/1.0.0/default/default028mm/{z}/{y}/{x}.jpg",
    cache: "nasa-trek-ceres-fc/{z}/{y}/{x}.jpg",
    content_type: JPEG,
    min_z: 0,
    max_z: 5,
    bounds: &[],
    credit: "",
    concurrency: 2,
    pixels: 256,
    grid: Grid::Equirect,
};

/// NASA Vesta Trek's layer of the Dawn Framing Camera HAMO mosaic (74 pixels per degree), levels 0 to 6.
pub static VESTA_FC: TileSource = TileSource {
    id: "vesta-fc",
    kind: Kind::Imagery,
    url: "https://trek.nasa.gov/tiles/Vesta/EQ/Vesta_Dawn_FC_HAMO_Mosaic_Global_74ppd/1.0.0/default/default028mm/{z}/{y}/{x}.jpg",
    cache: "nasa-trek-vesta-fc/{z}/{y}/{x}.jpg",
    content_type: JPEG,
    min_z: 0,
    max_z: 6,
    bounds: &[],
    credit: "",
    concurrency: 2,
    pixels: 256,
    grid: Grid::Equirect,
};

const PNG: &str = "image/png";

/// A finer layer of NASA Trek over part or all of a body, read from its capabilities document
/// (`.../<layer>/1.0.0/WMTSCapabilities.xml`): 256-pixel PNG tiles in the same plate carrée
/// matrix set as the base layers. Its deepest level was found by asking for tiles.
const fn trek(
    id: &'static str,
    url: &'static str,
    cache: &'static str,
    min_z: u8,
    max_z: u8,
    bounds: &'static [[f64; 4]],
) -> TileSource {
    TileSource {
        id,
        kind: Kind::Imagery,
        url,
        cache,
        content_type: PNG,
        min_z,
        max_z,
        bounds,
        credit: "",
        concurrency: 2,
        pixels: 256,
        grid: Grid::Equirect,
    }
}

/// The Kaguya (SELENE) Terrain Camera morning orthomosaic, levels 0 to 10 on Moon Trek, about
/// 20 m per pixel at level 10: four times finer than the WAC mosaic. Grey with transparency
/// where it has no data, where the WAC mosaic shows through. From level 9, where it starts to
/// show more than the WAC mosaic; its lighting varies more, so the WAC mosaic stays the base.
pub static MOON_KAGUYA: TileSource = trek(
    "moon-kaguya",
    "https://trek.nasa.gov/tiles/Moon/EQ/Kaguya_TCortho_Mosaic_Global_4096ppd/1.0.0/default/default028mm/{z}/{y}/{x}.png",
    "nasa-trek-moon-kaguya/{z}/{y}/{x}.png",
    9,
    10,
    &[],
);

/// LRO Narrow Angle Camera mosaics of the Apollo landing sites, about 0.5 m per pixel, where the
/// astronauts' footpaths and the rovers' tracks can be seen. Boxes from Moon Trek's catalogue.
pub static MOON_NAC: [TileSource; 6] = [
    trek(
        "moon-nac-tranquility",
        "https://trek.nasa.gov/tiles/Moon/EQ/LRO_NAC_Apollo11_Mosaic_p/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-moon-nac-apollo11/{z}/{y}/{x}.png",
        11,
        15,
        &[[23.4485, 0.1465, 23.5397, 1.1149]],
    ),
    trek(
        "moon-nac-surveyor",
        "https://trek.nasa.gov/tiles/Moon/EQ/LRO_NAC_Apollo12_Mosaic_p/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-moon-nac-apollo12/{z}/{y}/{x}.png",
        11,
        16,
        &[[-23.4443, -3.4714, -23.3571, -2.5018]],
    ),
    trek(
        "moon-nac-fra-mauro",
        "https://trek.nasa.gov/tiles/Moon/EQ/LRO_NAC_Apollo14_Mosaic_p/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-moon-nac-apollo14/{z}/{y}/{x}.png",
        11,
        15,
        &[[-17.4901, -4.1921, -17.3907, -3.2253]],
    ),
    trek(
        "moon-nac-hadley",
        "https://trek.nasa.gov/tiles/Moon/EQ/LRO_NAC_Mosaic_26N004E_50cmp/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-moon-nac-26n004e/{z}/{y}/{x}.png",
        11,
        14,
        &[[2.9308, 25.4463, 4.3991, 26.7661]],
    ),
    trek(
        "moon-nac-descartes",
        "https://trek.nasa.gov/tiles/Moon/EQ/LRO_NAC_Mosaic_09S015E_50cmp/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-moon-nac-09s015e/{z}/{y}/{x}.png",
        11,
        15,
        &[[14.8050, -9.6332, 16.1413, -8.3134]],
    ),
    trek(
        "moon-nac-taurus-littrow",
        "https://trek.nasa.gov/tiles/Moon/EQ/NAC_DTM_APOLLO17_MOSAIC_120CM/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-moon-nac-apollo17/{z}/{y}/{x}.png",
        11,
        14,
        &[[29.9059, 19.3905, 31.6580, 21.3035]],
    ),
];

/// The Murray Lab's global CTX mosaic (beta01, uncontrolled, about 5 m per pixel), as Esri
/// serves it for Mars Trek: 512-pixel grey PNG tiles in a plate carrée grid of 2 by 1 at level 0,
/// levels 0 to 12, laid out as {z}/{row}/{column} (the service's own description). From level 8,
/// where it is finer than the Viking colour mosaic.
pub static MARS_CTX: TileSource = TileSource {
    id: "mars-ctx",
    kind: Kind::Imagery,
    url: "https://astro.arcgis.com/arcgis/rest/services/OnMars/CTX/MapServer/tile/{z}/{y}/{x}",
    cache: "mars-ctx-beta01/{z}/{y}/{x}.png",
    content_type: PNG,
    min_z: 8,
    max_z: 12,
    bounds: &[],
    credit: "",
    concurrency: 2,
    pixels: 512,
    grid: Grid::Equirect,
};

/// HiRISE mosaics of the rover landing sites on Mars Trek, 25 to 50 cm per pixel, from level
/// 13, where they are finer than CTX. The rovers' tracks can be seen in places.
pub static MARS_HIRISE: [TileSource; 4] = [
    trek(
        "mars-hirise-curiosity",
        "https://trek.nasa.gov/tiles/Mars/EQ/curiosity_hirise_mosaic/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-mars-hirise-curiosity/{z}/{y}/{x}.png",
        13,
        16,
        &[[137.1225, -4.9255, 137.7298, -4.2489]],
    ),
    trek(
        "mars-hirise-opportunity",
        "https://trek.nasa.gov/tiles/Mars/EQ/opportunity_hirise_mosaic/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-mars-hirise-opportunity/{z}/{y}/{x}.png",
        13,
        17,
        &[[-5.5965, -2.7514, -4.9954, -1.8994]],
    ),
    trek(
        "mars-hirise-spirit",
        "https://trek.nasa.gov/tiles/Mars/EQ/spirit_hirise_mosaic/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-mars-hirise-spirit/{z}/{y}/{x}.png",
        13,
        17,
        &[[175.4278, -14.6926, 175.5859, -14.5357]],
    ),
    trek(
        "mars-hirise-sojourner",
        "https://trek.nasa.gov/tiles/Mars/EQ/sojourner_hirise_mosaic/1.0.0/default/default028mm/{z}/{y}/{x}.png",
        "nasa-trek-mars-hirise-sojourner/{z}/{y}/{x}.png",
        13,
        16,
        &[[-33.2880, 19.0227, -33.2087, 19.1599]],
    ),
];

/// Dawn's LAMO mosaic of Ceres on Ceres Trek, grey, levels 0 to 7 (about 35 m per pixel at
/// level 7): from level 6, beyond the colour-shaded HAMO base.
pub static CERES_LAMO: TileSource = TileSource {
    content_type: JPEG,
    ..trek(
        "ceres-lamo",
        "https://trek.nasa.gov/tiles/Ceres/EQ/CE_LAMO_G_00N_180E_EQU_CLR_scale.eq/1.0.0/default/default028mm/{z}/{y}/{x}.jpg",
        "nasa-trek-ceres-lamo/{z}/{y}/{x}.jpg",
        6,
        7,
        &[],
    )
};

/// Vesta Trek's blend of Dawn's HAMO and LAMO mosaics, levels 0 to 7: level 7, one beyond the
/// HAMO base.
pub static VESTA_LAMO: TileSource = trek(
    "vesta-lamo",
    "https://trek.nasa.gov/tiles/Vesta/EQ/VestaHAMOLAMOBlend.eq/1.0.0/default/default028mm/{z}/{y}/{x}.png",
    "nasa-trek-vesta-lamo/{z}/{y}/{x}.png",
    7,
    7,
    &[],
);

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
    pixels: 512,
    grid: Grid::Mercator,
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
    pixels: 256,
    grid: Grid::Mercator,
};

/// Host used to look up the map service's current URL.
pub const MAP_TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";

macro_rules! wms {
    ($base:literal, $layer:literal) => {
        wms!($base, $layer, "256")
    };
    ($base:literal, $layer:literal, $size:literal) => {
        concat!(
            $base,
            "?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=",
            $layer,
            "&STYLES=&CRS=EPSG:3857&BBOX={bbox}&WIDTH=",
            $size,
            "&HEIGHT=",
            $size,
            "&FORMAT=image/jpeg"
        )
    };
}

/// Detail imagery, best first. Smaller regions come before the larger ones around them.
pub static DETAIL: [TileSource; 11] = [
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
        &[[5.8, 50.3, 7.8, 52.35], [7.8, 50.7, 8.3, 52.1], [8.3, 50.7, 9.0, 52.55], [9.0, 51.4, 9.5, 52.0]],
        "Geobasis NRW",
        2,
    ),
    // Each image takes this service about a second whatever its size: 512 pixels took 10% longer
    // than 256 when measured, so one request for four tiles is kinder to it and much faster.
    TileSource {
        pixels: 512,
        ..detail(
            "lgln",
            wms!("https://opendata.lgln.niedersachsen.de/doorman/noauth/dop_wms", "ni_dop20", "512"),
            "lgln/{z}/{x}/{y}.jpg",
            18,
            &[
                [6.6, 52.24, 8.0, 53.8],
                [8.0, 52.1, 10.7, 53.4],
                [9.0, 51.35, 10.7, 52.1],
                [8.0, 53.4, 9.7, 54.0],
                [10.7, 52.1, 11.1, 52.9],
                [10.9, 52.9, 11.7, 53.15],
            ],
            // The service asks for the year of retrieval; update it with each release.
            "LGLN (2026) CC BY 4.0",
            2,
        )
    },
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
        &[
            [-1.9, 43.35, 6.8, 50.0],
            [-5.2, 46.0, -1.9, 50.0],
            [-1.9, 43.0, -0.5, 43.35],
            [-0.5, 42.65, 1.7, 43.35],
            [1.7, 42.3, 6.8, 43.35],
            [-2.0, 50.0, 4.3, 51.1],
            [6.8, 47.4, 8.3, 49.1],
            [8.5, 41.3, 9.6, 43.1],
        ],
        "© IGN",
        4,
    ),
    detail(
        "pnoa",
        wms!("https://www.ign.es/wms-inspire/pnoa-ma", "OI.OrthoimageCoverage"),
        "pnoa/{z}/{x}/{y}.jpg",
        19,
        &[
            [-9.4, 42.15, 3.4, 43.85],
            [-6.2, 36.0, 3.4, 42.15],
            [-7.4, 36.9, -6.2, 40.0],
            [1.1, 38.6, 4.4, 40.1],
            [-18.2, 27.6, -13.4, 29.5],
        ],
        "PNOA CC BY 4.0 scne.es",
        2,
    ),
    detail(
        "pdok",
        "https://service.pdok.nl/hwh/luchtfotorgb/wmts/v1_0/Actueel_orthoHR/EPSG:3857/{z}/{x}/{y}.jpeg",
        "pdok/{z}/{x}/{y}.jpg",
        19,
        &[[3.2, 51.2, 7.3, 53.6], [5.6, 50.7, 6.3, 51.2]],
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
    // Last among the detail sources: it combines national services, so anything nearer the
    // original comes first and this one fills the places without their own entry above.
    TileSource {
        id: "versatiles",
        kind: Kind::Imagery,
        url: "https://tiles.versatiles.org/tiles/satellite/{z}/{x}/{y}",
        cache: "versatiles/{z}/{x}/{y}.webp",
        content_type: "image/webp",
        min_z: DETAIL_MIN_Z,
        // It advertises 19, but sampled tiles across Europe all ended at 17.
        max_z: 17,
        bounds: &[
            [-1.9, 43.35, 8.3, 51.1],     // France
            [-5.2, 46.0, -1.9, 49.0],     // Brittany
            [8.5, 41.3, 9.6, 43.1],       // Corsica
            [5.8, 45.8, 15.1, 55.1],      // Germany, Switzerland, western Austria
            [8.0, 54.5, 15.2, 57.8],      // Denmark
            [2.5, 49.4, 7.3, 53.6],       // Benelux
            [12.0, 46.4, 17.2, 49.1],     // eastern Austria
            [12.0, 48.5, 18.9, 51.1],     // Czechia
            [16.1, 45.7, 22.9, 49.7],     // Hungary, Slovakia
            [20.2, 43.6, 29.8, 48.3],     // Romania
            [13.4, 42.3, 19.5, 46.6],     // Croatia
            [18.4, 39.6, 21.1, 43.6],     // Montenegro, Albania
            [20.9, 53.9, 28.3, 59.7],     // Baltic states
            [-9.6, 36.9, -6.1, 42.2],     // Portugal
            [32.2, 34.5, 34.7, 35.75],    // Cyprus
            [14.1, 35.8, 14.6, 36.1],     // Malta
        ],
        credit: "VersaTiles sources https://versatiles.org/sources/",
        // One project's server, so it gets the gentle limit of the on-demand services.
        concurrency: 2,
        pixels: 512,
        grid: Grid::Mercator,
    },
];

impl TileSource {
    pub fn covers(&self, k: TileKey) -> bool {
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

    /// Which of this source's images holds a tile. Its images may be coarser than the tile.
    pub fn source_key(&self, k: TileKey) -> TileKey {
        let same_detail = if self.pixels > 256 {
            k.z.saturating_sub(1)
        } else {
            k.z
        };
        k.ancestor(same_detail.min(self.max_z))
    }

    /// The finest tile that still gets new detail from this source.
    pub fn detail_z(&self) -> u8 {
        if self.pixels > 256 {
            self.max_z + 1
        } else {
            self.max_z
        }
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
        !self.bounds.is_empty() && !crate::service_lib::is_service(self)
    }
}

fn fill(template: &str, k: TileKey) -> String {
    template
        .replace("{z}", &k.z.to_string())
        .replace("{z02}", &format!("{:02}", k.z))
        .replace("{x}", &k.x.to_string())
        .replace("{y}", &k.y.to_string())
        // TMS counts rows from the south.
        .replace("{-y}", &((1u64 << k.z) - 1 - k.y as u64).to_string())
}

fn mercator_bbox(k: TileKey) -> String {
    let size = 2.0 * MERCATOR_EDGE / (1u64 << k.z) as f64;
    let west = -MERCATOR_EDGE + k.x as f64 * size;
    let north = MERCATOR_EDGE - k.y as f64 * size;
    format!(
        "{:.3},{:.3},{:.3},{:.3}",
        west,
        north - size,
        west + size,
        north
    )
}

/// All sources a cache server can serve.
pub fn all() -> impl Iterator<Item = &'static TileSource> {
    [
        &EOX,
        &TERRAIN,
        &MAP,
        &MOON_WAC,
        &MARS_VIKING,
        &MERCURY_MDIS,
        &CERES_FC,
        &VESTA_FC,
    ]
    .into_iter()
    .chain(DETAIL.iter())
    .chain(
        crate::body::all()
            .iter()
            .flat_map(|b| b.detail.iter().copied()),
    )
}

pub fn by_id(id: &str) -> Option<&'static TileSource> {
    all().find(|s| s.id == id)
}

/// Which imagery could serve a tile, best first, with the source tile to ask for. A tile
/// beyond a source's detail takes a part of a coarser image.
pub fn imagery_for(k: TileKey) -> Vec<(&'static TileSource, TileKey)> {
    let body = crate::body::current();
    if !body.surface_data {
        // The finer layers first, each where it has pictures, then the base.
        let mut out: Vec<_> = body
            .detail
            .iter()
            .filter(|s| k.z >= s.min_z && s.covers(k))
            .map(|s| (*s, s.source_key(k)))
            .collect();
        out.push((body.imagery, body.imagery.source_key(k)));
        return out;
    }
    // A chosen map service replaces the imagery wherever it has tiles; elsewhere the usual
    // imagery stays, rather than leaving holes.
    if let Some(base) = serving(k) {
        return vec![(base, base.source_key(k))];
    }
    let mut out: Vec<_> = DETAIL
        .iter()
        .filter(|s| k.z >= s.min_z && s.covers(k))
        .map(|s| (s, s.source_key(k)))
        .collect();
    out.push((&EOX, EOX.source_key(k)));
    out
}

/// The chosen map service, if it has a picture for this tile.
fn serving(k: TileKey) -> Option<&'static TileSource> {
    crate::service_lib::active()
        .base
        .filter(|b| k.z >= b.min_z && b.covers(k))
}

/// How far a tile may be split at this place, given the imagery available there.
pub fn max_tile_z(k: TileKey) -> u8 {
    let body = crate::body::current();
    if !body.surface_data {
        return body
            .detail
            .iter()
            .filter(|s| s.covers(k))
            .map(|s| s.detail_z())
            .fold(body.imagery.detail_z(), u8::max);
    }
    if k.z < BASE_TILE_MAX_Z {
        return BASE_TILE_MAX_Z;
    }
    DETAIL
        .iter()
        .filter(|s| s.covers(k))
        .map(TileSource::detail_z)
        .chain(serving(k).map(TileSource::detail_z))
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
        assert_eq!(ids(key_at(47.27, 11.4, 12)), ["eox"]);
        assert_eq!(
            ids(key_at(47.27, 11.4, 13)),
            ["basemap-at", "versatiles", "eox"]
        );
    }

    #[test]
    fn close_views_prefer_detail_with_the_base_as_fallback() {
        assert_eq!(
            ids(key_at(47.27, 11.4, 16)),
            ["basemap-at", "versatiles", "eox"]
        );
        assert_eq!(
            ids(key_at(48.14, 11.58, 16)),
            ["bavaria", "versatiles", "eox"]
        );
        assert_eq!(
            ids(key_at(46.02, 7.75, 16)),
            ["swisstopo", "versatiles", "eox"]
        );
        assert_eq!(ids(key_at(48.86, 2.35, 16)), ["ign", "versatiles", "eox"]);
        assert_eq!(ids(key_at(52.37, 4.90, 16)), ["pdok", "versatiles", "eox"]);
        assert_eq!(
            ids(key_at(51.23, 6.78, 16)),
            ["nrw", "pdok", "versatiles", "eox"]
        );
        assert_eq!(
            ids(key_at(52.52, 13.40, 16)),
            ["berlin", "versatiles", "eox"]
        );
        assert_eq!(ids(key_at(40.42, -3.70, 16)), ["pnoa", "eox"]);
        assert_eq!(ids(key_at(50.11, 8.68, 16)), ["versatiles", "eox"]);
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
        for (name, lat, lon) in [
            ("Madrid", 40.42, -3.70),
            ("Barcelona", 41.40, 2.17),
            ("Bilbao", 43.26, -2.93),
            ("San Sebastian", 43.32, -1.98),
            ("Pamplona", 42.82, -1.64),
            ("Palma", 39.57, 2.65),
            ("Tenerife", 28.46, -16.25),
        ] {
            assert_eq!(ids(key_at(lat, lon, 18)), ["pnoa", "eox"], "{name}");
        }
        for (name, lat, lon, expected) in [
            ("Biarritz", 43.48, -1.56, "ign"),
            ("Perpignan", 42.70, 2.90, "ign"),
            ("Lisbon", 38.72, -9.14, "versatiles"),
            ("Porto", 41.15, -8.61, "versatiles"),
        ] {
            assert_eq!(ids(key_at(lat, lon, 18))[0], expected, "{name}");
        }
        for (name, lat, lon) in [
            ("Frankfurt", 50.11, 8.68),
            ("Hamburg", 53.55, 9.99),
            ("Dresden", 51.05, 13.74),
            ("Brussels", 50.85, 4.35),
            ("Copenhagen", 55.68, 12.57),
            ("Prague", 50.08, 14.43),
            ("Budapest", 47.50, 19.04),
            ("Bucharest", 44.43, 26.10),
            ("Zagreb", 45.81, 15.98),
            ("Tallinn", 59.44, 24.75),
            ("Riga", 56.95, 24.11),
            ("Vilnius", 54.69, 25.28),
            ("Tirana", 41.33, 19.82),
            ("Podgorica", 42.44, 19.26),
            ("Nicosia", 35.17, 33.36),
            ("Valletta", 35.90, 14.51),
        ] {
            assert_eq!(ids(key_at(lat, lon, 18)), ["versatiles", "eox"], "{name}");
        }
        for (name, lat, lon) in [
            ("Cologne", 50.94, 6.96),
            ("Aachen", 50.78, 6.08),
            ("Muenster", 51.96, 7.63),
            ("Siegen", 50.87, 8.02),
            ("Bielefeld", 52.02, 8.53),
            ("Minden", 52.29, 8.92),
            ("Hoexter", 51.77, 9.38),
        ] {
            assert_eq!(ids(key_at(lat, lon, 18))[0], "nrw", "{name}");
        }
        for (name, lat, lon) in [
            ("Hameln", 52.10, 9.28),
            ("Osnabrueck", 52.27, 8.05),
            ("Hannover", 52.37, 9.73),
            ("Goettingen", 51.53, 9.93),
            ("Oldenburg", 53.14, 8.21),
            ("Cuxhaven", 53.86, 8.69),
            ("Wolfsburg", 52.42, 10.78),
            ("Lueneburg", 53.25, 10.41),
            ("Luechow", 52.97, 11.15),
        ] {
            assert_eq!(
                ids(key_at(lat, lon, 18)),
                ["lgln", "versatiles", "eox"],
                "{name}"
            );
        }
        for (name, lat, lon) in [
            ("Hamburg", 53.55, 9.99),
            ("Kassel", 51.31, 9.49),
            ("Magdeburg", 52.13, 11.63),
            ("Erfurt", 50.98, 11.03),
        ] {
            assert_eq!(ids(key_at(lat, lon, 18)), ["versatiles", "eox"], "{name}");
        }
        assert_eq!(
            ids(key_at(51.72, 8.75, 18)),
            ["nrw", "versatiles", "eox"],
            "Paderborn"
        );
        // On the Austrian border, so Austria's service is asked first and finds nothing.
        assert_eq!(
            ids(key_at(48.15, 17.11, 18)),
            ["basemap-at", "versatiles", "eox"]
        );
        for (name, lat, lon) in [
            ("Warsaw", 52.23, 21.01),
            ("Rome", 41.90, 12.50),
            ("London", 51.50, -0.12),
            ("Tokyo", 35.68, 139.76),
        ] {
            assert_eq!(
                ids(key_at(lat, lon, 18)),
                ["eox"],
                "{name} has no service yet"
            );
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
    fn large_images_serve_the_tiles_one_level_finer() {
        let v = by_id("versatiles").unwrap();
        for z in [15, 16, 17, 18] {
            let k = key_at(50.11, 8.68, z);
            assert_eq!(v.source_key(k), k.ancestor(z - 1));
        }
        let k = key_at(50.11, 8.68, 19);
        assert_eq!(v.source_key(k), k.ancestor(17));
        assert_eq!(
            TileSource::url_from(v.url, TileKey::new(17, 69_760, 45_445)),
            "https://tiles.versatiles.org/tiles/satellite/17/69760/45445"
        );
    }

    #[test]
    fn tiles_split_further_only_where_detail_exists() {
        assert_eq!(max_tile_z(key_at(50.11, 8.68, 16)), 18);
        assert_eq!(
            max_tile_z(key_at(53.55, 9.99, 16)),
            18,
            "no finer service covers Hamburg"
        );
        assert_eq!(max_tile_z(key_at(52.10, 9.28, 16)), 19);
        assert_eq!(max_tile_z(key_at(47.27, 11.4, 16)), 19);
        assert_eq!(max_tile_z(key_at(48.14, 11.58, 16)), 18);
        assert_eq!(max_tile_z(key_at(40.42, -3.70, 16)), 19);
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
        assert!(
            TileSource::url_from(ign.url, k).contains("TILEMATRIX=18&TILEROW=91000&TILECOL=139000")
        );
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
    fn large_wms_images_are_requested_at_their_full_size() {
        let lgln = by_id("lgln").unwrap();
        let k = key_at(52.10, 9.28, 19);
        assert_eq!(lgln.source_key(k), k.ancestor(18));
        assert_eq!(lgln.source_key(k.ancestor(15)), k.ancestor(14));
        assert!(TileSource::url_from(lgln.url, k).contains("&WIDTH=512&HEIGHT=512&"));
        assert!(by_id("nrw").unwrap().url.contains("&WIDTH=256&HEIGHT=256&"));
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

    #[test]
    fn moon_tiles_are_addressed_by_row_then_column_and_cached_apart() {
        let k = TileKey::new(8, 100, 200);
        assert_eq!(
            TileSource::url_from(MOON_WAC.url, k),
            "https://trek.nasa.gov/tiles/Moon/EQ/LRO_WAC_Mosaic_Global_303ppd_v02/1.0.0/default/default028mm/8/200/100.jpg"
        );
        assert_eq!(MOON_WAC.cache_name(k), "nasa-trek-moon-wac/8/200/100.jpg");
        assert_ne!(MOON_WAC.cache_name(k), EOX.cache_name(k));
        assert_eq!(MOON_WAC.grid, Grid::Equirect);
        assert_eq!(by_id("moon-wac").map(|s| s.max_z), Some(8));
        // A tile deeper than the service has is taken from its level 8 ancestor.
        assert_eq!(
            MOON_WAC.source_key(TileKey::new(10, 4, 4)),
            TileKey::new(8, 1, 1)
        );
        // Its host is not Earth's, so server pacing is kept apart too.
        assert_ne!(
            crate::fetcher::host_of(MOON_WAC.url),
            crate::fetcher::host_of(EOX.url)
        );
    }
}
