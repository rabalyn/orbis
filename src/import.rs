//! Reads places, tracks and outlines from files: GeoJSON (RFC 7946), GPX 1.1, KML 2.2 (OGC),
//! KMZ (a zip holding a KML) and IGC (FAI flight logs). Independent of the window system.
//! Files are not trusted: sizes are limited, nothing here panics on bad input, and a KMZ is
//! only ever read into memory (entry names are never used as paths) with a limit on what
//! decompression may produce.

use crate::search::{self, Place, Target};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

/// The largest file read, in bytes. Compressed KMZ contents are held to the same limit.
pub const MAX_FILE_BYTES: u64 = 64 << 20;
/// Items kept from one file. Anything beyond is dropped and reported.
const MAX_ITEMS: usize = 50_000;
/// Positions kept from one file.
const MAX_POINTS: usize = 3_000_000;
const MAX_ZIP_ENTRIES: usize = 10_000;
const MAX_XML_DEPTH: usize = 128;
const MAX_NAME_CHARS: usize = 120;
const MAX_GEOMETRY_DEPTH: usize = 8;

/// A position. Angles are in radians, the altitude in metres where the file has one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pos {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<f64>,
}

impl Pos {
    /// From degrees; `None` for anything that is not a position on the globe.
    fn from_degrees(lat: f64, lon: f64, alt: Option<f64>) -> Option<Pos> {
        let ok = lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0;
        ok.then(|| Pos { lat: lat.to_radians(), lon: lon.to_radians(), alt: alt.filter(|a| a.is_finite()) })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Point(Pos),
    Line(Vec<Pos>),
    /// The outer ring, then any holes.
    Polygon(Vec<Vec<Pos>>),
}

/// How a point is drawn when it is not the plain dot: its radius in pixels and its colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub radius: f32,
    pub colour: [u8; 4],
}

/// How an area is drawn when it is not the plain yellow one: edge and fill colours, and the
/// edge's width in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    pub edge: [u8; 4],
    pub fill: [u8; 4],
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub shape: Shape,
    /// South, west, north and east limits of the positions, not allowing for the antimeridian.
    pub extent: [f64; 4],
    pub mark: Option<Mark>,
    pub outline: Option<Outline>,
}

impl Item {
    pub fn new(name: &str, shape: Shape) -> Item {
        let mut item = Item { name: clean(name), shape, extent: [0.0; 4], mark: None, outline: None };
        let mut e = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in item.positions() {
            e = [e[0].min(p.lat), e[1].min(p.lon), e[2].max(p.lat), e[3].max(p.lon)];
        }
        item.extent = e;
        item
    }

    pub fn positions(&self) -> Box<dyn Iterator<Item = &Pos> + '_> {
        match &self.shape {
            Shape::Point(p) => Box::new(std::iter::once(p)),
            Shape::Line(l) => Box::new(l.iter()),
            Shape::Polygon(rings) => Box::new(rings.iter().flatten()),
        }
    }

    /// What the item is called in a list.
    pub fn title(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match self.shape {
            Shape::Point(_) => "Point",
            Shape::Line(_) => "Line",
            Shape::Polygon(_) => "Area",
        }
        .to_string()
    }

    /// Where to fly to see the item.
    pub fn target(&self) -> Option<Target> {
        target_for(self.positions().map(|p| (p.lat, p.lon)), matches!(self.shape, Shape::Point(_)))
    }
}

/// The contents of one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imported {
    pub name: String,
    pub items: Vec<Item>,
    /// Some of the file was left out for being too big.
    pub truncated: bool,
}

