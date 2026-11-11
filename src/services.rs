//! Map services the user adds: their description, the tile addresses built from it, and the
//! checks that keep a bad address or answer from doing harm. Pure and independent of the window.
//!
//! Three kinds are understood: XYZ (an address template), WMTS (from its capabilities document,
//! in web-mercator matrix sets such as GoogleMapsCompatible) and WMS 1.1.1 / 1.3.0 (asked for
//! EPSG:3857 pictures with GetMap). Standards: OGC 07-057r7 (WMTS), OGC 06-042 (WMS 1.3.0),
//! OGC 01-068r3 (WMS 1.1.1). Tile numbering for XYZ is the usual "slippy map" one, and TMS
//! counts rows from the south.

use crate::capabilities::{Document, MatrixSet, Wms, WmsLayer, Wmts, WmtsLayer};
use crate::sources::ABSOLUTE_MAX_Z;
use serde_json::{json, Value};

/// Half the width of the web-mercator square, in metres.
const MERCATOR_EDGE: f64 = 20_037_508.342_789_244;
/// The scale denominator of web-mercator zoom 0 for 256-pixel tiles at 0.28 mm per pixel, as
/// in the WMTS GoogleMapsCompatible definition.
const SCALE_AT_ZOOM_0: f64 = 559_082_264.028_717_8;
pub const MAX_URL_CHARS: usize = 2048;
const MAX_TEXT_CHARS: usize = 120;
/// WMS has no zoom limit of its own; this keeps requests to sensible detail.
const WMS_DEFAULT_MAX_Z: u8 = 18;
const XYZ_DEFAULT_MAX_Z: u8 = 19;
/// The names of web-mercator coordinate systems, in the order they are preferred.
const MERCATOR_CRS: [&str; 5] = [
    "EPSG:3857",
    "EPSG:900913",
    "EPSG:3785",
    "EPSG:102100",
    "EPSG:102113",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Xyz,
    Wmts,
    Wms,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Xyz => "xyz",
            Kind::Wmts => "wmts",
            Kind::Wms => "wms",
        }
    }

    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "xyz" => Kind::Xyz,
            "wmts" => Kind::Wmts,
            "wms" => Kind::Wms,
            _ => return None,
        })
    }
}

/// One map service, with everything needed to ask it for a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    /// Lower-case letters, digits and hyphens: names its folder in the cache.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// What the user gave: the address template, or the capabilities address.
    pub url: String,
    /// The tile address with `{z}`, `{x}`, `{y}` (or `{-y}`), or `{bbox}` for WMS.
    pub tiles: String,
    pub layer: String,
    pub format: String,
    pub tile_size: u32,
    pub min_z: u8,
    pub max_z: u8,
    /// West, south, east, north in degrees; none means the whole world.
    pub bounds: Option<[f64; 4]>,
    /// What the service itself asks to be credited, if it says.
    pub attribution: String,
    /// What the user wants shown as credit.
    pub credit: String,
}

impl Service {
    pub fn host(&self) -> String {
        crate::fetcher::host_of(&self.tiles)
    }

    /// The credit shown while the service's pictures are on screen: the user's own wording,
    /// else what the service asks for, else at least where the pictures come from.
    pub fn shown_credit(&self) -> String {
        [&self.credit, &self.attribution]
            .into_iter()
            .map(|c| c.trim())
            .find(|c| !c.is_empty())
            .map_or_else(|| self.host(), str::to_string)
    }

