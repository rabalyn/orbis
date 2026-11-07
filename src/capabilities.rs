//! Reads the capabilities documents of WMTS and WMS services into plain lists.
//!
//! Formats: OGC 07-057r7 (WMTS 1.0.0), OGC 06-042 (WMS 1.3.0) and OGC 01-068r3 (WMS 1.1.1).
//! Only what Orbis needs is read: layers, formats, styles, matrix sets, coordinate systems,
//! bounding boxes and the addresses to ask. Everything else is ignored, and anything missing
//! or odd makes a layer unusable rather than stopping the whole document.



use crate::import::{walk, Tok};

/// The largest capabilities document that is read.
pub const MAX_BYTES: usize = 8 << 20;

/// An element with its attributes (without namespace prefixes), text and children.
#[derive(Debug, Default, Clone)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == key).map(|a| a.1.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The element at the end of a path of child names.
    pub fn at(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |n, name| n.child(name))
    }

    /// The trimmed text of a direct child.
    pub fn text_of(&self, name: &str) -> Option<String> {
        let t = self.child(name)?.text.trim();
        (!t.is_empty()).then(|| t.to_string())
    }
}

/// Builds the element tree. Elements nest at most as deep as `import::walk` allows.
pub fn parse_tree(text: &str) -> Result<Node, String> {
    if text.len() > MAX_BYTES {
        return Err("the description is too large".into());
    }
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    walk(text, |tok| {
        match tok {
            Tok::Open(name, attrs) => stack.push(Node { name, attrs, ..Node::default() }),
            Tok::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            Tok::Close(_) => {
                if let Some(done) = stack.pop() {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(done),
                        None => root = Some(done),
                    }
                }
            }
        }
        true
    })
    .map_err(|e| format!("not a readable XML document ({e})"))?;
    root.ok_or_else(|| "not a readable XML document".to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub format: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WmtsLayer {
    pub id: String,
    pub title: String,
    pub formats: Vec<String>,
    /// Style identifiers, the default first.
    pub styles: Vec<String>,
    /// The matrix sets this layer is offered in.
    pub sets: Vec<String>,
    /// For each linked set that lists limits, the matrices it has tiles in.
    pub limits: Vec<(String, Vec<String>)>,
    pub resources: Vec<Resource>,
    /// Extra dimensions with their default values.
    pub dimensions: Vec<(String, String)>,
    /// West, south, east, north in degrees.
    pub bounds: Option<[f64; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Matrix {
    pub id: String,
    pub top_left: (f64, f64),
    pub tile: (u32, u32),
    pub size: (u64, u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatrixSet {
    pub id: String,
    pub crs: String,
    pub matrices: Vec<Matrix>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wmts {
    pub layers: Vec<WmtsLayer>,
    pub sets: Vec<MatrixSet>,
    /// The address for GetTile requests in key-value form, if the document gives one.
    pub get_tile_url: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WmsLayer {
    pub name: String,
    pub title: String,
    /// Coordinate systems, including those inherited from enclosing layers.
    pub crs: Vec<String>,
    pub bounds: Option<[f64; 4]>,
    pub attribution: Option<String>,
    pub min_scale_denominator: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Wms {
    pub version: String,
    pub layers: Vec<WmsLayer>,
    pub formats: Vec<String>,
    pub get_map_url: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Document {
    Wmts(Wmts),
    Wms(Wms),
}

/// Reads a capabilities document of either kind, saying plainly what it is if it is neither.
pub fn parse(text: &str) -> Result<Document, String> {
    let head = text.trim_start();
    if head.len() >= 5 && head[..5].eq_ignore_ascii_case("<html") || head.get(..9).is_some_and(|h| h.eq_ignore_ascii_case("<!doctype")) {
        return Err("the address returned a web page, not a service description".into());
    }
    let root = parse_tree(text)?;
    match root.name.as_str() {
        "Capabilities" => parse_wmts(&root).map(Document::Wmts),
        "WMS_Capabilities" | "WMT_MS_Capabilities" => parse_wms(&root).map(Document::Wms),
        "ServiceExceptionReport" | "ExceptionReport" => {
            let why = first_text(&root).unwrap_or_default();
            Err(format!("the service answered with an error: {}", clip(&why, 160)))
        }
        "html" | "HTML" => Err("the address returned a web page, not a service description".into()),
        other => Err(format!("not a WMTS or WMS description (it starts with <{other}>)")),
    }
}

fn first_text(n: &Node) -> Option<String> {
    let t = n.text.trim();
    if !t.is_empty() {
        return Some(t.to_string());
    }
    n.children.iter().find_map(first_text)
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect::<String>().replace(['\n', '\r'], " ")
}

fn numbers(text: &str) -> Vec<f64> {
    text.split_whitespace().filter_map(|t| t.parse::<f64>().ok()).filter(|v| v.is_finite()).collect()
}

fn lon_lat_box(lower: &str, upper: &str) -> Option<[f64; 4]> {
    let (l, u) = (numbers(lower), numbers(upper));
    sane_box(*l.first()?, *l.get(1)?, *u.first()?, *u.get(1)?)
}

fn sane_box(west: f64, south: f64, east: f64, north: f64) -> Option<[f64; 4]> {
    let ok = (-180.0..=180.0).contains(&west)
        && (-180.0..=180.0).contains(&east)
        && (-90.0..=90.0).contains(&south)
        && (-90.0..=90.0).contains(&north)
        && south < north
        && west != east;
    ok.then_some([west, south, east, north])
}

// ---- WMTS ----

fn parse_wmts(root: &Node) -> Result<Wmts, String> {
    let contents = root.child("Contents").ok_or("the service description has no layers")?;
    let layers: Vec<WmtsLayer> = contents.all("Layer").filter_map(wmts_layer).collect();
    let sets = contents.all("TileMatrixSet").filter_map(matrix_set).collect();
    let get_tile_url = root
        .child("OperationsMetadata")
        .and_then(|m| m.all("Operation").find(|o| o.attr("name") == Some("GetTile")))
        .and_then(|o| o.at(&["DCP", "HTTP", "Get"]))
        .and_then(|g| g.attr("href").map(str::to_string));
    let title = root.at(&["ServiceIdentification"]).and_then(|s| s.text_of("Title")).unwrap_or_default();
    if layers.is_empty() {
        return Err("the service description lists no layers".into());
    }
    Ok(Wmts { layers, sets, get_tile_url, title })
}

fn wmts_layer(n: &Node) -> Option<WmtsLayer> {
    let id = n.text_of("Identifier")?;
    let mut styles: Vec<(String, bool)> = n
        .all("Style")
        .filter_map(|s| Some((s.text_of("Identifier")?, s.attr("isDefault") == Some("true"))))
        .collect();
    styles.sort_by_key(|s| !s.1);
    let resources = n
        .all("ResourceURL")
        .filter(|r| r.attr("resourceType").is_none_or(|t| t == "tile"))
        .filter_map(|r| Some(Resource { format: r.attr("format")?.to_string(), template: r.attr("template")?.to_string() }))
        .collect();
    let dimensions = n
        .all("Dimension")
        .filter_map(|d| Some((d.text_of("Identifier")?, d.text_of("Default")?)))
        .collect();
    let mut sets = Vec::new();
    let mut limits = Vec::new();
    for link in n.all("TileMatrixSetLink") {
        let Some(set) = link.text_of("TileMatrixSet") else { continue };
        if let Some(l) = link.child("TileMatrixSetLimits") {
            let ids = l.all("TileMatrixLimits").filter_map(|m| m.text_of("TileMatrix")).collect();
            limits.push((set.clone(), ids));
        }
        sets.push(set);
    }
    let bounds = n
        .child("WGS84BoundingBox")
        .and_then(|b| lon_lat_box(&b.text_of("LowerCorner")?, &b.text_of("UpperCorner")?));
    Some(WmtsLayer {
        title: n.text_of("Title").unwrap_or_else(|| id.clone()),
        id,
        formats: n.all("Format").map(|f| f.text.trim().to_string()).filter(|f| !f.is_empty()).collect(),
        styles: styles.into_iter().map(|s| s.0).collect(),
        sets,
        limits,
        resources,
        dimensions,
        bounds,
    })
}

fn matrix_set(n: &Node) -> Option<MatrixSet> {
    let id = n.text_of("Identifier")?;
    let crs = n.text_of("SupportedCRS").unwrap_or_default();
    let matrices = n
        .all("TileMatrix")
        .filter_map(|m| {
            let corner = numbers(&m.text_of("TopLeftCorner")?);
            Some(Matrix {
                id: m.text_of("Identifier")?,
                top_left: (*corner.first()?, *corner.get(1)?),
                tile: (m.text_of("TileWidth")?.parse().ok()?, m.text_of("TileHeight")?.parse().ok()?),
                size: (m.text_of("MatrixWidth")?.parse().ok()?, m.text_of("MatrixHeight")?.parse().ok()?),
            })
        })
        .collect();
    Some(MatrixSet { id, crs, matrices })
}

// ---- WMS ----

fn parse_wms(root: &Node) -> Result<Wms, String> {
    let version = root.attr("version").unwrap_or("").to_string();
    let capability = root.child("Capability").ok_or("the service description has no capabilities")?;
    let mut layers = Vec::new();
    if let Some(top) = capability.child("Layer") {
        collect_wms_layers(top, &Inherited::default(), &mut layers);
    }
    let get_map = capability.at(&["Request", "GetMap"]);
    let formats = get_map.map(|g| g.all("Format").map(|f| f.text.trim().to_string()).filter(|f| !f.is_empty()).collect()).unwrap_or_default();
    let get_map_url = get_map
        .and_then(|g| g.at(&["DCPType", "HTTP", "Get", "OnlineResource"]))
        .and_then(|r| r.attr("href").map(str::to_string));
    let title = root.at(&["Service"]).and_then(|s| s.text_of("Title")).unwrap_or_default();
    if layers.is_empty() {
        return Err("the service description lists no named layers".into());
    }
    Ok(Wms { version, layers, formats, get_map_url, title })
}

#[derive(Default, Clone)]
struct Inherited {
    crs: Vec<String>,
    bounds: Option<[f64; 4]>,
    attribution: Option<String>,
    min_scale: Option<f64>,
}

fn collect_wms_layers(n: &Node, parent: &Inherited, out: &mut Vec<WmsLayer>) {
    let mut here = parent.clone();
    for c in n.children.iter().filter(|c| c.name == "CRS" || c.name == "SRS") {
        for code in c.text.split_whitespace() {
            if !here.crs.iter().any(|x| x == code) {
                here.crs.push(code.to_string());
            }
        }
    }
    if let Some(b) = n.child("EX_GeographicBoundingBox") {
        let v = |k: &str| b.text_of(k).and_then(|t| t.parse::<f64>().ok());
        if let (Some(w), Some(s), Some(e), Some(nn)) = (v("westBoundLongitude"), v("southBoundLatitude"), v("eastBoundLongitude"), v("northBoundLatitude")) {
            here.bounds = sane_box(w, s, e, nn).or(here.bounds);
        }
    } else if let Some(b) = n.child("LatLonBoundingBox") {
        let v = |k: &str| b.attr(k).and_then(|t| t.trim().parse::<f64>().ok());
        if let (Some(w), Some(s), Some(e), Some(nn)) = (v("minx"), v("miny"), v("maxx"), v("maxy")) {
            here.bounds = sane_box(w, s, e, nn).or(here.bounds);
        }
    }
    if let Some(a) = n.child("Attribution").and_then(|a| a.text_of("Title")) {
        here.attribution = Some(a);
    }
    if let Some(m) = n.text_of("MinScaleDenominator").and_then(|t| t.parse::<f64>().ok()).filter(|m| *m > 0.0) {
        here.min_scale = Some(m);
    }
    if let Some(name) = n.text_of("Name") {
        out.push(WmsLayer {
            title: n.text_of("Title").unwrap_or_else(|| name.clone()),
            name,
            crs: here.crs.clone(),
            bounds: here.bounds,
            attribution: here.attribution.clone(),
            min_scale_denominator: here.min_scale,
        });
    }
    for child in n.all("Layer") {
        collect_wms_layers(child, &here, out);
    }
}

#[cfg(test)]
pub mod fixtures {
    pub const WMTS: &str = r#"<?xml version="1.0"?>
<Capabilities xmlns="http://www.opengis.net/wmts/1.0" xmlns:ows="http://www.opengis.net/ows/1.1" version="1.0.0">
 <ows:ServiceIdentification><ows:Title>Test WMTS</ows:Title></ows:ServiceIdentification>
 <ows:OperationsMetadata>
  <ows:Operation name="GetCapabilities"><ows:DCP><ows:HTTP><ows:Get xlink:href="http://x/cap"/></ows:HTTP></ows:DCP></ows:Operation>
  <ows:Operation name="GetTile"><ows:DCP><ows:HTTP><ows:Get xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="https://t.example/wmts?"/></ows:HTTP></ows:DCP></ows:Operation>
 </ows:OperationsMetadata>
 <Contents>
  <Layer>
   <ows:Title>Relief</ows:Title><ows:Identifier>relief</ows:Identifier>
   <ows:WGS84BoundingBox><ows:LowerCorner>5.0 45.0</ows:LowerCorner><ows:UpperCorner>11.0 48.0</ows:UpperCorner></ows:WGS84BoundingBox>
   <Style isDefault="true"><ows:Identifier>default</ows:Identifier></Style>
   <Format>image/jpeg</Format><Format>image/png</Format>
   <Dimension><ows:Identifier>Time</ows:Identifier><Default>current</Default></Dimension>
   <TileMatrixSetLink><TileMatrixSet>GoogleMapsCompatible</TileMatrixSet>
    <TileMatrixSetLimits><TileMatrixLimits><TileMatrix>0</TileMatrix></TileMatrixLimits><TileMatrixLimits><TileMatrix>1</TileMatrix></TileMatrixLimits><TileMatrixLimits><TileMatrix>2</TileMatrix></TileMatrixLimits></TileMatrixSetLimits>
   </TileMatrixSetLink>
   <ResourceURL format="image/png" resourceType="tile" template="https://t.example/rest/relief/{Style}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.png"/>
  </Layer>
  <Layer>
   <ows:Title>Polar</ows:Title><ows:Identifier>polar</ows:Identifier>
   <Format>image/png</Format>
   <TileMatrixSetLink><TileMatrixSet>EPSG:4326</TileMatrixSet></TileMatrixSetLink>
  </Layer>
  <TileMatrixSet><ows:Identifier>GoogleMapsCompatible</ows:Identifier><ows:SupportedCRS>urn:ogc:def:crs:EPSG::3857</ows:SupportedCRS>
   <TileMatrix><ows:Identifier>0</ows:Identifier><ScaleDenominator>559082264.0287178</ScaleDenominator><TopLeftCorner>-20037508.3428 20037508.3428</TopLeftCorner><TileWidth>256</TileWidth><TileHeight>256</TileHeight><MatrixWidth>1</MatrixWidth><MatrixHeight>1</MatrixHeight></TileMatrix>
   <TileMatrix><ows:Identifier>1</ows:Identifier><ScaleDenominator>279541132.0143589</ScaleDenominator><TopLeftCorner>-20037508.3428 20037508.3428</TopLeftCorner><TileWidth>256</TileWidth><TileHeight>256</TileHeight><MatrixWidth>2</MatrixWidth><MatrixHeight>2</MatrixHeight></TileMatrix>
   <TileMatrix><ows:Identifier>2</ows:Identifier><ScaleDenominator>139770566.0071794</ScaleDenominator><TopLeftCorner>-20037508.3428 20037508.3428</TopLeftCorner><TileWidth>256</TileWidth><TileHeight>256</TileHeight><MatrixWidth>4</MatrixWidth><MatrixHeight>4</MatrixHeight></TileMatrix>
   <TileMatrix><ows:Identifier>3</ows:Identifier><ScaleDenominator>69885283.0</ScaleDenominator><TopLeftCorner>-20037508.3428 20037508.3428</TopLeftCorner><TileWidth>256</TileWidth><TileHeight>256</TileHeight><MatrixWidth>8</MatrixWidth><MatrixHeight>8</MatrixHeight></TileMatrix>
  </TileMatrixSet>
  <TileMatrixSet><ows:Identifier>EPSG:4326</ows:Identifier><ows:SupportedCRS>urn:ogc:def:crs:EPSG::4326</ows:SupportedCRS>
   <TileMatrix><ows:Identifier>EPSG:4326:0</ows:Identifier><TopLeftCorner>90.0 -180.0</TopLeftCorner><TileWidth>256</TileWidth><TileHeight>256</TileHeight><MatrixWidth>2</MatrixWidth><MatrixHeight>1</MatrixHeight></TileMatrix>
  </TileMatrixSet>
 </Contents>
</Capabilities>"#;

    pub const WMS130: &str = r#"<?xml version="1.0"?>
<WMS_Capabilities version="1.3.0" xmlns="http://www.opengis.net/wms" xmlns:xlink="http://www.w3.org/1999/xlink">
 <Service><Name>WMS</Name><Title>Test WMS</Title></Service>
 <Capability>
  <Request><GetMap><Format>image/jpeg</Format><Format>image/png</Format>
   <DCPType><HTTP><Get><OnlineResource xlink:href="https://w.example/wms?map=a&amp;"/></Get></HTTP></DCPType></GetMap></Request>
  <Layer>
   <Title>Root</Title><CRS>EPSG:4326</CRS><CRS>EPSG:3857</CRS>
   <EX_GeographicBoundingBox><westBoundLongitude>-10</westBoundLongitude><eastBoundLongitude>30</eastBoundLongitude><southBoundLatitude>35</southBoundLatitude><northBoundLatitude>60</northBoundLatitude></EX_GeographicBoundingBox>
   <Attribution><Title>Test Agency</Title></Attribution>
   <Layer><Name>ortho</Name><Title>Orthophoto</Title></Layer>
   <Layer><Name>dem</Name><Title>Heights</Title><CRS>EPSG:25832</CRS><MinScaleDenominator>2183</MinScaleDenominator></Layer>
  </Layer>
 </Capability>
</WMS_Capabilities>"#;

    pub const WMS111: &str = r#"<?xml version="1.0"?>
<WMT_MS_Capabilities version="1.1.1">
 <Service><Name>OGC:WMS</Name><Title>Old WMS</Title></Service>
 <Capability>
  <Request><GetMap><Format>image/png</Format>
   <DCPType><HTTP><Get><OnlineResource xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="http://old.example/wms?"/></Get></HTTP></DCPType></GetMap></Request>
  <Layer>
   <Title>Root</Title><SRS>EPSG:4326 EPSG:900913</SRS>
   <LatLonBoundingBox minx="-5" miny="40" maxx="10" maxy="52"/>
   <Layer><Name>land</Name><Title>Land</Title></Layer>
  </Layer>
 </Capability>
</WMT_MS_Capabilities>"#;
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn reads_a_wmts_document() {
        let Document::Wmts(d) = parse(WMTS).unwrap() else { panic!("not WMTS") };
        assert_eq!(d.title, "Test WMTS");
        assert_eq!(d.get_tile_url.as_deref(), Some("https://t.example/wmts?"));
        assert_eq!(d.layers.len(), 2);
        let l = &d.layers[0];
        assert_eq!((l.id.as_str(), l.title.as_str()), ("relief", "Relief"));
        assert_eq!(l.formats, ["image/jpeg", "image/png"]);
        assert_eq!(l.styles, ["default"]);
        assert_eq!(l.sets, ["GoogleMapsCompatible"]);
        assert_eq!(l.limits[0].1, ["0", "1", "2"]);
        assert_eq!(l.dimensions, [("Time".to_string(), "current".to_string())]);
        assert_eq!(l.bounds, Some([5.0, 45.0, 11.0, 48.0]));
        assert!(l.resources[0].template.contains("{TileMatrix}"));
        assert_eq!(d.sets.len(), 2);
        assert_eq!(d.sets[0].matrices.len(), 4);
        assert_eq!(d.sets[0].matrices[2].size, (4, 4));
    }

    #[test]
    fn reads_wms_130_with_inheritance() {
        let Document::Wms(d) = parse(WMS130).unwrap() else { panic!("not WMS") };
        assert_eq!(d.version, "1.3.0");
        assert_eq!(d.formats, ["image/jpeg", "image/png"]);
        assert_eq!(d.get_map_url.as_deref(), Some("https://w.example/wms?map=a&"));
        let names: Vec<_> = d.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["ortho", "dem"], "the unnamed root only groups");
        assert_eq!(d.layers[0].crs, ["EPSG:4326", "EPSG:3857"]);
        assert_eq!(d.layers[1].crs, ["EPSG:4326", "EPSG:3857", "EPSG:25832"]);
        assert_eq!(d.layers[0].bounds, Some([-10.0, 35.0, 30.0, 60.0]));
        assert_eq!(d.layers[0].attribution.as_deref(), Some("Test Agency"));
        assert_eq!(d.layers[0].min_scale_denominator, None);
        assert_eq!(d.layers[1].min_scale_denominator, Some(2183.0));
    }

    #[test]
    fn reads_wms_111() {
        let Document::Wms(d) = parse(WMS111).unwrap() else { panic!("not WMS") };
        assert_eq!(d.version, "1.1.1");
        assert_eq!(d.layers[0].crs, ["EPSG:4326", "EPSG:900913"]);
        assert_eq!(d.layers[0].bounds, Some([-5.0, 40.0, 10.0, 52.0]));
    }

    #[test]
    fn bad_documents_give_plain_errors() {
        assert!(parse("").is_err());
        assert!(parse("<Capabilities><Contents>").unwrap_err().contains("XML"));
        assert!(parse("<html><body>Login</body></html>").unwrap_err().contains("web page"));
        assert!(parse("<!DOCTYPE html><html></html>").unwrap_err().contains("web page"));
        assert!(parse("<feed/>").unwrap_err().contains("<feed>"));
        let e = parse("<ServiceExceptionReport><ServiceException>Bad layer</ServiceException></ServiceExceptionReport>").unwrap_err();
        assert!(e.contains("Bad layer"), "{e}");
        assert!(parse("<Capabilities xmlns='x'><Contents/></Capabilities>").unwrap_err().contains("no layers"));
        assert!(parse("<WMS_Capabilities version='1.3.0'><Capability/></WMS_Capabilities>").is_err());
        assert!(parse("<WMS_Capabilities><Service/></WMS_Capabilities>").is_err());
    }

    #[test]
    fn broken_entries_are_skipped_not_fatal() {
        let xml = "<Capabilities><Contents><Layer><Title>No id</Title></Layer><Layer><Identifier>ok</Identifier></Layer>\
                   <TileMatrixSet><Identifier>S</Identifier><TileMatrix><Identifier>0</Identifier><TopLeftCorner>x y</TopLeftCorner></TileMatrix></TileMatrixSet></Contents></Capabilities>";
        let Document::Wmts(d) = parse(xml).unwrap() else { panic!() };
        assert_eq!(d.layers.len(), 1);
        assert!(d.sets[0].matrices.is_empty());
    }

    #[test]
    fn nonsense_boxes_are_dropped() {
        assert_eq!(sane_box(10.0, 50.0, 5.0, 40.0), None);
        assert_eq!(sane_box(-200.0, 0.0, 5.0, 40.0), None);
        assert!(sane_box(-5.0, 40.0, 10.0, 50.0).is_some());
    }
}
