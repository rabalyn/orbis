//! Decodes Mapbox Vector Tiles in the OpenMapTiles schema into what we draw. Tiles come from
//! the network, so malformed input must yield an error or partial data, never a panic.

use crate::geo::TileKey;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoadClass {
    Path,
    Service,
    Minor,
    Tertiary,
    Secondary,
    Primary,
    Trunk,
    Motorway,
    Rail,
    Ferry,
}

impl RoadClass {
    #[cfg(test)]
    pub const ALL: [RoadClass; 10] = [
        Self::Path,
        Self::Service,
        Self::Minor,
        Self::Tertiary,
        Self::Secondary,
        Self::Primary,
        Self::Trunk,
        Self::Motorway,
        Self::Rail,
        Self::Ferry,
    ];

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "motorway" => Self::Motorway,
            "trunk" => Self::Trunk,
            "primary" => Self::Primary,
            "secondary" => Self::Secondary,
            "tertiary" => Self::Tertiary,
            "minor" | "busway" => Self::Minor,
            "service" | "raceway" => Self::Service,
            "track" | "path" => Self::Path,
            "rail" | "transit" => Self::Rail,
            "ferry" => Self::Ferry,
            _ => return None,
        })
    }
}

pub type Line = Vec<[f32; 2]>;

const MIN_PEAK_Z: u8 = 9;

#[derive(Debug, Clone, PartialEq)]
pub struct Road {
    pub class: RoadClass,
    pub tunnel: bool,
    /// In tile coordinates, 0..1.
    pub line: Line,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Boundary {
    pub admin_level: i64,
    pub disputed: bool,
    pub line: Line,
}

/// One building's footprint and extent. Features in the data merge many buildings with
/// equal attributes, so this is one polygon of such a feature.
#[derive(Debug, Clone, PartialEq)]
pub struct Building {
    /// The closed outer ring, then any closed holes, in tile coordinates.
    pub rings: Vec<Line>,
    /// Top of the building above the ground, in metres.
    pub height: f32,
    /// Where the building starts above the ground, in metres. Above zero it is a raised part.
    pub min_height: f32,
    /// sRGB colour, when the data has one.
    pub colour: Option<[u8; 3]>,
    /// The zoned land use it stands in, if any.
    pub zone: Option<Zone>,
}

/// What land is zoned for, from the `landuse` layer's `class`, in the three kinds a city
/// builder knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Zone {
    Residential,
    Commercial,
    Industrial,
}

impl Zone {
    fn parse(class: &str) -> Option<Zone> {
        Some(match class {
            "residential" => Zone::Residential,
            "commercial" | "retail" => Zone::Commercial,
            "industrial" => Zone::Industrial,
            _ => return None,
        })
    }
}

/// A zoned area of a tile: its rings in tile coordinates, and their bounding box
/// (west, north, east, south).
struct ZoneArea {
    zone: Zone,
    rings: Vec<Line>,
    bbox: [f32; 4],
}

fn read_zones(layer: &Layer, out: &mut Vec<ZoneArea>) {
    let class_key = Fields { layer }.index("class");
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        if f.kind != 3 {
            continue;
        }
        let Some(Value::Text(class)) = get(layer, &f, class_key) else { continue };
        let Some(zone) = Zone::parse(class) else { continue };
        let rings: Vec<Line> = parse_geometry(f.geometry, layer.extent).into_iter().filter(|r| r.len() >= 4).collect();
        let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in rings.iter().flatten() {
            bbox = [bbox[0].min(p[0]), bbox[1].min(p[1]), bbox[2].max(p[0]), bbox[3].max(p[1])];
        }
        if !rings.is_empty() {
            out.push(ZoneArea { zone, rings, bbox });
        }
    }
}

/// Even-odd test over all rings, so that holes count.
fn in_rings(rings: &[Line], p: [f32; 2]) -> bool {
    let mut hit = false;
    for ring in rings {
        for w in ring.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                hit = !hit;
            }
        }
    }
    hit
}