impl Imported {
    pub fn target(&self) -> Option<Target> {
        target_for(self.items.iter().flat_map(|i| i.positions()).map(|p| (p.lat, p.lon)), false)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportError {
    TooLarge,
    Unrecognised,
    Malformed(String),
    Empty,
    Io(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::TooLarge => write!(f, "the file is too large"),
            ImportError::Unrecognised => write!(f, "not GeoJSON, GPX, KML, KMZ or IGC"),
            ImportError::Malformed(why) => write!(f, "damaged: {why}"),
            ImportError::Empty => write!(f, "nothing to show in it"),
            ImportError::Io(why) => write!(f, "{why}"),
        }
    }
}

/// South, west, north, east of some positions. If they are spread around the globe, the box
/// is the narrowest one, so that it may cross the antimeridian, with east west of west.
pub fn bounding_box(points: impl Iterator<Item = (f64, f64)>) -> Option<[f64; 4]> {
    let (mut s, mut n) = (f64::MAX, f64::MIN);
    let mut lons = Vec::new();
    for (lat, lon) in points {
        s = s.min(lat);
        n = n.max(lat);
        lons.push(lon);
    }
    if lons.is_empty() {
        return None;
    }
    lons.sort_by(f64::total_cmp);
    // The biggest gap between neighbouring longitudes, round the back, is left out.
    let tau = 2.0 * std::f64::consts::PI;
    let mut gap = lons[0] + tau - lons[lons.len() - 1];
    let (mut west, mut east) = (lons[0], lons[lons.len() - 1]);
    for w in lons.windows(2) {
        if w[1] - w[0] > gap {
            gap = w[1] - w[0];
            west = w[1];
            east = w[0];
        }
    }
    Some([s, west, n, east])
}

fn target_for(points: impl Iterator<Item = (f64, f64)>, single: bool) -> Option<Target> {
    let [s, w, n, e] = bounding_box(points)?;
    let mut dlon = e - w;
    if dlon < 0.0 {
        dlon += 2.0 * std::f64::consts::PI;
    }
    let mut lon = w + dlon / 2.0;
    if lon > std::f64::consts::PI {
        lon -= 2.0 * std::f64::consts::PI;
    }
    let place = Place { name: String::new(), lat: (s + n) / 2.0, lon, bbox: (!single).then_some([s, w, n, e]) };
    Some(Target { lat: place.lat, lon: place.lon, distance: search::distance_for(&place) })
}

/// Everything imported in this run.
#[derive(Default)]
pub struct Library {
    docs: Arc<Vec<Arc<Imported>>>,
    /// Counts changes, so that pictures made from the contents know when to start over.
    pub generation: u64,
}

impl Library {
    pub fn add(&mut self, doc: Imported) {
        Arc::make_mut(&mut self.docs).push(Arc::new(doc));
        self.generation += 1;
    }

    /// The contents as they are now, for another thread.
    pub fn snapshot(&self) -> Arc<Vec<Arc<Imported>>> {
        self.docs.clone()
    }

    /// The number of items in all files.
    pub fn item_count(&self) -> usize {
        self.docs.iter().map(|d| d.items.len()).sum()
    }

    /// The `index`th item over all files, with the file it came from.
    pub fn item(&self, mut index: usize) -> Option<(&Imported, &Item)> {
        for d in self.docs.iter() {
            if index < d.items.len() {
                return Some((d, &d.items[index]));
            }
            index -= d.items.len();
        }
        None
    }
}

/// Collects items under the limits.
#[derive(Default)]
struct Builder {
    items: Vec<Item>,
    points: usize,
    truncated: bool,
}

impl Builder {
    /// False once a limit is reached, so the caller can stop reading.
    fn add(&mut self, name: &str, shape: Shape) -> bool {
        let count = match &shape {
            Shape::Point(_) => 1,
            Shape::Line(l) => l.len(),
            Shape::Polygon(r) => r.iter().map(Vec::len).sum(),
        };
        if self.items.len() >= MAX_ITEMS || self.points + count > MAX_POINTS {
            self.truncated = true;
            return false;
        }
        self.points += count;
        self.items.push(Item::new(name, shape));
        true
    }

    fn finish(self, name: &str) -> Result<Imported, ImportError> {
        if self.items.is_empty() {
            return Err(ImportError::Empty);
        }
        Ok(Imported { name: clean(name), items: self.items, truncated: self.truncated })
    }
}

fn clean(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_NAME_CHARS).collect()
}