    /// Sets the id from the name and the address, so the same service always has the same one.
    pub fn assign_id(&mut self) {
        let slug: String = self
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let slug: String = slug.chars().take(24).collect();
        let hash = fnv(&format!("{}\n{}\n{}", self.tiles, self.layer, self.format));
        self.id = if slug.is_empty() {
            format!("{hash:08x}")
        } else {
            format!("{slug}-{hash:08x}")
        };
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "kind": self.kind.name(), "url": self.url,
            "tiles": self.tiles, "layer": self.layer, "format": self.format, "tile_size": self.tile_size,
            "min_zoom": self.min_z, "max_zoom": self.max_z, "bounds": self.bounds,
            "attribution": self.attribution, "credit": self.credit,
        })
    }

    /// Reads an entry, checking it the same way a freshly typed one is checked: the file
    /// is the user's, but may have been edited or damaged.
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(clean);
        let need = |k: &str| {
            text(k)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| format!("no {k}"))
        };
        let num = |k: &str| {
            v.get(k)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("no {k}"))
        };
        let kind = Kind::from_name(&need("kind")?).ok_or("unknown kind")?;
        let tiles = v
            .get("tiles")
            .and_then(Value::as_str)
            .ok_or("no tiles address")?
            .to_string();
        check_url(&tiles)?;
        let bounds = match v.get("bounds") {
            Some(Value::Array(a)) if a.len() == 4 => {
                let n: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
                Some(<[f64; 4]>::try_from(n).map_err(|_| "bad bounds")?)
            }
            _ => None,
        };
        let mut s = Service {
            id: String::new(),
            name: need("name")?,
            kind,
            url: v
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            tiles,
            layer: text("layer").unwrap_or_default(),
            format: text("format").unwrap_or_default(),
            tile_size: u32::try_from(num("tile_size")?).map_err(|_| "bad tile size")?,
            min_z: u8::try_from(num("min_zoom")?).map_err(|_| "bad zoom")?,
            max_z: u8::try_from(num("max_zoom")?).map_err(|_| "bad zoom")?,
            bounds,
            attribution: text("attribution").unwrap_or_default(),
            credit: text("credit").unwrap_or_default(),
        };
        if !(64..=1024).contains(&s.tile_size) || s.min_z > s.max_z || s.max_z > ABSOLUTE_MAX_Z {
            return Err("tile size or zoom out of range".into());
        }
        if !has_tile_fields(&s.tiles, kind) {
            return Err("the tile address lacks its tile numbers".into());
        }
        // The stored id names a folder, so it is only used if it is plain.
        let stored = text("id").unwrap_or_default();
        if !stored.is_empty()
            && stored.len() <= 64
            && stored
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            s.id = stored;
        } else {
            s.assign_id();
        }
        Ok(s)
    }
}

fn fnv(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

/// Text from a file or a service, without control characters and not too long.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn has_tile_fields(tiles: &str, kind: Kind) -> bool {
    match kind {
        Kind::Wms => tiles.contains("{bbox}"),
        _ => {
            (tiles.contains("{z}") || tiles.contains("{z02}"))
                && tiles.contains("{x}")
                && (tiles.contains("{y}") || tiles.contains("{-y}"))
        }
    }
}

/// Percent-encodes a value for use in an address; the colon is kept as it is harmless.
pub fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b':') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Only plain http and https addresses with a host and no user name or password are used.
pub fn check_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL_CHARS {
        return Err("the address is too long".into());
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the address contains spaces or control characters".into());
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .ok_or("only http and https addresses are used")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("addresses with a user name or password are not used".into());
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            authority
        }
    });
    if host.is_empty() || host.contains(['{', '}']) {
        return Err("the address has no host name".into());
    }
    Ok(())
}

/// Adds a query to an address that may already have one, as WMS and WMTS addresses often do.
pub fn append_query(base: &str, query: &str) -> String {
    match base.split_once('#').map_or(base, |b| b.0) {
        b if !b.contains('?') => format!("{b}?{query}"),
        b if b.ends_with('?') || b.ends_with('&') => format!("{b}{query}"),
        b => format!("{b}&{query}"),
    }
}

// ---- XYZ ----

/// An XYZ service from an address template. `{s}` becomes `a` so that all tiles come from one
/// host, which keeps the pacing and the limit on requests meaningful. `tms` counts rows from
/// the south, as `{-y}` does.
pub fn xyz_service(template: &str, tms: bool) -> Result<Service, String> {
    let mut tiles = template.trim().replace("{s}", "a").replace("{S}", "a");
    check_url(&tiles)?;
    tiles = tiles
        .replace("{Z}", "{z}")
        .replace("{X}", "{x}")
        .replace("{Y}", "{y}");
    if tms {
        tiles = tiles.replace("{y}", "{-y}");
    }
    if tiles.contains("{bbox}") || !has_tile_fields(&tiles, Kind::Xyz) {
        return Err(
            "an XYZ address needs {z}, {x} and {y} (or {-y}) where the tile numbers go".into(),
        );
    }
    let mut s = Service {
        id: String::new(),
        name: crate::fetcher::host_of(&tiles),
        kind: Kind::Xyz,
        url: template.trim().to_string(),
        tiles,
        layer: String::new(),
        format: String::new(),
        tile_size: 256,
        min_z: 0,
        max_z: XYZ_DEFAULT_MAX_Z,
        bounds: None,
        attribution: String::new(),
        credit: String::new(),
    };
    s.assign_id();
    Ok(s)
}