/// Gives each building the zone its middle stands in. Where zones overlap, as shops within a
/// residential area, the smaller one is taken.
fn zone_buildings(zones: &[ZoneArea], buildings: &mut [Building]) {
    if zones.is_empty() {
        return;
    }
    for b in buildings {
        let Some(outer) = b.rings.first() else { continue };
        let corners = &outer[..outer.len().saturating_sub(1)];
        if corners.is_empty() {
            continue;
        }
        let n = corners.len() as f32;
        let (sx, sy) = corners.iter().fold((0.0, 0.0), |s, p| (s.0 + p[0], s.1 + p[1]));
        let p = [sx / n, sy / n];
        b.zone = zones
            .iter()
            .filter(|z| p[0] >= z.bbox[0] && p[1] >= z.bbox[1] && p[0] <= z.bbox[2] && p[1] <= z.bbox[3])
            .filter(|z| in_rings(&z.rings, p))
            .min_by(|a, b| {
                let size = |z: &ZoneArea| (z.bbox[2] - z.bbox[0]) * (z.bbox[3] - z.bbox[1]);
                size(a).total_cmp(&size(b))
            })
            .map(|z| z.zone);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LabelStyle {
    Country,
    State,
    City,
    Town,
    Village,
    Local,
    Sea,
    Lake,
    Peak,
    Road,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    pub style: LabelStyle,
    /// Lower comes first when labels compete for space.
    pub priority: i32,
    pub lat: f64,
    pub lon: f64,
    /// For labels along a line: a chord through the label's position, covering a fifth of
    /// the line's length, as (lat, lon) pairs.
    pub along: Option<[(f64, f64); 2]>,
}

#[derive(Default, Debug)]
pub struct VectorTile {
    pub roads: Vec<Road>,
    pub boundaries: Vec<Boundary>,
    pub buildings: Vec<Building>,
    pub labels: Arc<Vec<Label>>,
}

/// Coarser tiles only hold flattened outlines without heights.
pub const BUILDING_Z: u8 = 14;
const DEFAULT_BUILDING_HEIGHT: f64 = 6.0;
/// Taller than anything standing, so bad data cannot make a spike reaching into space.
const MAX_BUILDING_HEIGHT: f64 = 1000.0;

/// Reads a colour as hexadecimal or as one of the common CSS names used in the data.
fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().to_ascii_lowercase();
    if let Some(hex) = text.strip_prefix('#') {
        let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
        return match digits[..] {
            [r, g, b] => Some([r * 17, g * 17, b * 17]),
            [r1, r2, g1, g2, b1, b2] => Some([r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2]),
            _ => None,
        };
    }
    Some(match text.as_str() {
        "white" => [255, 255, 255],
        "black" => [40, 40, 40],
        "grey" | "gray" => [128, 128, 128],
        "lightgrey" | "lightgray" | "silver" => [200, 200, 200],
        "darkgrey" | "darkgray" => [100, 100, 100],
        "red" => [190, 60, 50],
        "darkred" | "maroon" => [125, 40, 35],
        "brown" => [140, 90, 60],
        "tan" => [210, 180, 140],
        "beige" => [235, 225, 200],
        "orange" => [225, 140, 60],
        "pink" => [235, 180, 190],
        "yellow" => [235, 215, 90],
        "lightyellow" | "ivory" => [250, 245, 205],
        "green" => [90, 140, 90],
        "lightgreen" => [170, 215, 160],
        "blue" => [80, 110, 170],
        "lightblue" => [170, 205, 225],
        _ => return None,
    })
}

/// The surveyor's formula. In tile coordinates, outer rings come out positive and holes negative.
fn signed_area(ring: &[[f32; 2]]) -> f64 {
    ring.windows(2)
        .map(|w| w[0][0] as f64 * w[1][1] as f64 - w[1][0] as f64 * w[0][1] as f64)
        .sum::<f64>()
        / 2.0
}

fn read_buildings(layer: &Layer, out: &mut Vec<Building>) {
    let fields = Fields { layer };
    let (height_key, min_key) = (fields.index("render_height"), fields.index("render_min_height"));
    let (colour_key, hide_key) = (fields.index("colour"), fields.index("hide_3d"));
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        // A hidden outline is already drawn in more detail by its building parts.
        if f.kind != 3 || get(layer, &f, hide_key).is_some_and(Value::truthy) {
            continue;
        }
        let number = |key| get(layer, &f, key).and_then(Value::number).filter(|n| n.is_finite());
        let height = number(height_key).unwrap_or(DEFAULT_BUILDING_HEIGHT).clamp(1.0, MAX_BUILDING_HEIGHT);
        let min_height = number(min_key).unwrap_or(0.0).clamp(0.0, MAX_BUILDING_HEIGHT);
        if min_height >= height {
            continue;
        }
        let colour = match get(layer, &f, colour_key) {
            Some(Value::Text(text)) => parse_colour(text),
            _ => None,
        };
        let mut current: Option<Building> = None;
        for ring in parse_geometry(f.geometry, layer.extent) {
            if ring.len() < 4 {
                continue;
            }
            let area = signed_area(&ring);
            if area > 0.0 {
                out.extend(current.take());
                current = Some(Building {
                    rings: vec![ring],
                    height: height as f32,
                    min_height: min_height as f32,
                    colour,
                    zone: None,
                });
            } else if area < 0.0 {
                if let Some(building) = &mut current {
                    building.rings.push(ring);
                }
            }
        }
        out.extend(current);
    }
}

/// Which name fields to try, best first.
pub fn name_fields(lang: &str) -> Vec<String> {
    const OTHER_SCRIPTS: &[&str] = &["ar", "be", "bg", "el", "fa", "he", "hi", "ja", "ka", "ko", "ru", "th", "uk", "zh"];
    let mut fields = Vec::new();
    if !lang.is_empty() {
        fields.push(format!("name:{lang}"));
        fields.push(format!("name_{lang}"));
    }
    if !OTHER_SCRIPTS.contains(&lang) {
        fields.push("name:latin".to_string());
    }
    fields.push("name".to_string());
    fields
}