/// Reads a file from disk. `name` for the result comes from the file's name.
pub fn load_file(path: &Path) -> Result<Imported, ImportError> {
    let io = |e: std::io::Error| ImportError::Io(e.to_string());
    let file = std::fs::File::open(path).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(ImportError::Io("not a file".into()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    parse(&bytes, &stem)
}

/// Works out the format from the contents, and reads it. `name` names the result.
pub fn parse(bytes: &[u8], name: &str) -> Result<Imported, ImportError> {
    if bytes.starts_with(b"PK\x03\x04") {
        return kmz(bytes, name);
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with('{') {
        geojson(text, name)
    } else if text.starts_with('<') {
        match xml_root(text).as_deref() {
            Some("gpx") => gpx(text, name),
            Some("kml") => kml(text, name),
            _ => Err(ImportError::Unrecognised),
        }
    } else if text.starts_with('A') && text.lines().any(|l| l.starts_with('B')) {
        igc(text, name)
    } else {
        Err(ImportError::Unrecognised)
    }
}

/// The name of the first element.
fn xml_root(text: &str) -> Option<String> {
    let mut reader = Reader::from_str(text);
    for _ in 0..64 {
        match reader.read_event().ok()? {
            Event::Start(e) | Event::Empty(e) => {
                return Some(e.local_name().as_ref().to_ascii_lowercase());
            }
            Event::Eof => return None,
            _ => {}
        }
    }
    None
}

// ---- GeoJSON ----

fn position(v: &Value) -> Option<Pos> {
    let a = v.as_array()?;
    Pos::from_degrees(a.get(1)?.as_f64()?, a.first()?.as_f64()?, a.get(2).and_then(Value::as_f64))
}

fn positions(v: &Value) -> Vec<Pos> {
    v.as_array().map(|a| a.iter().filter_map(position).collect()).unwrap_or_default()
}

fn geojson_geometry(g: &Value, name: &str, out: &mut Builder, depth: usize) -> bool {
    let coords = g.get("coordinates");
    let empty = Value::Null;
    let coords = coords.unwrap_or(&empty);
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    match g.get("type").and_then(Value::as_str) {
        Some("Point") => position(coords).is_none_or(|p| out.add(name, Shape::Point(p))),
        Some("MultiPoint") => positions(coords).into_iter().all(|p| out.add(name, Shape::Point(p))),
        Some("LineString") => {
            let l = positions(coords);
            l.len() < 2 || out.add(name, Shape::Line(l))
        }
        Some("MultiLineString") => list(coords).iter().map(positions).all(|l| l.len() < 2 || out.add(name, Shape::Line(l))),
        Some("Polygon") => {
            let rings: Vec<_> = list(coords).iter().map(positions).filter(|r| r.len() >= 3).collect();
            rings.is_empty() || out.add(name, Shape::Polygon(rings))
        }
        Some("MultiPolygon") => list(coords).iter().all(|poly| {
            let rings: Vec<_> = list(poly).iter().map(positions).filter(|r| r.len() >= 3).collect();
            rings.is_empty() || out.add(name, Shape::Polygon(rings))
        }),
        Some("GeometryCollection") if depth < MAX_GEOMETRY_DEPTH => {
            let parts = g.get("geometries").and_then(Value::as_array);
            parts.is_none_or(|p| p.iter().all(|g| geojson_geometry(g, name, out, depth + 1)))
        }
        _ => true,
    }
}

fn feature_name(f: &Value) -> String {
    let props = f.get("properties");
    for key in ["name", "Name", "title", "label"] {
        if let Some(s) = props.and_then(|p| p.get(key)).and_then(Value::as_str) {
            return s.to_string();
        }
    }
    String::new()
}

fn geojson_object(v: &Value, out: &mut Builder, depth: usize) -> bool {
    if depth > MAX_GEOMETRY_DEPTH {
        return true;
    }
    match v.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => {
            let features = v.get("features").and_then(Value::as_array);
            features.is_none_or(|f| f.iter().all(|f| geojson_object(f, out, depth + 1)))
        }
        Some("Feature") => match v.get("geometry") {
            Some(g) if g.is_object() => geojson_geometry(g, &feature_name(v), out, 0),
            _ => true,
        },
        _ => geojson_geometry(v, "", out, 0),
    }
}

pub fn geojson(text: &str, name: &str) -> Result<Imported, ImportError> {
    let doc: Value = serde_json::from_str(text).map_err(|e| ImportError::Malformed(e.to_string()))?;
    if !doc.is_object() {
        return Err(ImportError::Malformed("not an object".into()));
    }
    let mut out = Builder::default();
    geojson_object(&doc, &mut out, 0);
    out.finish(name)
}

// ---- XML ----

pub(crate) enum Tok {
    Open(String, Vec<(String, String)>),
    Close(String),
    Text(String),
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Streams the elements and text of an XML document to `f`, which says whether to go on.
/// Names have their prefix removed. Entities other than the predefined and numeric ones are
/// dropped, so nothing is expanded.
pub(crate) fn walk(text: &str, mut f: impl FnMut(Tok) -> bool) -> Result<(), ImportError> {
    let bad = |e: &dyn std::fmt::Display| ImportError::Malformed(e.to_string());
    let mut reader = Reader::from_str(text);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0usize;
    let mut pending = String::new();
    let mut seen_root = false;
    loop {
        let event = reader.read_event().map_err(|e| bad(&e))?;
        let is_text = matches!(event, Event::Text(_) | Event::CData(_) | Event::GeneralRef(_));
        if !is_text && !pending.is_empty() && !f(Tok::Text(std::mem::take(&mut pending))) {
            return Ok(());
        }
        match event {
            Event::Start(e) => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err(ImportError::Malformed("elements nested too deeply".into()));
                }
                seen_root = true;
                let name = e.local_name().as_ref().to_string();
                let mut attrs = Vec::new();
                for a in e.attributes().flatten() {
                    let key = a.key.local_name().as_ref().to_string();
                    if let Ok(v) = a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                        attrs.push((key, v.into_owned()));
                    }
                }
                if !f(Tok::Open(name, attrs)) {
                    return Ok(());
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                let name = e.local_name().as_ref().to_string();
                if !f(Tok::Close(name)) {
                    return Ok(());
                }
            }
            Event::Text(t) => pending.push_str(&t.xml10_content()),
            Event::CData(t) => pending.push_str(&t.xml10_content()),
            Event::GeneralRef(r) => pending.extend(entity(&r)),
            Event::Eof => break,
            _ => {}
        }
    }
    if seen_root && depth != 0 {
        return Err(ImportError::Malformed("the file ends inside an element".into()));
    }
    if !seen_root {
        return Err(ImportError::Malformed("no elements".into()));
    }
    Ok(())
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|a| a.0 == key).map(|a| a.1.as_str())
}

fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

// ---- GPX ----

/// GPX 1.1: waypoints, and tracks and routes as lines. Each track segment is one line.
pub fn gpx(text: &str, name: &str) -> Result<Imported, ImportError> {
    let mut out = Builder::default();
    let mut path: Vec<String> = Vec::new();
    let (mut wpt_name, mut container_name) = (String::new(), String::new());
    let mut point: Option<(f64, f64)> = None;
    let mut ele: Option<f64> = None;
    let mut line: Vec<Pos> = Vec::new();
    walk(text, |tok| {
        match tok {
            Tok::Open(n, attrs) => {
                match n.as_str() {
                    "wpt" | "trkpt" | "rtept" => {
                        let lat = attr(&attrs, "lat").and_then(number);
                        let lon = attr(&attrs, "lon").and_then(number);
                        point = lat.zip(lon);
                        ele = None;
                    }
                    "trk" | "rte" => {
                        container_name.clear();
                        line.clear();
                    }
                    "trkseg" => line.clear(),
                    _ => {}
                }
                if n == "wpt" {
                    wpt_name.clear();
                }
                path.push(n);
            }
            Tok::Text(t) => {
                let n = path.len();
                if n >= 2 && path[n - 1] == "name" {
                    match path[n - 2].as_str() {
                        "wpt" => wpt_name = t,
                        "trk" | "rte" => container_name = t,
                        _ => {}
                    }
                } else if n >= 1 && path[n - 1] == "ele" {
                    ele = number(&t);
                }
            }
            Tok::Close(n) => {
                path.pop();
                let pos = point.and_then(|(lat, lon)| Pos::from_degrees(lat, lon, ele));
                match n.as_str() {
                    "wpt" => {
                        point = None;
                        if let Some(p) = pos {
                            return out.add(&wpt_name, Shape::Point(p));
                        }
                    }
                    "trkpt" | "rtept" => {
                        point = None;
                        line.extend(pos);
                    }
                    "trkseg" | "rte" if line.len() >= 2 => {
                        return out.add(&container_name, Shape::Line(std::mem::take(&mut line)));
                    }
                    _ => {}
                }
            }
        }
        true
    })?;
    out.finish(name)
}

// ---- KML ----

fn coordinates(text: &str) -> Vec<Pos> {
    text.split_whitespace()
        .filter_map(|tuple| {
            let mut parts = tuple.split(',');
            let lon = number(parts.next()?)?;
            let lat = number(parts.next()?)?;
            let alt = parts.next().and_then(number);
            Pos::from_degrees(lat, lon, alt)
        })
        .collect()
}