/// The address to ask for a capabilities document. An address that already is a request is
/// used as it is, as is one that names an XML file; otherwise the request is added, for WMTS if
/// the address says so and for WMS if not.
pub fn capabilities_request(url: &str) -> String {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or("");
    if lower.contains("request=getcapabilities") || path.ends_with(".xml") {
        return url.to_string();
    }
    if lower.contains("wmts") {
        append_query(url, "SERVICE=WMTS&REQUEST=GetCapabilities&VERSION=1.0.0")
    } else {
        append_query(url, "SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0")
    }
}

/// Whether an address is an XYZ template rather than a place to ask for a description.
pub fn looks_like_template(url: &str) -> bool {
    url.contains("{z}") || url.contains("{Z}")
}

/// The layers a description offers, as (identifier, title).
pub fn layer_list(doc: &Document) -> Vec<(String, String)> {
    match doc {
        Document::Wmts(d) => d
            .layers
            .iter()
            .map(|l| (l.id.clone(), l.title.clone()))
            .collect(),
        Document::Wms(d) => d
            .layers
            .iter()
            .map(|l| (l.name.clone(), l.title.clone()))
            .collect(),
    }
}

/// A service from a capabilities document, for the named layer or else the first usable one.
pub fn from_capabilities(
    doc: &Document,
    url: &str,
    layer: Option<&str>,
) -> Result<Service, String> {
    let mut s = match doc {
        Document::Wmts(d) => wmts_service(d, url, layer)?,
        Document::Wms(d) => wms_service(d, url, layer)?,
    };
    s.url = url.trim().to_string();
    check_url(&s.tiles)?;
    s.assign_id();
    Ok(s)
}

fn pick<'a, T>(
    items: &'a [T],
    wanted: Option<&str>,
    id: impl Fn(&T) -> &str,
    title: impl Fn(&T) -> &str,
) -> Result<Vec<&'a T>, String> {
    match wanted {
        None => Ok(items.iter().collect()),
        Some(w) => {
            let exact: Vec<_> = items.iter().filter(|i| id(i) == w).collect();
            if !exact.is_empty() {
                return Ok(exact);
            }
            let loose: Vec<_> = items
                .iter()
                .filter(|i| id(i).eq_ignore_ascii_case(w) || title(i).eq_ignore_ascii_case(w))
                .collect();
            if loose.is_empty() {
                Err(format!("the service has no layer called {w}"))
            } else {
                Ok(loose)
            }
        }
    }
}

fn best_format<'a>(offered: impl Iterator<Item = &'a str>) -> Option<String> {
    let all: Vec<&str> = offered.collect();
    ["image/png", "image/jpeg", "image/jpg", "image/webp"]
        .into_iter()
        .find_map(|want| {
            all.iter()
                .find(|f| f.trim().to_ascii_lowercase().starts_with(want))
                .map(|f| {
                    // The base type only: parameters such as "; mode=8bit" are not part of a request.
                    f.split(';').next().unwrap_or(f).trim().to_string()
                })
        })
}

// ---- WMTS ----

fn is_mercator_name(crs: &str) -> bool {
    let c = crs.to_ascii_uppercase();
    ["3857", "900913", "3785", "102100", "102113"]
        .iter()
        .any(|code| c.ends_with(code))
}

/// The levels of a matrix set that match web-mercator tiling, as (level, matrix).
fn mercator_levels(set: &MatrixSet) -> Vec<(u8, &crate::capabilities::Matrix)> {
    if !is_mercator_name(&set.crs) {
        return Vec::new();
    }
    set.matrices
        .iter()
        .filter_map(|m| {
            let square = m.tile.0 == m.tile.1 && (128..=1024).contains(&m.tile.0);
            let whole = m.size.0 == m.size.1 && m.size.0.is_power_of_two();
            let corner = (m.top_left.0.abs() - MERCATOR_EDGE).abs() < 1.0
                && (m.top_left.1.abs() - MERCATOR_EDGE).abs() < 1.0;
            let z = m.size.0.trailing_zeros() as u8;
            (square && whole && corner && z <= ABSOLUTE_MAX_Z).then_some((z, m))
        })
        .collect()
}