/// The two-letter language in a locale such as `de_DE.UTF-8`; empty when there is none.
pub fn language_from_locale(locale: &str) -> String {
    let code: String = locale.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    match code.as_str() {
        "" | "C" | "POSIX" => String::new(),
        _ => code.to_ascii_lowercase(),
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

enum Wire<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
    Fixed64(u64),
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.data.get(self.pos)?;
            self.pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Some(value);
            }
        }
        None
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    /// The next field as (number, value). `None` at the end or on malformed data.
    fn field(&mut self) -> Option<(u64, Wire<'a>)> {
        if self.pos >= self.data.len() {
            return None;
        }
        let key = self.varint()?;
        let wire = match key & 7 {
            0 => Wire::Varint(self.varint()?),
            1 => Wire::Fixed64(u64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            2 => {
                let n = usize::try_from(self.varint()?).ok()?;
                Wire::Bytes(self.take(n)?)
            }
            5 => Wire::Fixed32(u32::from_le_bytes(self.take(4)?.try_into().ok()?)),
            _ => return None,
        };
        Some((key >> 3, wire))
    }
}

fn zigzag(n: u64) -> i64 {
    (n >> 1) as i64 ^ -((n & 1) as i64)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Value<'a> {
    Text(&'a str),
    Number(f64),
    Bool(bool),
    Other,
}

impl Value<'_> {
    fn text(self) -> Option<String> {
        match self {
            Value::Text(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            _ => None,
        }
    }

    fn number(self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(n),
            _ => None,
        }
    }

    fn truthy(self) -> bool {
        match self {
            Value::Bool(b) => b,
            Value::Number(n) => n != 0.0,
            _ => false,
        }
    }
}

fn parse_value(data: &[u8]) -> Value<'_> {
    let mut r = Reader::new(data);
    while let Some((n, w)) = r.field() {
        match (n, w) {
            (1, Wire::Bytes(b)) => return std::str::from_utf8(b).map_or(Value::Other, Value::Text),
            (2, Wire::Fixed32(v)) => return Value::Number(f32::from_bits(v) as f64),
            (3, Wire::Fixed64(v)) => return Value::Number(f64::from_bits(v)),
            (4, Wire::Varint(v)) => return Value::Number(v as i64 as f64),
            (5, Wire::Varint(v)) => return Value::Number(v as f64),
            (6, Wire::Varint(v)) => return Value::Number(zigzag(v) as f64),
            (7, Wire::Varint(v)) => return Value::Bool(v != 0),
            _ => {}
        }
    }
    Value::Other
}

struct Feature<'a> {
    kind: u64,
    tags: Vec<(u32, u32)>,
    geometry: &'a [u8],
}

struct Layer<'a> {
    name: &'a str,
    extent: f32,
    keys: Vec<&'a str>,
    values: Vec<Value<'a>>,
    features: Vec<&'a [u8]>,
}

fn parse_layer(data: &[u8]) -> Option<Layer<'_>> {
    let mut layer = Layer { name: "", extent: 4096.0, keys: Vec::new(), values: Vec::new(), features: Vec::new() };
    let mut r = Reader::new(data);
    while let Some((n, w)) = r.field() {
        match (n, w) {
            (1, Wire::Bytes(b)) => layer.name = std::str::from_utf8(b).ok()?,
            (2, Wire::Bytes(b)) => layer.features.push(b),
            (3, Wire::Bytes(b)) => layer.keys.push(std::str::from_utf8(b).ok()?),
            (4, Wire::Bytes(b)) => layer.values.push(parse_value(b)),
            (5, Wire::Varint(v)) if v > 0 => layer.extent = v as f32,
            _ => {}
        }
    }
    Some(layer)
}

fn parse_feature(data: &[u8]) -> Option<Feature<'_>> {
    let mut f = Feature { kind: 0, tags: Vec::new(), geometry: &[] };
    let mut r = Reader::new(data);
    while let Some((n, w)) = r.field() {
        match (n, w) {
            (2, Wire::Bytes(b)) => {
                let mut tags = Reader::new(b);
                while tags.pos < b.len() {
                    let key = tags.varint()?;
                    let value = tags.varint()?;
                    f.tags.push((u32::try_from(key).ok()?, u32::try_from(value).ok()?));
                }
            }
            (3, Wire::Varint(v)) => f.kind = v,
            (4, Wire::Bytes(b)) => f.geometry = b,
            _ => {}
        }
    }
    Some(f)
}

/// Decodes a geometry into its parts, in tile coordinates 0..1. The tile's edge buffer means
/// values may fall slightly outside that range.
fn parse_geometry(data: &[u8], extent: f32) -> Vec<Line> {
    let mut parts: Vec<Line> = Vec::new();
    let mut r = Reader::new(data);
    let (mut x, mut y) = (0i64, 0i64);
    while r.pos < data.len() {
        let Some(command) = r.varint() else { break };
        let (id, count) = (command & 7, command >> 3);
        match id {
            1 | 2 => {
                for _ in 0..count {
                    let (Some(dx), Some(dy)) = (r.varint(), r.varint()) else {
                        return parts;
                    };
                    x = x.wrapping_add(zigzag(dx));
                    y = y.wrapping_add(zigzag(dy));
                    let point = [x as f32 / extent, y as f32 / extent];
                    if id == 1 || parts.is_empty() {
                        parts.push(vec![point]);
                    } else if let Some(part) = parts.last_mut() {
                        part.push(point);
                    }
                }
            }
            7 => {
                if let Some(first) = parts.last().and_then(|p| p.first().copied()) {
                    if let Some(part) = parts.last_mut() {
                        part.push(first);
                    }
                }
            }
            _ => return parts,
        }
    }
    parts
}