/// KML 2.2 placemarks: Point, LineString, LinearRing, Polygon and MultiGeometry, in any
/// nesting of Folders and Documents. Styles, regions and time spans are not read.
pub fn kml(text: &str, name: &str) -> Result<Imported, ImportError> {
    let mut out = Builder::default();
    let mut path: Vec<String> = Vec::new();
    let mut in_placemark = false;
    let mut pm_name = String::new();
    let mut shapes: Vec<Shape> = Vec::new();
    let mut rings: Vec<Vec<Pos>> = Vec::new();
    walk(text, |tok| {
        match tok {
            Tok::Open(n, _) => {
                if n == "Placemark" {
                    in_placemark = true;
                    pm_name.clear();
                    shapes.clear();
                    rings.clear();
                }
                if n == "Polygon" {
                    rings.clear();
                }
                path.push(n);
            }
            Tok::Text(t) => {
                let n = path.len();
                if !in_placemark || n < 2 {
                    return true;
                }
                if path[n - 1] == "name" && path[n - 2] == "Placemark" {
                    pm_name = t;
                } else if path[n - 1] == "coordinates" {
                    let list = coordinates(&t);
                    match path[n - 2].as_str() {
                        "Point" => shapes.extend(list.first().map(|p| Shape::Point(*p))),
                        "LineString" if list.len() >= 2 => shapes.push(Shape::Line(list)),
                        "LinearRing" if list.len() >= 3 => {
                            let parent = n.checked_sub(3).map(|i| path[i].as_str());
                            match parent {
                                Some("outerBoundaryIs") => rings.insert(0, list),
                                Some("innerBoundaryIs") => rings.push(list),
                                _ => shapes.push(Shape::Line(list)),
                            }
                        }
                        _ => {}
                    }
                }
            }
            Tok::Close(n) => {
                path.pop();
                match n.as_str() {
                    "Polygon" if !rings.is_empty() => shapes.push(Shape::Polygon(std::mem::take(&mut rings))),
                    "Placemark" => {
                        in_placemark = false;
                        for s in shapes.drain(..) {
                            if !out.add(&pm_name, s) {
                                return false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        true
    })?;
    out.finish(name)
}

// ---- KMZ ----

/// A zip holding a KML: `doc.kml` if there is one, otherwise the first `.kml`. Entries are
/// read in memory only, so their names cannot place anything on disk, and no more than
/// `MAX_FILE_BYTES` are taken from one, whatever it claims.
pub fn kmz(bytes: &[u8], name: &str) -> Result<Imported, ImportError> {
    let bad = |e: zip::result::ZipError| ImportError::Malformed(e.to_string());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(bad)?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(ImportError::Malformed("too many entries".into()));
    }
    let mut chosen: Option<usize> = None;
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(i) else { continue };
        // Names that would climb out of a folder are never used.
        if entry.is_dir() || entry.enclosed_name().is_none() {
            continue;
        }
        let lower = entry.name().map(|n| n.to_ascii_lowercase()).unwrap_or_default();
        if lower == "doc.kml" {
            chosen = Some(i);
            break;
        }
        if lower.ends_with(".kml") && chosen.is_none() {
            chosen = Some(i);
        }
    }
    let index = chosen.ok_or_else(|| ImportError::Malformed("no KML inside".into()))?;
    let entry = archive.by_index(index).map_err(bad)?;
    if entry.size() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let mut data = Vec::new();
    entry.take(MAX_FILE_BYTES + 1).read_to_end(&mut data).map_err(|e| ImportError::Malformed(e.to_string()))?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let text = String::from_utf8_lossy(&data);
    kml(text.trim_start_matches('\u{feff}'), name)
}

// ---- IGC ----

/// Degrees from `DDMMmmm` or `DDDMMmmm` and a hemisphere letter.
fn igc_angle(digits: &str, degree_digits: usize, hemisphere: char, plus: char, minus: char) -> Option<f64> {
    if digits.len() != degree_digits + 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let degrees: f64 = digits[..degree_digits].parse().ok()?;
    let minutes: f64 = digits[degree_digits..].parse::<f64>().ok()? / 1000.0;
    if minutes >= 60.0 {
        return None;
    }
    let value = degrees + minutes / 60.0;
    match hemisphere {
        h if h == plus => Some(value),
        h if h == minus => Some(-value),
        _ => None,
    }
}

/// The text after the first colon of an H record, or after the code if there is none.
fn h_value(line: &str, code_len: usize) -> String {
    match line.split_once(':') {
        Some((_, v)) => v.trim().to_string(),
        None => line.get(code_len..).unwrap_or("").trim().to_string(),
    }
}

/// IGC flight logs (FAI Technical Specification for IGC-approved GNSS flight recorders):
/// the B records make one line, with the GNSS altitude. The pilot and glider type from the
/// H records, and the date, make the name.
pub fn igc(text: &str, name: &str) -> Result<Imported, ImportError> {
    let (mut pilot, mut glider, mut date) = (String::new(), String::new(), String::new());
    let mut line: Vec<Pos> = Vec::new();
    for record in text.lines() {
        let record = record.trim_end();
        let Some(kind) = record.chars().next() else { continue };
        match kind {
            'H' if record.len() >= 5 => match record.get(2..5) {
                Some("PLT") => pilot = h_value(record, 5),
                Some("GTY") => glider = h_value(record, 5),
                Some("DTE") => {
                    // "HFDTE020716" or "HFDTEDATE:020716,01".
                    let digits: String = h_value(record, 5).chars().take_while(char::is_ascii_digit).collect();
                    if digits.len() >= 6 {
                        let (d, m, y) = (&digits[0..2], &digits[2..4], &digits[4..6]);
                        let century = if y.parse::<u32>().unwrap_or(0) < 80 { "20" } else { "19" };
                        date = format!("{century}{y}-{m}-{d}");
                    }
                }
                _ => {}
            },
            'B' if record.is_ascii() && record.len() >= 35 => {
                let lat = igc_angle(&record[7..14], 2, record.as_bytes()[14] as char, 'N', 'S');
                let lon = igc_angle(&record[15..23], 3, record.as_bytes()[23] as char, 'E', 'W');
                let alt = record[30..35].parse::<f64>().ok();
                if let (Some(lat), Some(lon)) = (lat, lon) {
                    if line.len() >= MAX_POINTS {
                        break;
                    }
                    line.extend(Pos::from_degrees(lat, lon, alt));
                }
            }
            _ => {}
        }
    }
    let mut parts: Vec<&str> = [pilot.as_str(), glider.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        parts.push("Flight");
    }
    let mut title = parts.join(", ");
    if !date.is_empty() {
        title.push_str(&format!(" ({date})"));
    }
    let mut out = Builder::default();
    if line.len() >= 2 {
        out.add(&title, Shape::Line(line));
    }
    out.finish(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GPX: &str = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
  <wpt lat="46.5" lon="11.8"><ele>2000.5</ele><name>Hut &amp; view</name></wpt>
  <wpt lat="95" lon="11.8"><name>Bad</name></wpt>
  <trk><name>Ridge</name>
    <trkseg>
      <trkpt lat="46.0" lon="11.0"><ele>1000</ele></trkpt>
      <trkpt lat="46.1" lon="11.1"/>
      <trkpt lat="46.2" lon="x"/>
    </trkseg>
    <trkseg><trkpt lat="47" lon="12"/></trkseg>
  </trk>
  <rte><name>Road</name><rtept lat="45" lon="10"/><rtept lat="45.5" lon="10.5"/></rte>
</gpx>"#;

    #[test]
    fn gpx_gives_waypoints_tracks_and_routes() {
        let g = gpx(GPX, "f").unwrap();
        assert_eq!(g.items.len(), 3, "{:?}", g.items);
        assert_eq!(g.items[0].name, "Hut & view");
        let Shape::Point(p) = g.items[0].shape else { panic!() };
        assert!((p.lat.to_degrees() - 46.5).abs() < 1e-9 && p.alt == Some(2000.5));
        let Shape::Line(l) = &g.items[1].shape else { panic!() };
        assert_eq!((g.items[1].name.as_str(), l.len()), ("Ridge", 2), "the bad point and the one-point segment are dropped");
        assert_eq!(l[0].alt, Some(1000.0));
        assert_eq!(g.items[2].name, "Road");
    }

    #[test]
    fn gpx_that_is_broken_is_an_error_not_a_panic() {
        assert!(matches!(gpx("<gpx><wpt lat=\"1\" lon=\"2\">", "f"), Err(ImportError::Malformed(_))));
        assert!(matches!(gpx("<gpx></wpt></gpx>", "f"), Err(ImportError::Malformed(_))));
        assert_eq!(gpx("<gpx/>", "f"), Err(ImportError::Empty));
        assert!(gpx("", "f").is_err());
        let deep = "<a>".repeat(500);
        assert!(matches!(gpx(&deep, "f"), Err(ImportError::Malformed(_))));
    }

    #[test]
    fn entities_are_not_expanded() {
        let t = r#"<!DOCTYPE gpx [<!ENTITY big "AAAAAAAAAAAAAAAA">]><gpx><wpt lat="1" lon="2"><name>a&big;b&#65;&#x42;</name></wpt></gpx>"#;
        let g = gpx(t, "f").unwrap();
        assert_eq!(g.items[0].name, "abAB");
    }

    const KML: &str = r#"<?xml version="1.0"?>
<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2">
<Document><name>Doc</name>
 <Folder><name>Folder</name>
  <Placemark><name>Peak</name><Point><coordinates>11.8,46.5,3000</coordinates></Point></Placemark>
  <Placemark><name>Trail</name><LineString><coordinates>
     11.0,46.0,0 11.1,46.1,10
     11.2,46.2 </coordinates></LineString></Placemark>
  <Placemark><name>Meadow</name><Polygon>
    <outerBoundaryIs><LinearRing><coordinates>10,45,0 11,45,0 11,46,0 10,45,0</coordinates></LinearRing></outerBoundaryIs>
    <innerBoundaryIs><LinearRing><coordinates>10.4,45.2 10.6,45.2 10.5,45.4 10.4,45.2</coordinates></LinearRing></innerBoundaryIs>
  </Polygon></Placemark>
  <Placemark><name>Both</name><MultiGeometry>
     <Point><coordinates>1,2</coordinates></Point>
     <LineString><coordinates>1,2 3,4</coordinates></LineString>
  </MultiGeometry></Placemark>
 </Folder>
</Document></kml>"#;

    #[test]
    fn kml_gives_each_geometry_with_its_placemark_name() {
        let k = kml(KML, "f").unwrap();
        let names: Vec<_> = k.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Peak", "Trail", "Meadow", "Both", "Both"]);
        let Shape::Point(p) = k.items[0].shape else { panic!() };
        assert_eq!(p.alt, Some(3000.0));
        let Shape::Line(l) = &k.items[1].shape else { panic!() };
        assert_eq!(l.len(), 3);
        assert_eq!(l[2].alt, None);
        let Shape::Polygon(r) = &k.items[2].shape else { panic!() };
        assert_eq!((r.len(), r[0].len(), r[1].len()), (2, 4, 4), "outer ring first, then the hole");
        assert!(matches!(k.items[4].shape, Shape::Line(_)));
    }

    #[test]
    fn kml_malformed_input_is_an_error_or_less_data() {
        assert!(kml("<kml><Placemark><name>x", "f").is_err());
        assert_eq!(kml("<kml><Document/></kml>", "f"), Err(ImportError::Empty));
        let t = "<kml><Placemark><Point><coordinates>abc,def 200,100 1,2</coordinates></Point></Placemark></kml>";
        let k = kml(t, "f").unwrap();
        assert_eq!(k.items.len(), 1, "bad tuples are skipped: {:?}", k.items);
        let Shape::Point(p) = k.items[0].shape else { panic!() };
        assert!((p.lon.to_degrees() - 1.0).abs() < 1e-9);
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut cursor);
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        cursor.into_inner()
    }

    #[test]
    fn kmz_reads_the_kml_inside() {
        let z = make_zip(&[("images/a.png", b"x"), ("../evil.kml", b"<kml/>"), ("doc.kml", KML.as_bytes())]);
        let k = parse(&z, "f").unwrap();
        assert_eq!(k.items.len(), 5);
        let z = make_zip(&[("a/other.KML", KML.as_bytes())]);
        assert_eq!(parse(&z, "f").unwrap().items.len(), 5);
    }

    #[test]
    fn kmz_without_usable_content_is_an_error() {
        assert!(matches!(parse(&make_zip(&[("a.txt", b"x")]), "f"), Err(ImportError::Malformed(_))));
        assert!(matches!(parse(&make_zip(&[("../x.kml", KML.as_bytes())]), "f"), Err(ImportError::Malformed(_))));
        assert!(parse(b"PK\x03\x04garbage", "f").is_err());
    }

    #[test]
    fn kmz_decompression_is_bounded() {
        // About 70 MB of zeros compresses to a few kilobytes.
        let big = vec![b' '; (MAX_FILE_BYTES + 1000) as usize];
        let z = make_zip(&[("doc.kml", &big)]);
        assert!(z.len() < 1 << 20);
        assert_eq!(parse(&z, "f"), Err(ImportError::TooLarge));
    }

    const GEOJSON: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"name":"A"},"geometry":{"type":"Point","coordinates":[11.8,46.5,2000]}},
      {"type":"Feature","properties":{"title":"B"},"geometry":{"type":"LineString","coordinates":[[11,46],[12,47],[12.5,"x"]]}},
      {"type":"Feature","properties":null,"geometry":{"type":"Polygon","coordinates":[[[10,45],[11,45],[11,46],[10,45]]]}},
      {"type":"Feature","properties":{},"geometry":null},
      {"type":"Feature","properties":{"name":"M"},"geometry":{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}},
      {"type":"Feature","properties":{"name":"G"},"geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[5,6]}]}}
    ]}"#;

    #[test]
    fn geojson_gives_points_lines_and_polygons() {
        let g = geojson(GEOJSON, "f").unwrap();
        let names: Vec<_> = g.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "", "M", "M", "G"]);
        let Shape::Point(p) = g.items[0].shape else { panic!() };
        assert!((p.lat.to_degrees() - 46.5).abs() < 1e-9 && (p.lon.to_degrees() - 11.8).abs() < 1e-9, "longitude comes first");
        assert_eq!(p.alt, Some(2000.0));
        let Shape::Line(l) = &g.items[1].shape else { panic!() };
        assert_eq!(l.len(), 2);
        assert!(matches!(g.items[2].shape, Shape::Polygon(_)));
    }

    #[test]
    fn geojson_accepts_a_bare_geometry_and_rejects_nonsense() {
        let g = geojson(r#"{"type":"Point","coordinates":[1,2]}"#, "f").unwrap();
        assert_eq!(g.items.len(), 1);
        for t in ["{", "[]", r#"{"type":"FeatureCollection"}"#, r#"{"type":"Point","coordinates":[999,2]}"#, r#"{"type":"Nope"}"#] {
            assert!(geojson(t, "f").is_err(), "{t}");
        }
        let nested = format!("{}{}", r#"{"type":"GeometryCollection","geometries":["#.repeat(50), "]}".repeat(50));
        assert!(geojson(&nested, "f").is_err());
    }

    const IGC: &str = "AXCSAAA\r\n\
HFDTE020716\r\n\
HFPLTPILOTINCHARGE:Jane Doe\r\n\
HFGTYGLIDERTYPE:Example 3\r\n\
B1101355206343N00006198WA0058700558\r\n\
B1101455206259N00006295WA0059300573\r\n\
B1101552506259S00006295WV0059300573\r\n\
B110165206259N0000629\r\n\
LXXX ignored\r\n";

    #[test]
    fn igc_gives_one_line_named_after_pilot_and_glider() {
        let g = igc(IGC, "f").unwrap();
        assert_eq!(g.items.len(), 1);
        assert_eq!(g.items[0].name, "Jane Doe, Example 3 (2016-07-02)");
        let Shape::Line(l) = &g.items[0].shape else { panic!() };
        assert_eq!(l.len(), 3, "the short record is skipped");
        assert!((l[0].lat.to_degrees() - (52.0 + 6.343 / 60.0)).abs() < 1e-9);
        assert!((l[0].lon.to_degrees() + (6.198 / 60.0)).abs() < 1e-9, "west is negative");
        assert_eq!(l[0].alt, Some(558.0), "the GNSS altitude");
        assert!(l[2].lat < 0.0, "south is negative");
    }

    #[test]
    fn igc_without_fixes_or_with_nonsense_is_an_error() {
        assert_eq!(igc("AXXX\r\nHFDTE020716\r\n", "f"), Err(ImportError::Empty));
        assert!(igc("B\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}", "f").is_err());
        let bad = "AXXX\nB1101359999999N00006198WA0058700558\nB1101355206343N00006198WA0058700558\n";
        assert!(igc(bad, "f").is_err(), "one good fix is not a line");
    }

    #[test]
    fn the_format_is_found_from_the_contents() {
        assert!(parse(GPX.as_bytes(), "f").is_ok());
        assert!(parse(KML.as_bytes(), "f").is_ok());
        assert!(parse(GEOJSON.as_bytes(), "f").is_ok());
        assert!(parse(IGC.as_bytes(), "f").is_ok());
        let bom = format!("\u{feff}{GEOJSON}");
        assert!(parse(bom.as_bytes(), "f").is_ok());
        for t in [&b""[..], b"hello", b"<html></html>", b"\xff\xfe\x00\x01", b"<"] {
            assert_eq!(parse(t, "f"), Err(ImportError::Unrecognised), "{t:?}");
        }
    }

    #[test]
    fn the_library_numbers_items_across_files() {
        let mut lib = Library::default();
        assert!(lib.item(0).is_none());
        lib.add(gpx(GPX, "one").unwrap());
        lib.add(kml(KML, "two").unwrap());
        assert_eq!((lib.item_count(), lib.generation), (8, 2));
        assert_eq!(lib.item(2).unwrap().0.name, "one");
        assert_eq!(lib.item(3).unwrap().1.name, "Peak");
        assert!(lib.item(8).is_none());
    }

    #[test]
    fn the_fixtures_in_the_repository_read() {
        let g = load_file(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/alps_route.gpx"))).unwrap();
        assert_eq!(g.items.len(), 7);
        let k = load_file(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/alps_areas.kml"))).unwrap();
        assert!(matches!(k.items[0].shape, Shape::Polygon(ref r) if r.len() == 2));
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean(" a\n\tb  c "), "a b c");
        assert_eq!(clean(&"x".repeat(1000)).chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn many_items_are_cut_short_and_reported() {
        let mut t = String::from("<gpx>");
        for i in 0..(MAX_ITEMS + 5) {
            t.push_str(&format!("<wpt lat=\"1\" lon=\"{}\"/>", (i % 100) as f64 / 10.0));
        }
        t.push_str("</gpx>");
        let g = gpx(&t, "f").unwrap();
        assert!(g.truncated && g.items.len() == MAX_ITEMS);
    }

    #[test]
    fn boxes_and_targets_cross_the_antimeridian_the_short_way() {
        let r = |d: f64| d.to_radians();
        let b = bounding_box([(r(1.0), r(170.0)), (r(2.0), r(-175.0))].into_iter()).unwrap();
        assert!((b[1].to_degrees() - 170.0).abs() < 1e-9 && (b[3].to_degrees() + 175.0).abs() < 1e-9);
        let b = bounding_box([(r(1.0), r(10.0)), (r(2.0), r(20.0)), (r(3.0), r(15.0))].into_iter()).unwrap();
        assert!((b[1].to_degrees() - 10.0).abs() < 1e-9 && (b[3].to_degrees() - 20.0).abs() < 1e-9);
        assert!(bounding_box(std::iter::empty()).is_none());
        let g = gpx(GPX, "f").unwrap();
        let t = g.target().unwrap();
        assert!(t.lat.to_degrees() > 45.0 && t.lat.to_degrees() < 47.0 && t.distance > 10_000.0);
        let single = g.items[0].target().unwrap();
        assert!((single.distance - 4000.0).abs() < 1.0);
    }

    #[test]
    fn files_are_read_from_disk_with_limits() {
        let dir = std::env::temp_dir().join(format!("orbis-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("walk.gpx");
        std::fs::write(&path, GPX).unwrap();
        assert_eq!(load_file(&path).unwrap().name, "walk");
        assert!(matches!(load_file(&dir), Err(ImportError::Io(_))));
        assert!(matches!(load_file(&dir.join("none.gpx")), Err(ImportError::Io(_))));
    }
}