fn describe_sets(d: &Wmts, l: &WmtsLayer) -> String {
    let names: Vec<String> = l
        .sets
        .iter()
        .map(|id| match d.sets.iter().find(|s| &s.id == id) {
            Some(s) if !s.crs.is_empty() => {
                format!("{id} ({})", s.crs.rsplit("::").next().unwrap_or(&s.crs))
            }
            _ => id.clone(),
        })
        .collect();
    if names.is_empty() {
        "no matrix sets".into()
    } else {
        names.join(", ")
    }
}

fn wmts_service(d: &Wmts, url: &str, wanted: Option<&str>) -> Result<Service, String> {
    let candidates = pick(&d.layers, wanted, |l| &l.id, |l| &l.title)?;
    let mut last = String::new();
    for l in &candidates {
        match wmts_layer_service(d, l, url) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    if wanted.is_none() && candidates.len() > 1 {
        return Err(format!(
            "none of the {} layers can be used. For example, {last}",
            candidates.len()
        ));
    }
    Err(last)
}

fn wmts_layer_service(d: &Wmts, l: &WmtsLayer, url: &str) -> Result<Service, String> {
    let format = best_format(l.formats.iter().map(String::as_str))
        .ok_or_else(|| format!("layer {} offers no PNG, JPEG or WebP tiles", l.id))?;
    // The first linked set that has web-mercator levels.
    let mut chosen = None;
    for set_id in &l.sets {
        let Some(set) = d.sets.iter().find(|s| &s.id == set_id) else {
            continue;
        };
        let allowed = l
            .limits
            .iter()
            .find(|(s, _)| s == set_id)
            .map(|(_, ids)| ids);
        let levels: Vec<_> = mercator_levels(set)
            .into_iter()
            .filter(|(_, m)| allowed.is_none_or(|ids| ids.contains(&m.id)))
            .collect();
        if !levels.is_empty() {
            chosen = Some((set, levels));
            break;
        }
    }
    let Some((set, levels)) = chosen else {
        return Err(format!(
            "layer {} is offered only in matrix sets that are not web mercator: {}. Only web-mercator sets (such as GoogleMapsCompatible) can be used",
            l.id,
            describe_sets(d, l)
        ));
    };
    // The template can only count levels from their numbers, so identifiers must end in them.
    let prefix = levels[0]
        .1
        .id
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string();
    let levels = &levels;
    // Some services pad the level to two digits ("00", "01", ... "18").
    let padded = levels
        .iter()
        .all(|(z, m)| m.id == format!("{prefix}{z:02}"));
    let follows = padded || levels.iter().all(|(z, m)| m.id == format!("{prefix}{z}"));
    let level_token = if padded && levels.iter().any(|(z, _)| *z < 10) {
        "{z02}"
    } else {
        "{z}"
    };
    if !follows {
        // Name the first matrix that breaks the pattern, which is the one to look at.
        let (z, odd) = levels
            .iter()
            .find(|(z, m)| m.id != format!("{prefix}{z}") && m.id != format!("{prefix}{z:02}"))
            .unwrap_or(&levels[0]);
        return Err(format!(
            "layer {} uses matrix names that do not follow the zoom levels: level {z} is called {}",
            l.id, odd.id
        ));
    }
    let (min_z, max_z) = (
        levels.iter().map(|x| x.0).min().unwrap_or(0),
        levels.iter().map(|x| x.0).max().unwrap_or(0),
    );
    let style = l.styles.first().map_or("default", String::as_str);
    let matrix = format!("{prefix}{level_token}");
    let resource = l
        .resources
        .iter()
        .find(|r| r.format.eq_ignore_ascii_case(&format))
        .or_else(|| {
            l.resources
                .first()
                .filter(|r| best_format(std::iter::once(r.format.as_str())).is_some())
        });
    let (tiles, format) = match resource {
        Some(r) => (
            rest_template(&r.template, l, &set.id, style, &matrix)?,
            best_format(std::iter::once(r.format.as_str())).unwrap_or(format),
        ),
        None => {
            let base = d
                .get_tile_url
                .clone()
                .unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).to_string());
            let mut q = format!(
                "SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER={}&STYLE={}&TILEMATRIXSET={}&TILEMATRIX={}&TILEROW={{y}}&TILECOL={{x}}&FORMAT={}",
                pct(&l.id),
                pct(style),
                pct(&set.id),
                pct(&prefix) + level_token,
                pct(&format)
            );
            for (name, value) in &l.dimensions {
                q.push_str(&format!("&{}={}", pct(name), pct(value)));
            }
            (append_query(&base, &q), format)
        }
    };
    Ok(Service {
        id: String::new(),
        name: clean(&if d.title.is_empty() || d.title == l.title {
            l.title.clone()
        } else {
            format!("{}: {}", d.title, l.title)
        }),
        kind: Kind::Wmts,
        url: String::new(),
        tiles,
        layer: l.id.clone(),
        format,
        tile_size: levels[0].1.tile.0,
        min_z,
        max_z: max_z.min(ABSOLUTE_MAX_Z),
        bounds: l.bounds,
        attribution: String::new(),
        credit: String::new(),
    })
}