/// Fast property lookup for one layer: the key index of each field we ask for.
struct Fields<'l, 'a> {
    layer: &'l Layer<'a>,
}

impl Fields<'_, '_> {
    fn index(&self, name: &str) -> Option<u32> {
        self.layer.keys.iter().position(|k| *k == name).map(|i| i as u32)
    }
}

fn get<'a>(layer: &Layer<'a>, f: &Feature, key: Option<u32>) -> Option<Value<'a>> {
    let key = key?;
    let (_, value) = f.tags.iter().find(|t| t.0 == key)?;
    layer.values.get(*value as usize).copied()
}

fn inside(p: [f32; 2]) -> bool {
    (0.0..1.0).contains(&p[0]) && (0.0..1.0).contains(&p[1])
}

fn point_to_lat_lon(key: TileKey, p: [f32; 2]) -> (f64, f64) {
    key.lat_lon(p[0] as f64, p[1] as f64)
}

/// The position at a fraction of a polyline's length.
fn along_line(line: &[[f32; 2]], fraction: f32) -> [f32; 2] {
    let total: f32 = line.windows(2).map(|w| distance(w[0], w[1])).sum();
    let mut left = total * fraction;
    for w in line.windows(2) {
        let d = distance(w[0], w[1]);
        if left <= d && d > 0.0 {
            let t = left / d;
            return [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
        }
        left -= d;
    }
    *line.last().unwrap_or(&[0.0, 0.0])
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Decodes a tile. `None` means it is not a readable vector tile at all.
pub fn decode(data: &[u8], key: TileKey, names: &[String]) -> Option<VectorTile> {
    let mut tile = VectorTile::default();
    let mut labels = Vec::new();
    let mut zones = Vec::new();
    let mut r = Reader::new(data);
    while r.pos < data.len() {
        let (n, wire) = r.field()?;
        let (3, Wire::Bytes(bytes)) = (n, wire) else { continue };
        let Some(layer) = parse_layer(bytes) else { continue };
        match layer.name {
            "transportation" => read_roads(&layer, &mut tile.roads),
            "boundary" => read_boundaries(&layer, &mut tile.boundaries),
            "building" if key.z >= BUILDING_Z => read_buildings(&layer, &mut tile.buildings),
            "landuse" if key.z >= BUILDING_Z => read_zones(&layer, &mut zones),
            "place" => read_places(&layer, key, names, &mut labels),
            "water_name" => read_water_names(&layer, key, names, &mut labels),
            "mountain_peak" => read_peaks(&layer, key, names, &mut labels),
            "transportation_name" => read_road_names(&layer, key, names, &mut labels),
            _ => {}
        }
    }
    zone_buildings(&zones, &mut tile.buildings);
    tile.labels = Arc::new(labels);
    Some(tile)
}

fn first_name<'a>(layer: &Layer<'a>, f: &Feature, fields: &[Option<u32>]) -> Option<String> {
    fields.iter().find_map(|k| get(layer, f, *k)?.text())
}

fn name_indices(layer: &Layer, names: &[String]) -> Vec<Option<u32>> {
    let fields = Fields { layer };
    names.iter().map(|n| fields.index(n)).collect()
}

fn read_roads(layer: &Layer, out: &mut Vec<Road>) {
    let fields = Fields { layer };
    let (class_key, brunnel_key) = (fields.index("class"), fields.index("brunnel"));
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        if f.kind != 2 {
            continue;
        }
        let Some(class) = get(layer, &f, class_key).and_then(|v| match v {
            Value::Text(s) => RoadClass::parse(s),
            _ => None,
        }) else {
            continue;
        };
        let tunnel = get(layer, &f, brunnel_key) == Some(Value::Text("tunnel"));
        for line in parse_geometry(f.geometry, layer.extent) {
            if line.len() >= 2 {
                out.push(Road { class, tunnel, line });
            }
        }
    }
}

fn read_boundaries(layer: &Layer, out: &mut Vec<Boundary>) {
    let fields = Fields { layer };
    let level = fields.index("admin_level");
    let maritime = fields.index("maritime");
    let disputed = fields.index("disputed");
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        if f.kind != 2 || get(layer, &f, maritime).is_some_and(Value::truthy) {
            continue;
        }
        let Some(admin_level) = get(layer, &f, level).and_then(Value::number) else { continue };
        let is_disputed = get(layer, &f, disputed).is_some_and(Value::truthy);
        for line in parse_geometry(f.geometry, layer.extent) {
            if line.len() >= 2 {
                out.push(Boundary { admin_level: admin_level as i64, disputed: is_disputed, line });
            }
        }
    }
}

fn read_places(layer: &Layer, key: TileKey, names: &[String], out: &mut Vec<Label>) {
    let name_keys = name_indices(layer, names);
    let fields = Fields { layer };
    let (class_key, rank_key) = (fields.index("class"), fields.index("rank"));
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        let Some(Value::Text(class)) = get(layer, &f, class_key) else { continue };
        // Far out, countries are what matters; states and cities join in as the view closes in.
        let (style, base) = match class {
            "country" => (LabelStyle::Country, 100),
            "state" | "province" if key.z >= 5 => (LabelStyle::State, 250),
            "city" if key.z >= 4 => (LabelStyle::City, 300),
            "town" => (LabelStyle::Town, 500),
            "village" => (LabelStyle::Village, 650),
            "suburb" | "quarter" | "borough" => (LabelStyle::Local, 750),
            "hamlet" | "neighbourhood" | "isolated_dwelling" => (LabelStyle::Local, 850),
            _ => continue,
        };
        let Some(text) = first_name(layer, &f, &name_keys) else { continue };
        let rank = get(layer, &f, rank_key).and_then(Value::number).unwrap_or(50.0) as i32;
        for part in parse_geometry(f.geometry, layer.extent) {
            let Some(&p) = part.first().filter(|p| inside(**p)) else { continue };
            let (lat, lon) = point_to_lat_lon(key, p);
            out.push(Label { text: text.clone(), style, priority: base + rank, lat, lon, along: None });
        }
    }
}

fn read_water_names(layer: &Layer, key: TileKey, names: &[String], out: &mut Vec<Label>) {
    let name_keys = name_indices(layer, names);
    let class_key = Fields { layer }.index("class");
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        let Some(text) = first_name(layer, &f, &name_keys) else { continue };
        let (style, priority) = match get(layer, &f, class_key) {
            Some(Value::Text("ocean")) => (LabelStyle::Sea, 50),
            Some(Value::Text("sea")) => (LabelStyle::Sea, 400),
            // Named ponds should not crowd out the districts around them.
            _ => (LabelStyle::Lake, 780),
        };
        for part in parse_geometry(f.geometry, layer.extent) {
            let Some(&p) = part.first().filter(|p| inside(**p)) else { continue };
            let (lat, lon) = point_to_lat_lon(key, p);
            out.push(Label { text: text.clone(), style, priority, lat, lon, along: None });
        }
    }
}

fn read_peaks(layer: &Layer, key: TileKey, names: &[String], out: &mut Vec<Label>) {
    // Peaks are ranked within their tile, so from afar even small hills rank first.
    if key.z < MIN_PEAK_Z {
        return;
    }
    let name_keys = name_indices(layer, names);
    let fields = Fields { layer };
    let (class_key, rank_key, ele_key) = (fields.index("class"), fields.index("rank"), fields.index("ele"));
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        if f.kind != 1 || get(layer, &f, class_key) != Some(Value::Text("peak")) {
            continue;
        }
        let Some(name) = first_name(layer, &f, &name_keys) else { continue };
        let text = match get(layer, &f, ele_key).and_then(Value::number) {
            Some(ele) if ele > 0.0 => format!("▲ {name} {} m", ele as i64),
            _ => format!("▲ {name}"),
        };
        let rank = get(layer, &f, rank_key).and_then(Value::number).unwrap_or(10.0) as i32;
        for part in parse_geometry(f.geometry, layer.extent) {
            let Some(&p) = part.first().filter(|p| inside(**p)) else { continue };
            let (lat, lon) = point_to_lat_lon(key, p);
            out.push(Label { text: text.clone(), style: LabelStyle::Peak, priority: 620 + rank, lat, lon, along: None });
        }
    }
}