/// A RESTful template with its parts filled in, leaving only the tile numbers.
fn rest_template(
    template: &str,
    l: &WmtsLayer,
    set: &str,
    style: &str,
    matrix: &str,
) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..]
            .find('}')
            .ok_or("the tile address template is damaged")?
            + open;
        let name = &rest[open + 1..close];
        let value = match name {
            "TileMatrixSet" => pct(set),
            "TileMatrix" => matrix.to_string(),
            "TileRow" => "{y}".to_string(),
            "TileCol" => "{x}".to_string(),
            "Style" => pct(style),
            other => match l.dimensions.iter().find(|d| d.0 == other) {
                Some((_, v)) => pct(v),
                None => {
                    return Err(format!(
                        "layer {} needs a value for {other}, which has no default",
                        l.id
                    ))
                }
            },
        };
        out.push_str(&value);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

// ---- WMS ----

fn mercator_crs(l: &WmsLayer) -> Option<&str> {
    MERCATOR_CRS.iter().find_map(|want| {
        l.crs
            .iter()
            .find(|c| c.eq_ignore_ascii_case(want))
            .map(String::as_str)
    })
}

/// The highest zoom whose pictures are not finer than the layer's smallest scale allows.
pub fn max_zoom_for_scale(min_scale_denominator: f64) -> u8 {
    (SCALE_AT_ZOOM_0 / min_scale_denominator)
        .log2()
        .floor()
        .clamp(0.0, ABSOLUTE_MAX_Z as f64) as u8
}