fn read_road_names(layer: &Layer, key: TileKey, names: &[String], out: &mut Vec<Label>) {
    let name_keys = name_indices(layer, names);
    let fields = Fields { layer };
    let (class_key, ref_key) = (fields.index("class"), fields.index("ref"));
    for bytes in &layer.features {
        let Some(f) = parse_feature(bytes) else { continue };
        // Point features here are motorway junctions, which would crowd out everything else.
        if f.kind != 2 {
            continue;
        }
        let major_road = matches!(
            get(layer, &f, class_key),
            Some(Value::Text("motorway" | "trunk" | "primary"))
        );
        let reference = get(layer, &f, ref_key).and_then(Value::text);
        let text = if major_road {
            reference.or_else(|| first_name(layer, &f, &name_keys))
        } else {
            first_name(layer, &f, &name_keys)
        };
        let Some(text) = text else { continue };
        let priority = if major_road { 700 } else { 900 };
        for line in parse_geometry(f.geometry, layer.extent) {
            if line.len() < 2 {
                continue;
            }
            let mid = along_line(&line, 0.5);
            if !inside(mid) {
                continue;
            }
            let (lat, lon) = point_to_lat_lon(key, mid);
            let along = Some([
                point_to_lat_lon(key, along_line(&line, 0.4)),
                point_to_lat_lon(key, along_line(&line, 0.6)),
            ]);
            out.push(Label { text: text.clone(), style: LabelStyle::Road, priority, lat, lon, along });
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        while v >= 0x80 {
            out.push(v as u8 | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    fn bytes_field(n: u64, data: &[u8], out: &mut Vec<u8>) {
        varint(n << 3 | 2, out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }

    fn varint_field(n: u64, v: u64, out: &mut Vec<u8>) {
        varint(n << 3, out);
        varint(v, out);
    }

    fn zig(v: i64) -> u64 {
        ((v << 1) ^ (v >> 63)) as u64
    }

    #[derive(Clone)]
    pub enum V {
        S(&'static str),
        Sint(i64),
    }

    /// A feature for test data: type, properties and geometry commands.
    type TestFeature<'a> = (u64, Vec<(&'a str, V)>, Vec<u64>);

    /// Encodes one layer of features.
    pub fn layer(name: &str, features: &[TestFeature]) -> Vec<u8> {
        let mut l = Vec::new();
        bytes_field(1, name.as_bytes(), &mut l);
        let (mut keys, mut values): (Vec<String>, Vec<Vec<u8>>) = (Vec::new(), Vec::new());
        for (kind, props, geometry) in features {
            let mut tags = Vec::new();
            for (k, v) in props {
                // Real tiles share one key table entry per field name.
                let key = keys.iter().position(|e| e == k).unwrap_or_else(|| {
                    keys.push(k.to_string());
                    keys.len() - 1
                });
                let mut encoded = Vec::new();
                match v {
                    V::S(s) => bytes_field(1, s.as_bytes(), &mut encoded),
                    V::Sint(i) => varint_field(6, zig(*i), &mut encoded),
                }
                values.push(encoded);
                varint(key as u64, &mut tags);
                varint(values.len() as u64 - 1, &mut tags);
            }
            let mut f = Vec::new();
            bytes_field(2, &tags, &mut f);
            varint_field(3, *kind, &mut f);
            let mut g = Vec::new();
            for c in geometry {
                varint(*c, &mut g);
            }
            bytes_field(4, &g, &mut f);
            bytes_field(2, &f, &mut l);
        }
        for k in &keys {
            bytes_field(3, k.as_bytes(), &mut l);
        }
        for v in &values {
            bytes_field(4, v, &mut l);
        }
        varint_field(5, 4096, &mut l);
        let mut tile = Vec::new();
        bytes_field(3, &l, &mut tile);
        tile
    }

    pub fn move_to(x: i64, y: i64) -> Vec<u64> {
        vec![1 << 3 | 1, zig(x), zig(y)]
    }

    pub fn line_to(points: &[(i64, i64)]) -> Vec<u64> {
        let mut out = vec![(points.len() as u64) << 3 | 2];
        for (x, y) in points {
            out.extend([zig(*x), zig(*y)]);
        }
        out
    }

    fn names() -> Vec<String> {
        name_fields("de")
    }

    const KEY: TileKey = TileKey::new(10, 544, 359);

    #[test]
    fn reads_roads_with_class_and_tunnel() {
        let mut g = move_to(0, 0);
        g.extend(line_to(&[(2048, 0), (0, 2048)]));
        let data = layer(
            "transportation",
            &[
                (2, vec![("class", V::S("motorway")), ("brunnel", V::S("tunnel"))], g.clone()),
                (2, vec![("class", V::S("minor"))], g.clone()),
                (2, vec![("class", V::S("motorway_construction"))], g),
            ],
        );
        let t = decode(&data, KEY, &names()).unwrap();
        assert_eq!(t.roads.len(), 2);
        assert_eq!(t.roads[0].class, RoadClass::Motorway);
        assert!(t.roads[0].tunnel);
        assert!(!t.roads[1].tunnel);
        assert_eq!(t.roads[0].line, vec![[0.0, 0.0], [0.5, 0.0], [0.5, 0.5]]);
    }

    #[test]
    fn multi_part_lines_stay_separate() {
        let mut g = move_to(0, 0);
        g.extend(line_to(&[(100, 0)]));
        g.extend(move_to(0, 100));
        g.extend(line_to(&[(100, 0)]));
        let data = layer("transportation", &[(2, vec![("class", V::S("primary"))], g)]);
        assert_eq!(decode(&data, KEY, &names()).unwrap().roads.len(), 2);
    }

    #[test]
    fn boundaries_skip_the_sea() {
        let mut g = move_to(0, 0);
        g.extend(line_to(&[(100, 0)]));
        let data = layer(
            "boundary",
            &[
                (2, vec![("admin_level", V::Sint(2)), ("maritime", V::Sint(0))], g.clone()),
                (2, vec![("admin_level", V::Sint(2)), ("maritime", V::Sint(1))], g),
            ],
        );
        let t = decode(&data, KEY, &names()).unwrap();
        assert_eq!(t.boundaries.len(), 1);
        assert_eq!(t.boundaries[0].admin_level, 2);
    }

    #[test]
    fn places_prefer_the_user_language_and_use_position() {
        let data = layer(
            "place",
            &[
                (1, vec![("class", V::S("city")), ("rank", V::Sint(7)), ("name", V::S("München")), ("name:de", V::S("München DE"))], move_to(2048, 2048)),
                (1, vec![("class", V::S("town")), ("name", V::S("Local")), ("name:latin", V::S("Latin"))], move_to(100, 100)),
                (1, vec![("class", V::S("town")), ("name", V::S("Only"))], move_to(100, 100)),
                (1, vec![("class", V::S("city"))], move_to(100, 100)),
                (1, vec![("class", V::S("city")), ("name", V::S("Outside"))], move_to(5000, 100)),
                (1, vec![("class", V::S("spaceport")), ("name", V::S("Unknown"))], move_to(100, 100)),
            ],
        );
        let t = decode(&data, KEY, &names()).unwrap();
        let labels: Vec<_> = t.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(labels, ["München DE", "Latin", "Only"]);
        let (lat, lon) = KEY.lat_lon(0.5, 0.5);
        assert!((t.labels[0].lat - lat).abs() < 1e-9 && (t.labels[0].lon - lon).abs() < 1e-9);
        assert_eq!(t.labels[0].priority, 307);
        assert_eq!(t.labels[0].style, LabelStyle::City);
    }

    #[test]
    fn far_views_keep_only_the_big_picture() {
        let place = |class| (1, vec![("class", V::S(class)), ("name", V::S(class))], move_to(10, 10));
        let peak = (1, vec![("class", V::S("peak")), ("name", V::S("Hill"))], move_to(10, 10));
        let mut data = layer(
            "place",
            &[place("continent"), place("country"), place("state"), place("city"), place("town")],
        );
        data.extend(layer("mountain_peak", &[peak]));
        let at = |z| {
            let t = decode(&data, TileKey::new(z, 0, 0), &names()).unwrap();
            t.labels.iter().map(|l| l.style).collect::<Vec<_>>()
        };
        assert_eq!(at(3), [LabelStyle::Country, LabelStyle::Town]);
        assert_eq!(at(4), [LabelStyle::Country, LabelStyle::City, LabelStyle::Town]);
        assert_eq!(at(5), [LabelStyle::Country, LabelStyle::State, LabelStyle::City, LabelStyle::Town]);
        assert_eq!(at(9).last(), Some(&LabelStyle::Peak));
    }

    #[test]
    fn peaks_show_their_height() {
        let data = layer(
            "mountain_peak",
            &[
                (1, vec![("class", V::S("peak")), ("name", V::S("Olperer")), ("ele", V::Sint(3476)), ("rank", V::Sint(1))], move_to(10, 10)),
                (1, vec![("class", V::S("saddle")), ("name", V::S("Joch"))], move_to(10, 10)),
                (2, vec![("class", V::S("peak")), ("name", V::S("Ridge"))], move_to(10, 10)),
            ],
        );
        let t = decode(&data, KEY, &names()).unwrap();
        assert_eq!(t.labels.len(), 1);
        assert_eq!(t.labels[0].text, "▲ Olperer 3476 m");
    }

    #[test]
    fn major_roads_use_their_number_and_junctions_are_skipped() {
        let mut g = move_to(0, 100);
        g.extend(line_to(&[(1000, 0)]));
        let data = layer(
            "transportation_name",
            &[
                (2, vec![("class", V::S("motorway")), ("ref", V::S("A 12")), ("name", V::S("Inntal"))], g.clone()),
                (2, vec![("class", V::S("minor")), ("ref", V::S("x")), ("name", V::S("Maria-Theresien-Straße"))], g),
                (1, vec![("class", V::S("motorway")), ("name", V::S("Junction"))], move_to(10, 10)),
            ],
        );
        let t = decode(&data, KEY, &names()).unwrap();
        let labels: Vec<_> = t.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(labels, ["A 12", "Maria-Theresien-Straße"]);
        assert!(t.labels[0].along.is_some());
    }

    fn close_path() -> Vec<u64> {
        vec![1 << 3 | 7]
    }

    /// A square ring; clockwise in tile coordinates makes an outer ring.
    fn square(x: i64, y: i64, size: i64, clockwise: bool) -> Vec<u64> {
        let mut g = move_to(x, y);
        let steps: [(i64, i64); 3] = if clockwise {
            [(size, 0), (0, size), (-size, 0)]
        } else {
            [(0, size), (size, 0), (0, -size)]
        };
        g.extend(line_to(&steps));
        g.extend(close_path());
        g
    }

    /// Geometry deltas continue from the last point, so rings after the first need to know it.
    fn after(previous_end: (i64, i64), target: (i64, i64)) -> (i64, i64) {
        (target.0 - previous_end.0, target.1 - previous_end.1)
    }

    const CITY: TileKey = TileKey::new(14, 8712, 5750);

    #[test]
    fn merged_features_split_into_buildings_with_their_holes() {
        // Outer ring at (0,0), a hole inside it, then a second building.
        let mut g = square(0, 0, 1000, true);
        // The first ring ended at (0, 1000).
        let d = after((0, 1000), (100, 100));
        g.extend(square(d.0, d.1, 100, false));
        // The hole ended at (200, 100).
        let d = after((200, 100), (2000, 2000));
        g.extend(square(d.0, d.1, 500, true));
        let data = layer(
            "building",
            &[(3, vec![("render_height", V::Sint(20)), ("render_min_height", V::Sint(0)), ("colour", V::S("#5a81a0"))], g)],
        );
        let t = decode(&data, CITY, &names()).unwrap();
        assert_eq!(t.buildings.len(), 2);
        assert_eq!(t.buildings[0].rings.len(), 2, "outer ring and hole");
        assert_eq!(t.buildings[1].rings.len(), 1);
        assert_eq!(t.buildings[0].height, 20.0);
        assert_eq!(t.buildings[1].colour, Some([0x5a, 0x81, 0xa0]));
        // Rings end where they begin.
        let ring = &t.buildings[0].rings[0];
        assert_eq!(ring.first(), ring.last());
    }

    #[test]
    fn building_attributes_are_filtered_and_limited() {
        let g = square(0, 0, 1000, true);
        let data = layer(
            "building",
            &[
                (3, vec![("render_height", V::Sint(30)), ("hide_3d", V::Sint(1))], g.clone()),
                (3, vec![("render_height", V::Sint(5)), ("render_min_height", V::Sint(9))], g.clone()),
                (3, vec![], g.clone()),
                (3, vec![("render_height", V::Sint(90000))], g.clone()),
                (2, vec![("render_height", V::Sint(5))], g),
            ],
        );
        let t = decode(&data, CITY, &names()).unwrap();
        let heights: Vec<f32> = t.buildings.iter().map(|b| b.height).collect();
        assert_eq!(heights, [6.0, 1000.0], "hidden and inverted are skipped, the rest defaulted or capped");
    }

    #[test]
    fn buildings_take_the_zone_they_stand_in_and_the_smaller_of_two() {
        // A residential square over most of the tile with shops in its middle, and an industrial
        // estate in the far corner.
        let mut tile = layer(
            "landuse",
            &[
                (3, vec![("class", V::S("residential"))], square(0, 0, 3000, true)),
                (3, vec![("class", V::S("retail"))], square(1000, 1000, 500, true)),
                (3, vec![("class", V::S("industrial"))], square(3500, 3500, 500, true)),
                (3, vec![("class", V::S("school"))], square(100, 100, 300, true)),
            ],
        );
        let houses = [(200, 200), (1100, 1100), (3600, 3600), (3200, 100)];
        let features: Vec<_> = houses.iter().map(|&(x, y)| (3, vec![("render_height", V::Sint(10))], square(x, y, 50, true))).collect();
        tile.extend(layer("building", &features));
        let t = decode(&tile, CITY, &names()).unwrap();
        let zones: Vec<Option<Zone>> = t.buildings.iter().map(|b| b.zone).collect();
        assert_eq!(zones, [Some(Zone::Residential), Some(Zone::Commercial), Some(Zone::Industrial), None]);
    }

    #[test]
    fn coarse_tiles_have_no_buildings() {
        let data = layer("building", &[(3, vec![], square(0, 0, 1000, true))]);
        assert!(decode(&data, TileKey::new(13, 4356, 2875), &names()).unwrap().buildings.is_empty());
        assert_eq!(decode(&data, CITY, &names()).unwrap().buildings.len(), 1);
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_colour("#5a81a0"), Some([0x5a, 0x81, 0xa0]));
        assert_eq!(parse_colour("#FFF"), Some([255, 255, 255]));
        assert_eq!(parse_colour(" White "), Some([255, 255, 255]));
        assert_eq!(parse_colour("lightyellow"), Some([250, 245, 205]));
        assert_eq!(parse_colour("#12"), None);
        assert_eq!(parse_colour("#gggggg"), None);
        assert_eq!(parse_colour("#ä"), None);
        assert_eq!(parse_colour("sparkly"), None);
    }

    #[test]
    fn line_position_interpolates_by_length() {
        let line = [[0.0, 0.0], [0.2, 0.0], [0.2, 0.2]];
        assert_eq!(along_line(&line, 0.5), [0.2, 0.0]);
        assert_eq!(along_line(&line, 1.0), [0.2, 0.2]);
        let p = along_line(&line, 0.25);
        assert!((p[0] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn language_choice() {
        assert_eq!(language_from_locale("de_DE.UTF-8"), "de");
        assert_eq!(language_from_locale("C"), "");
        assert_eq!(language_from_locale(""), "");
        assert_eq!(name_fields("de"), ["name:de", "name_de", "name:latin", "name"]);
        assert_eq!(name_fields("ja"), ["name:ja", "name_ja", "name"]);
        assert_eq!(name_fields(""), ["name:latin", "name"]);
    }

    #[test]
    fn empty_tile_decodes_to_nothing() {
        let t = decode(&[], KEY, &names()).unwrap();
        assert!(t.roads.is_empty() && t.labels.is_empty());
    }

    #[test]
    fn malformed_input_never_panics() {
        let mut g = move_to(0, 0);
        g.extend(line_to(&[(100, 100), (5, 5)]));
        let good = layer(
            "place",
            &[(1, vec![("class", V::S("city")), ("name", V::S("X"))], g)],
        );
        // Every truncation, and every single corrupted byte.
        for n in 0..good.len() {
            let _ = decode(&good[..n], KEY, &names());
        }
        for i in 0..good.len() {
            for v in [0u8, 0x7f, 0x80, 0xff] {
                let mut bad = good.clone();
                bad[i] = v;
                let _ = decode(&bad, KEY, &names());
            }
        }
        // Tag indices beyond the value table are ignored.
        let mut f = Vec::new();
        bytes_field(2, &[0, 200], &mut f);
        let mut l = Vec::new();
        bytes_field(1, b"place", &mut l);
        bytes_field(3, b"class", &mut l);
        bytes_field(2, &f, &mut l);
        let mut tile = Vec::new();
        bytes_field(3, &l, &mut tile);
        assert!(decode(&tile, KEY, &names()).unwrap().labels.is_empty());
        // A huge repeat count with no data behind it ends quickly.
        assert!(parse_geometry(&[0xff, 0xff, 0xff, 0xff, 0x0f], 4096.0).len() <= 1);
    }
}