fn wms_service(d: &Wms, url: &str, wanted: Option<&str>) -> Result<Service, String> {
    let crs_param = match d.version.as_str() {
        "1.3.0" => "CRS",
        "1.1.1" => "SRS",
        other => {
            return Err(format!(
                "WMS version {} is not supported (1.1.1 and 1.3.0 are)",
                if other.is_empty() { "unknown" } else { other }
            ))
        }
    };
    let candidates = pick(&d.layers, wanted, |l| &l.name, |l| &l.title)?;
    let usable: Vec<_> = candidates
        .iter()
        .filter(|l| mercator_crs(l).is_some())
        .collect();
    let Some(l) = usable.first() else {
        let offered = candidates
            .first()
            .map(|l| l.crs.join(", "))
            .unwrap_or_default();
        return Err(format!("no layer is offered in EPSG:3857 (web mercator), which is what is asked for. The first offers: {}", if offered.is_empty() { "no coordinate systems".into() } else { offered }));
    };
    let crs = mercator_crs(l).unwrap_or("EPSG:3857");
    let format = best_format(d.formats.iter().map(String::as_str))
        .ok_or("the service offers no PNG, JPEG or WebP pictures")?;
    let base = d
        .get_map_url
        .clone()
        .unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).to_string());
    // EPSG:3857 is east then north in both versions; 1.3.0 swaps only for coordinate systems
    // (such as EPSG:4326) whose own axis order is latitude first, which are never asked for.
    let mut q = format!(
        "SERVICE=WMS&VERSION={}&REQUEST=GetMap&LAYERS={}&STYLES=&{crs_param}={}&BBOX={{bbox}}&WIDTH=256&HEIGHT=256&FORMAT={}",
        d.version,
        pct(&l.name),
        pct(&crs.to_ascii_uppercase()),
        pct(&format)
    );
    if format.ends_with("png") || format.ends_with("webp") {
        q.push_str("&TRANSPARENT=TRUE");
    }
    Ok(Service {
        id: String::new(),
        name: clean(&if d.title.is_empty() || d.title == l.title {
            l.title.clone()
        } else {
            format!("{}: {}", d.title, l.title)
        }),
        kind: Kind::Wms,
        url: String::new(),
        tiles: append_query(&base, &q),
        layer: l.name.clone(),
        format,
        tile_size: 256,
        min_z: 0,
        max_z: l
            .min_scale_denominator
            .map_or(WMS_DEFAULT_MAX_Z, max_zoom_for_scale),
        bounds: l.bounds,
        attribution: l.attribution.as_deref().map(clean).unwrap_or_default(),
        credit: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{fixtures, parse};

    #[test]
    fn xyz_templates_are_checked_and_normalised() {
        let s = xyz_service("https://{s}.tile.example.org/{z}/{x}/{y}.png", false).unwrap();
        assert_eq!(s.tiles, "https://a.tile.example.org/{z}/{x}/{y}.png");
        assert_eq!(s.name, "a.tile.example.org");
        assert_eq!(s.kind, Kind::Xyz);
        assert_eq!(
            xyz_service("https://t.example/{z}/{x}/{y}.png", true)
                .unwrap()
                .tiles,
            "https://t.example/{z}/{x}/{-y}.png"
        );
        assert!(xyz_service("https://t.example/{z}/{x}/{-y}.png", false).is_ok());
        assert!(xyz_service("https://t.example/{z}/{x}.png", false).is_err());
        assert!(xyz_service("https://t.example/tiles.png", false).is_err());
        assert!(xyz_service("https://t.example/{bbox}/{z}/{x}/{y}", false).is_err());
    }

    #[test]
    fn only_plain_http_and_https_addresses_are_allowed() {
        for bad in [
            "file:///etc/passwd/{z}/{x}/{y}",
            "ftp://t.example/{z}/{x}/{y}",
            "javascript:alert(1)",
            "https://user:pw@t.example/{z}/{x}/{y}",
            "https://user@t.example/{z}/{x}/{y}",
            "https:///{z}/{x}/{y}",
            "https://t.example/a b/{z}/{x}/{y}",
            "t.example/{z}/{x}/{y}",
        ] {
            assert!(xyz_service(bad, false).is_err(), "{bad}");
        }
        assert!(check_url("HTTPS://T.example:8080/{z}/{x}/{y}").is_ok());
        assert!(
            check_url("http://t.example/x?a=b@c").is_ok(),
            "an @ after the host is not credentials"
        );
        assert!(check_url(&format!("https://t.example/{}", "a".repeat(MAX_URL_CHARS))).is_err());
    }

    #[test]
    fn capabilities_requests_are_built_from_plain_addresses() {
        assert_eq!(
            capabilities_request("https://h.example/wms"),
            "https://h.example/wms?SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0"
        );
        assert_eq!(
            capabilities_request("https://h.example/wms?map=a"),
            "https://h.example/wms?map=a&SERVICE=WMS&REQUEST=GetCapabilities&VERSION=1.3.0"
        );
        assert!(capabilities_request("https://h.example/WMTS/service").contains("SERVICE=WMTS"));
        assert_eq!(
            capabilities_request("https://h.example/x/WMTSCapabilities.xml"),
            "https://h.example/x/WMTSCapabilities.xml"
        );
        let given = "https://h.example/wms?service=WMS&request=GetCapabilities";
        assert_eq!(capabilities_request(given), given);
        assert!(looks_like_template("https://t/{z}/{x}/{y}") && !looks_like_template(given));
    }

    #[test]
    fn wmts_uses_the_restful_template_with_the_matrix_set_filled_in() {
        let doc = parse(fixtures::WMTS).unwrap();
        let s = from_capabilities(&doc, "https://t.example/wmts?x=1", None).unwrap();
        assert_eq!(s.kind, Kind::Wmts);
        assert_eq!(
            s.tiles,
            "https://t.example/rest/relief/default/GoogleMapsCompatible/{z}/{y}/{x}.png"
        );
        assert_eq!(
            (s.min_z, s.max_z),
            (0, 2),
            "the layer's limits stop at level 2"
        );
        assert_eq!(s.format, "image/png");
        assert_eq!(s.bounds, Some([5.0, 45.0, 11.0, 48.0]));
        assert_eq!(s.name, "Test WMTS: Relief");
        assert_eq!(s.layer, "relief");
        assert!(s.id.starts_with("test-wmts-relief-"));
    }

    #[test]
    fn wmts_with_two_digit_matrix_names_uses_a_padded_level() {
        let xml = fixtures::WMTS
            .replace(
                "<ows:Identifier>0</ows:Identifier>",
                "<ows:Identifier>00</ows:Identifier>",
            )
            .replace(
                "<ows:Identifier>1</ows:Identifier>",
                "<ows:Identifier>01</ows:Identifier>",
            )
            .replace(
                "<ows:Identifier>2</ows:Identifier>",
                "<ows:Identifier>02</ows:Identifier>",
            )
            .replace(
                "<ows:Identifier>3</ows:Identifier>",
                "<ows:Identifier>03</ows:Identifier>",
            )
            .replace("<TileMatrix>0</TileMatrix>", "<TileMatrix>00</TileMatrix>")
            .replace("<TileMatrix>1</TileMatrix>", "<TileMatrix>01</TileMatrix>")
            .replace("<TileMatrix>2</TileMatrix>", "<TileMatrix>02</TileMatrix>");
        let s = from_capabilities(
            &parse(&xml).unwrap(),
            "https://t.example/cap",
            Some("relief"),
        )
        .unwrap();
        assert!(
            s.tiles.contains("GoogleMapsCompatible/{z02}/{y}/{x}.png"),
            "{}",
            s.tiles
        );
        let k = crate::geo::TileKey::new(3, 5, 2);
        assert!(crate::sources::TileSource::url_from(&s.tiles, k).contains("/03/2/5.png"));
    }

    #[test]
    fn wmts_falls_back_to_key_value_requests() {
        let xml = fixtures::WMTS
            .replace("<ResourceURL", "<Gone")
            .replace("</ResourceURL>", "</Gone>");
        let xml = xml.replace(r#"<Gone format="image/png" resourceType="tile" template="https://t.example/rest/relief/{Style}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.png"/>"#, "");
        let doc = parse(&xml).unwrap();
        let s = from_capabilities(&doc, "https://t.example/cap", Some("relief")).unwrap();
        assert_eq!(
            s.tiles,
            "https://t.example/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=relief&STYLE=default&TILEMATRIXSET=GoogleMapsCompatible&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}&FORMAT=image%2Fpng&Time=current"
        );
    }

    #[test]
    fn wmts_says_clearly_when_only_other_sets_are_offered() {
        let doc = parse(fixtures::WMTS).unwrap();
        let e = from_capabilities(&doc, "https://t.example/cap", Some("polar")).unwrap_err();
        assert!(
            e.contains("not web mercator") && e.contains("EPSG:4326 (4326)"),
            "{e}"
        );
        let e = from_capabilities(&doc, "https://t.example/cap", Some("nothing")).unwrap_err();
        assert!(e.contains("no layer called nothing"), "{e}");
    }

    #[test]
    fn wmts_matrix_names_must_follow_the_levels() {
        let xml = fixtures::WMTS.replace(
            "<ows:Identifier>1</ows:Identifier>",
            "<ows:Identifier>one</ows:Identifier>",
        );
        let xml = xml.replace("<TileMatrix>1</TileMatrix>", "<TileMatrix>one</TileMatrix>");
        let doc = parse(&xml).unwrap();
        let e = from_capabilities(&doc, "https://t.example/cap", Some("relief")).unwrap_err();
        assert!(
            e.ends_with("do not follow the zoom levels: level 1 is called one"),
            "{e}"
        );
    }

    #[test]
    fn wms_130_asks_for_crs_and_east_north_boxes() {
        let doc = parse(fixtures::WMS130).unwrap();
        let s = from_capabilities(&doc, "https://w.example/wms", None).unwrap();
        assert_eq!(s.kind, Kind::Wms);
        assert_eq!(
            s.tiles,
            "https://w.example/wms?map=a&SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=ortho&STYLES=&CRS=EPSG:3857&BBOX={bbox}&WIDTH=256&HEIGHT=256&FORMAT=image%2Fpng&TRANSPARENT=TRUE"
        );
        assert_eq!(s.attribution, "Test Agency");
        assert_eq!(s.bounds, Some([-10.0, 35.0, 30.0, 60.0]));
        assert_eq!(s.max_z, WMS_DEFAULT_MAX_Z);
        // The box for the whole world is the same square in both versions.
        let url = crate::sources::TileSource::url_from(&s.tiles, crate::geo::TileKey::new(0, 0, 0));
        assert!(
            url.contains("BBOX=-20037508.343,-20037508.343,20037508.343,20037508.343"),
            "{url}"
        );
        // Zoom 1, tile x=1, y=0 is the north-east quarter: east and north come last.
        let url = crate::sources::TileSource::url_from(&s.tiles, crate::geo::TileKey::new(1, 1, 0));
        assert!(
            url.contains("BBOX=0.000,0.000,20037508.343,20037508.343"),
            "{url}"
        );
    }

    #[test]
    fn wms_111_asks_for_srs_and_takes_an_alias_of_mercator() {
        let doc = parse(fixtures::WMS111).unwrap();
        let s = from_capabilities(&doc, "http://old.example/wms", Some("land")).unwrap();
        assert!(s.tiles.starts_with("http://old.example/wms?SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=land&STYLES=&SRS=EPSG:900913&BBOX={bbox}"), "{}", s.tiles);
        assert!(s.tiles.ends_with("FORMAT=image%2Fpng&TRANSPARENT=TRUE"));
    }

    #[test]
    fn wms_without_mercator_or_with_an_old_version_is_refused_plainly() {
        let doc = parse(&fixtures::WMS130.replace("<CRS>EPSG:3857</CRS>", "")).unwrap();
        let e = from_capabilities(&doc, "https://w.example/wms", None).unwrap_err();
        assert!(e.contains("EPSG:3857") && e.contains("EPSG:4326"), "{e}");
        let doc =
            parse(&fixtures::WMS130.replace("version=\"1.3.0\"", "version=\"1.1.0\"")).unwrap();
        assert!(from_capabilities(&doc, "https://w.example/wms", None)
            .unwrap_err()
            .contains("1.1.0"));
    }

    #[test]
    fn wms_max_zoom_follows_the_smallest_scale() {
        assert_eq!(max_zoom_for_scale(559_082_264.0), 0);
        assert_eq!(max_zoom_for_scale(2183.0), 17);
        assert_eq!(max_zoom_for_scale(1.0), ABSOLUTE_MAX_Z);
        let doc = parse(fixtures::WMS130).unwrap();
        assert_eq!(
            from_capabilities(&doc, "https://w.example/wms", Some("dem"))
                .unwrap()
                .max_z,
            17
        );
    }

    #[test]
    fn credit_falls_back_from_the_users_to_the_services_to_the_host() {
        let mut s = xyz_service("https://t.example/{z}/{x}/{y}.png", false).unwrap();
        assert_eq!(s.shown_credit(), "t.example");
        s.attribution = "From the service".into();
        assert_eq!(s.shown_credit(), "From the service");
        s.credit = "  Mine ".into();
        assert_eq!(s.shown_credit(), "Mine");
    }

    #[test]
    fn json_round_trip_and_hostile_files() {
        let doc = parse(fixtures::WMS130).unwrap();
        let s = from_capabilities(&doc, "https://w.example/wms", None).unwrap();
        assert_eq!(Service::from_json(&s.to_json()).unwrap(), s);
        let mut v = s.to_json();
        v["id"] = json!("../../etc");
        let back = Service::from_json(&v).unwrap();
        assert_ne!(back.id, "../../etc");
        assert!(back
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        for (key, value) in [
            ("tiles", json!("file:///x/{z}/{x}/{y}")),
            ("tiles", json!("https://u:p@h/{z}/{x}/{y}")),
            ("kind", json!("ftp")),
            ("max_zoom", json!(60)),
            ("tile_size", json!(5)),
        ] {
            let mut v = s.to_json();
            v[key] = value;
            assert!(Service::from_json(&v).is_err(), "{key}");
        }
        assert!(Service::from_json(&json!({})).is_err());
        assert!(Service::from_json(&json!("text")).is_err());
    }

    #[test]
    fn ids_are_stable_and_differ_by_address() {
        let a = xyz_service("https://t.example/{z}/{x}/{y}.png", false).unwrap();
        let b = xyz_service("https://t.example/{z}/{x}/{y}.png", false).unwrap();
        let c = xyz_service("https://t.example/{z}/{y}/{x}.png", false).unwrap();
        assert_eq!(a.id, b.id);
        assert_ne!(a.id, c.id);
    }

    #[test]
    fn queries_are_appended_in_the_right_form() {
        assert_eq!(append_query("https://a/b", "x=1"), "https://a/b?x=1");
        assert_eq!(append_query("https://a/b?", "x=1"), "https://a/b?x=1");
        assert_eq!(
            append_query("https://a/b?k=v", "x=1"),
            "https://a/b?k=v&x=1"
        );
        assert_eq!(
            append_query("https://a/b?k=v&", "x=1"),
            "https://a/b?k=v&x=1"
        );
        assert_eq!(pct("image/png"), "image%2Fpng");
        assert_eq!(pct("EPSG:3857"), "EPSG:3857");
    }
}
