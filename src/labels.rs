//! Text labels: styles, shaping and rasterising on a background thread, an atlas allocator,
//! and choosing each frame which labels fit on screen.

use crate::camera::{Basis, FOV_Y};
use crate::geo::{ecef_to_geodetic, enu, geodetic_to_ecef, ray_ellipsoid};
use crate::vector::{Label, LabelStyle, NATIONAL_CAPITAL_MARK, STATE_CAPITAL_MARK};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight};
use glam::{DVec2, DVec3};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

pub const ATLAS_SIZE: u32 = 4096;
const HALO: i32 = 2;
const MAX_TEXT_CHARS: usize = 64;
const MAX_PLACED: usize = 220;
const PADDING: f64 = 3.0;
/// Two labels with the same text keep at least this many pixels apart.
const SAME_TEXT_GAP: f64 = 260.0;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TextKey {
    pub text: String,
    pub style: LabelStyle,
    pub scale: u32,
}

pub struct Appearance {
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub fill: [f32; 3],
    pub halo: [f32; 3],
}

pub fn appearance(style: LabelStyle) -> Appearance {
    const DARK: [f32; 3] = [0.02, 0.03, 0.05];
    let a = |size, bold, italic, fill, halo| Appearance { size, bold, italic, fill, halo };
    match style {
        LabelStyle::Country => a(17.0, true, false, [1.0, 1.0, 1.0], DARK),
        LabelStyle::State => a(13.0, false, false, [0.86, 0.84, 0.95], DARK),
        LabelStyle::City => a(16.0, true, false, [1.0, 1.0, 1.0], DARK),
        LabelStyle::Town => a(14.0, true, false, [0.96, 0.96, 0.96], DARK),
        LabelStyle::Village => a(13.0, false, false, [0.94, 0.94, 0.94], DARK),
        LabelStyle::Local => a(12.0, false, false, [0.84, 0.86, 0.88], DARK),
        LabelStyle::Sea => a(15.0, false, true, [0.62, 0.8, 1.0], [0.01, 0.04, 0.12]),
        LabelStyle::Lake => a(12.0, false, true, [0.62, 0.84, 1.0], [0.01, 0.04, 0.12]),
        LabelStyle::Peak => a(12.0, false, false, [1.0, 0.86, 0.7], DARK),
        LabelStyle::Road => a(11.0, false, false, [1.0, 0.97, 0.82], DARK),
        LabelStyle::Region => a(15.0, false, true, [1.0, 0.93, 0.8], DARK),
        LabelStyle::Feature => a(12.0, false, false, [0.95, 0.95, 0.9], DARK),
    }
}

/// A rasterised label: two bytes per pixel, the text's coverage and its halo's.
pub struct Mask {
    pub key: TextKey,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Spreads coverage outwards to make the dark outline that keeps text readable on imagery.
fn with_halo(width: usize, height: usize, coverage: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 2];
    let r2 = HALO * HALO + 1;
    for y in 0..height {
        for x in 0..width {
            let c = coverage[y * width + x];
            if c == 0 {
                continue;
            }
            out[(y * width + x) * 2] = c;
            for dy in -HALO..=HALO {
                for dx in -HALO..=HALO {
                    if dx * dx + dy * dy > r2 {
                        continue;
                    }
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height {
                        let cell = &mut out[(ny as usize * width + nx as usize) * 2 + 1];
                        *cell = (*cell).max(c);
                    }
                }
            }
        }
    }
    out
}

fn rasterize(fonts: &mut FontSystem, cache: &mut SwashCache, key: TextKey) -> Option<Mask> {
    let look = appearance(key.style);
    let scale = key.scale as f32;
    let size = look.size * scale;
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.25));
    let mut attrs = Attrs::new().family(Family::SansSerif);
    if look.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if look.italic {
        attrs = attrs.style(Style::Italic);
    }
    buffer.set_size(None, None);
    // The small star before a state capital is drawn larger, or it would be a speck.
    match key.text.strip_prefix(STATE_CAPITAL_MARK) {
        Some(rest) => {
            let big = size * 1.5;
            let mark = attrs.clone().metrics(Metrics::new(big, size * 1.25));
            let mut buf = [0u8; 4];
            buffer.set_rich_text([(&*STATE_CAPITAL_MARK.encode_utf8(&mut buf), mark), (rest, attrs.clone())], &attrs, Shaping::Advanced, None);
        }
        None => buffer.set_text(&key.text, &attrs, Shaping::Advanced, None),
    }
    buffer.shape_until_scroll(fonts, false);
    let text_width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
    if text_width <= 0.0 {
        return None;
    }
    let border = HALO + 1;
    let width = (text_width.ceil() as i32 + 2 * border) as usize;
    let height = ((size * 1.25).ceil() as i32 + 2 * border) as usize;
    if width as u32 > ATLAS_SIZE / 4 || height as u32 > ATLAS_SIZE / 4 {
        return None;
    }
    let mut coverage = vec![0u8; width * height];
    buffer.draw(fonts, cache, Color::rgb(255, 255, 255), |x, y, w, h, color| {
        for py in y..y + h as i32 {
            for px in x..x + w as i32 {
                let (cx, cy) = (px + border, py + border);
                if cx >= 0 && cy >= 0 && (cx as usize) < width && (cy as usize) < height {
                    let cell = &mut coverage[cy as usize * width + cx as usize];
                    *cell = (*cell).max(color.a());
                }
            }
        }
    });
    Some(Mask {
        key,
        width: width as u32,
        height: height as u32,
        pixels: with_halo(width, height, &coverage),
    })
}

/// Shapes and rasterises text on its own thread, because scanning the system's fonts and
/// shaping complex scripts would otherwise stall the frame.
#[cfg(not(target_arch = "wasm32"))]
pub struct TextWorker {
    requests: Sender<TextKey>,
    results: Receiver<Result<Mask, TextKey>>,
    pending: usize,
}

/// The fonts that come with the page, since a browser does not let it read the system's.
#[cfg(target_arch = "wasm32")]
fn page_fonts() -> FontSystem {
    let fonts: [&'static [u8]; 4] = [
        include_bytes!("../web/fonts/NotoSans-Regular.ttf"),
        include_bytes!("../web/fonts/NotoSans-Bold.ttf"),
        include_bytes!("../web/fonts/NotoSans-Italic.ttf"),
        // Arrows and shapes, such as the mark before a peak's name.
        include_bytes!("../web/fonts/NotoSansSymbols2-Subset.ttf"),
    ];
    let sources = fonts.into_iter().map(|f| cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(f)));
    let mut system = FontSystem::new_with_fonts(sources);
    system.db_mut().set_sans_serif_family("Noto Sans");
    system
}

/// In a browser, text is made between frames, as much as the frame's spare time allows.
#[cfg(target_arch = "wasm32")]
pub struct TextWorker {
    queue: std::collections::VecDeque<TextKey>,
    fonts: Option<FontSystem>,
    cache: SwashCache,
}

#[cfg(target_arch = "wasm32")]
impl TextWorker {
    pub fn new() -> Self {
        Self { queue: Default::default(), fonts: None, cache: SwashCache::new() }
    }

    pub fn request(&mut self, key: TextKey) {
        self.queue.push_back(key);
    }

    pub fn try_result(&mut self) -> Option<Result<Mask, TextKey>> {
        if self.queue.is_empty() || !crate::web::has_budget() {
            return None;
        }
        let key = self.queue.pop_front()?;
        let fonts = self.fonts.get_or_insert_with(page_fonts);
        Some(rasterize(fonts, &mut self.cache, key.clone()).ok_or(key))
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl TextWorker {
    pub fn new() -> Self {
        let (requests, incoming) = channel::<TextKey>();
        let (done, results) = channel();
        std::thread::spawn(move || {
            let mut fonts = FontSystem::new();
            let mut cache = SwashCache::new();
            for key in incoming {
                let answer = rasterize(&mut fonts, &mut cache, key.clone()).ok_or(key);
                if done.send(answer).is_err() {
                    return;
                }
            }
        });
        Self { requests, results, pending: 0 }
    }

    pub fn request(&mut self, key: TextKey) {
        if self.requests.send(key).is_ok() {
            self.pending += 1;
        }
    }

    pub fn try_result(&mut self) -> Option<Result<Mask, TextKey>> {
        let result = self.results.try_recv().ok()?;
        self.pending -= 1;
        Some(result)
    }

    pub fn is_idle(&self) -> bool {
        self.pending == 0
    }
}

/// Places rectangles into rows of a square texture.
#[derive(Debug)]
pub struct Shelves {
    size: u32,
    x: u32,
    y: u32,
    row: u32,
}

impl Shelves {
    pub fn new(size: u32) -> Self {
        Self { size, x: 0, y: 0, row: 0 }
    }

    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            return None;
        }
        if self.x + w > self.size {
            self.x = 0;
            self.y += self.row;
            self.row = 0;
        }
        if self.y + h > self.size {
            return None;
        }
        let at = (self.x, self.y);
        self.x += w;
        self.row = self.row.max(h);
        Some(at)
    }

    pub fn clear(&mut self) {
        (self.x, self.y, self.row) = (0, 0, 0);
    }
}

/// Where a label's text lies in the atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub index: usize,
    /// Centre on screen, in pixels.
    pub center: DVec2,
    pub angle: f64,
}

#[derive(Clone, Copy)]
struct Bounds {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Bounds {
    fn overlaps(&self, o: &Bounds) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
}

pub struct Screen {
    pub basis: Basis,
    pub size: DVec2,
    /// Screen areas labels must stay clear of, as (x0, y0, x1, y1).
    pub reserved: Vec<[f64; 4]>,
}

impl Screen {
    /// A world position's place on screen; `None` when it is behind the camera.
    pub fn project(&self, p: DVec3) -> Option<DVec2> {
        let rel = p - self.basis.pos;
        let depth = rel.dot(self.basis.look);
        if depth <= 1.0 {
            return None;
        }
        let t = (FOV_Y / 2.0).tan();
        let aspect = self.size.x / self.size.y;
        let ndc = DVec2::new(
            rel.dot(self.basis.right) / (depth * t * aspect),
            rel.dot(self.basis.up) / (depth * t),
        );
        Some(DVec2::new((ndc.x + 1.0) / 2.0 * self.size.x, (1.0 - ndc.y) / 2.0 * self.size.y))
    }

    /// The part of the segment from `a` to `b` that lies inside the view, widened by `margin`
    /// (1.0 is the view's own edges); `None` when none of it does. Projecting a point barely in
    /// front of the camera but far to the side throws it far off screen, so a segment seen there
    /// would sweep across the whole view; clipped first, it lands where it is seen.
    pub fn clip(&self, a: DVec3, b: DVec3, margin: f64) -> Option<(DVec3, DVec3)> {
        let bs = &self.basis;
        let t = (FOV_Y / 2.0).tan() * margin;
        let aspect = self.size.x / self.size.y;
        // Inside where the point's offset from the eye has a dot product of at least 0 with each normal.
        let planes = [
            bs.look * (t * aspect) - bs.right,
            bs.look * (t * aspect) + bs.right,
            bs.look * t - bs.up,
            bs.look * t + bs.up,
        ];
        let (mut s0, mut s1) = (0.0f64, 1.0f64);
        let (ra, rb) = (a - bs.pos, b - bs.pos);
        // In front of the camera by at least a metre, as `project` asks.
        for (da, db) in planes.iter().map(|n| (ra.dot(*n), rb.dot(*n))).chain([(ra.dot(bs.look) - 1.0, rb.dot(bs.look) - 1.0)]) {
            if da < 0.0 && db < 0.0 {
                return None;
            }
            if da < 0.0 {
                s0 = s0.max(da / (da - db));
            } else if db < 0.0 {
                s1 = s1.min(da / (da - db));
            }
        }
        (s0 <= s1).then(|| (a.lerp(b, s0), a.lerp(b, s1)))
    }

    fn on_screen(&self, p: DVec2) -> bool {
        p.x >= 0.0 && p.y >= 0.0 && p.x <= self.size.x && p.y <= self.size.y
    }

    /// Whether the ground lies behind a screen position, rather than sky or space.
    fn over_ground(&self, p: DVec2) -> bool {
        let t = (FOV_Y / 2.0).tan();
        let aspect = self.size.x / self.size.y;
        let ndc = DVec2::new(p.x / self.size.x * 2.0 - 1.0, 1.0 - p.y / self.size.y * 2.0);
        let b = &self.basis;
        let dir = (b.look + b.right * (ndc.x * t * aspect) + b.up * (ndc.y * t)).normalize();
        ray_ellipsoid(b.pos, dir, 0.0).is_some()
    }
}

/// How far along a label's own direction its text may be seen flat before it is dropped.
fn min_facing(style: LabelStyle) -> f64 {
    match style {
        LabelStyle::Village | LabelStyle::Local | LabelStyle::Road | LabelStyle::Peak => 0.1,
        _ => 0.02,
    }
}

/// Whether a mountain stands between the camera and a point.
pub fn hidden_by_terrain(from: DVec3, to: DVec3, ground: &dyn Fn(f64, f64) -> Option<f64>) -> bool {
    const STEPS: usize = 10;
    for i in 1..STEPS {
        // Denser near the label, where terrain is most likely to block it.
        let t = 1.0 - (i as f64 / STEPS as f64).powi(2);
        let p = from.lerp(to, t);
        let (lat, lon, h) = ecef_to_geodetic(p);
        if ground(lat, lon).is_some_and(|g| g > h + 15.0) {
            return true;
        }
    }
    false
}

/// A label's world position, standing on the terrain.
pub fn anchor(label: &Label, ground: &dyn Fn(f64, f64) -> Option<f64>) -> DVec3 {
    let h = ground(label.lat, label.lon).unwrap_or(0.0);
    geodetic_to_ecef(label.lat, label.lon, h + 5.0)
}

/// How far from the camera a place name may be shown, in metres on Earth. From afar only
/// countries, states and capitals are named, so the map stays easy to read; cities, towns and
/// villages join in as the camera comes closer.
pub fn max_distance(label: &Label) -> f64 {
    match label.style {
        LabelStyle::State => 2.5e6,
        LabelStyle::City if label.text.starts_with(NATIONAL_CAPITAL_MARK) => f64::INFINITY,
        LabelStyle::City if label.text.starts_with(STATE_CAPITAL_MARK) => 1.6e6,
        LabelStyle::City => 4.0e5,
        LabelStyle::Town => 1.5e5,
        LabelStyle::Village => 5.0e4,
        LabelStyle::Local => 1.5e4,
        LabelStyle::Country | LabelStyle::Sea | LabelStyle::Lake | LabelStyle::Peak | LabelStyle::Road => f64::INFINITY,
        LabelStyle::Region | LabelStyle::Feature => crate::planet_names::reach(label) * crate::body::EARTH.a / crate::body::current().a,
    }
}

/// Whether a label could appear at all in this view, before its text has been rasterised.
pub fn may_show(screen: &Screen, label: &Label) -> bool {
    let p = geodetic_to_ecef(label.lat, label.lon, 0.0);
    let (_, _, up) = enu(label.lat, label.lon);
    let to_camera = screen.basis.pos - p;
    if to_camera.length() > crate::body::scaled(max_distance(label)) {
        return false;
    }
    let facing = to_camera.normalize().dot(up);
    facing > min_facing(label.style) && screen.project(p).is_some_and(|s| screen.on_screen(s))
}

/// How labels meet the window: the globe seen through the camera, or the flat map.
pub trait View {
    fn size(&self) -> DVec2;
    /// Screen areas labels must stay clear of, as (x0, y0, x1, y1).
    fn reserved(&self) -> &[[f64; 4]];
    /// Whether a label could appear at all, before its text has been rasterised.
    fn may_show(&self, label: &Label) -> bool;
    /// Where the label's text is centred on screen.
    fn center(&self, label: &Label) -> Option<DVec2>;
    /// Where a point on the ground is on screen.
    fn point(&self, lat: f64, lon: f64) -> Option<DVec2>;
    /// Whether text with these corners may stand there.
    fn fits(&self, label: &Label, corners: &[DVec2; 4]) -> bool;
}

/// The globe, with the terrain that labels stand on and that can hide them.
pub struct GlobeView<'a> {
    pub screen: &'a Screen,
    pub ground: &'a dyn Fn(f64, f64) -> Option<f64>,
}

impl View for GlobeView<'_> {
    fn size(&self) -> DVec2 {
        self.screen.size
    }
    fn reserved(&self) -> &[[f64; 4]] {
        &self.screen.reserved
    }
    fn may_show(&self, label: &Label) -> bool {
        may_show(self.screen, label)
    }
    fn center(&self, label: &Label) -> Option<DVec2> {
        self.screen.project(anchor(label, self.ground))
    }
    fn point(&self, lat: f64, lon: f64) -> Option<DVec2> {
        self.screen.project(geodetic_to_ecef(lat, lon, 0.0))
    }
    fn fits(&self, label: &Label, corners: &[DVec2; 4]) -> bool {
        // Text must not hang over the edge of the globe or stick out into the sky.
        corners.iter().all(|c| self.screen.over_ground(*c)) && !hidden_by_terrain(self.screen.basis.pos, anchor(label, self.ground), self.ground)
    }
}

/// The flat map. It names places of the kinds the globe names from as far away as the
/// map's scale stands for.
pub struct FlatView<'a> {
    pub map: &'a crate::atlas::AtlasView,
    pub reserved: Vec<[f64; 4]>,
}

impl FlatView<'_> {
    fn on_screen(&self, p: DVec2) -> bool {
        p.x >= 0.0 && p.y >= 0.0 && p.x <= self.map.size.0 && p.y <= self.map.size.1
    }
}

impl View for FlatView<'_> {
    fn size(&self) -> DVec2 {
        DVec2::new(self.map.size.0, self.map.size.1)
    }
    fn reserved(&self) -> &[[f64; 4]] {
        &self.reserved
    }
    fn may_show(&self, label: &Label) -> bool {
        self.map.distance_for_scale() <= crate::body::scaled(max_distance(label))
            && self.point(label.lat, label.lon).is_some_and(|p| self.on_screen(p))
    }
    fn center(&self, label: &Label) -> Option<DVec2> {
        self.point(label.lat, label.lon)
    }
    fn point(&self, lat: f64, lon: f64) -> Option<DVec2> {
        let (x, y) = self.map.screen_of(self.map.project(lat, lon));
        Some(DVec2::new(x, y))
    }
    fn fits(&self, _label: &Label, corners: &[DVec2; 4]) -> bool {
        // Text must not hang over the edge of the map.
        corners.iter().all(|c| self.map.lat_lon_at(c.x, c.y).is_some())
    }
}

/// Chooses labels greedily in priority order, so that none overlap. `candidates` holds a
/// label and its text size, already sorted with the most important first.
pub fn place(view: &dyn View, candidates: &[(&Label, DVec2)]) -> Vec<Placed> {
    let size = view.size();
    let mut placed = Vec::new();
    let mut taken: Vec<Bounds> = view.reserved().iter().map(|r| Bounds { x0: r[0], y0: r[1], x1: r[2], y1: r[3] }).collect();
    let mut seen: HashMap<&str, Vec<DVec2>> = HashMap::new();
    for (index, (label, text_size)) in candidates.iter().enumerate() {
        if placed.len() >= MAX_PLACED {
            break;
        }
        if !view.may_show(label) {
            continue;
        }
        let Some(center) = view.center(label) else { continue };
        let half = *text_size / 2.0;

        let mut angle = 0.0;
        if let Some([a, b]) = label.along {
            let (Some(pa), Some(pb)) = (view.point(a.0, a.1), view.point(b.0, b.1)) else {
                continue;
            };
            let chord = pb - pa;
            // The chord is a fifth of the road, so this drops labels longer than their road.
            if chord.length() * 5.0 < text_size.x {
                continue;
            }
            angle = chord.y.atan2(chord.x);
            if angle > std::f64::consts::FRAC_PI_2 {
                angle -= std::f64::consts::PI;
            } else if angle < -std::f64::consts::FRAC_PI_2 {
                angle += std::f64::consts::PI;
            }
        }
        let (sin, cos) = angle.sin_cos();
        let extent = DVec2::new(
            half.x * cos.abs() + half.y * sin.abs() + PADDING,
            half.x * sin.abs() + half.y * cos.abs() + PADDING,
        );
        let bounds = Bounds {
            x0: center.x - extent.x,
            y0: center.y - extent.y,
            x1: center.x + extent.x,
            y1: center.y + extent.y,
        };
        if bounds.x0 < 0.0 || bounds.y0 < 0.0 || bounds.x1 > size.x || bounds.y1 > size.y {
            continue;
        }
        if taken.iter().any(|t| t.overlaps(&bounds)) {
            continue;
        }
        if seen
            .get(label.text.as_str())
            .is_some_and(|ps| ps.iter().any(|p| p.distance(center) < SAME_TEXT_GAP))
        {
            continue;
        }
        let corners = [
            DVec2::new(bounds.x0, bounds.y0),
            DVec2::new(bounds.x1, bounds.y0),
            DVec2::new(bounds.x0, bounds.y1),
            DVec2::new(bounds.x1, bounds.y1),
        ];
        if !view.fits(label, &corners) {
            continue;
        }
        taken.push(bounds);
        seen.entry(label.text.as_str()).or_default().push(center);
        // Axis-aligned text lands on whole pixels so that it stays sharp.
        let center = if angle == 0.0 {
            (center - half).round() + half
        } else {
            center
        };
        placed.push(Placed { index, center, angle });
    }
    placed
}

/// Trims text to a length the atlas can hold. Empty text gets `None`.
pub fn usable_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(MAX_TEXT_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    const NO_GROUND: &dyn Fn(f64, f64) -> Option<f64> = &|_, _| None;
    const LAT: f64 = 0.82;
    const LON: f64 = 0.2;

    fn screen(distance: f64, tilt: f64) -> Screen {
        let cam = Camera { lat: LAT, lon: LON, target_h: 0.0, distance, heading: 0.0, tilt, sky: 0.0 };
        Screen { basis: cam.basis(), size: DVec2::new(1200.0, 800.0), reserved: Vec::new() }
    }

    fn label(text: &str, lat: f64, lon: f64) -> Label {
        Label { text: text.into(), style: LabelStyle::City, priority: 0, lat, lon, along: None }
    }

    fn flat_map(distance: f64) -> crate::atlas::AtlasView {
        let mut v = crate::atlas::AtlasView::whole(crate::atlas::Projection::EqualEarth, 0.0, (1200.0, 800.0));
        v.match_globe(LAT, LON, distance);
        v
    }

    #[test]
    fn the_flat_map_names_places_where_they_are_drawn() {
        let map = flat_map(3.0e5);
        let view = FlatView { map: &map, reserved: Vec::new() };
        let a = label("Here", LAT, LON);
        let placed = place(&view, &[(&a, DVec2::new(40.0, 16.0))]);
        assert_eq!(placed.len(), 1);
        let (x, y) = map.screen_of(map.project(LAT, LON));
        assert!(placed[0].center.distance(DVec2::new(x, y)) < 1.0);
        // Overlapping names: only the first, more important one is kept.
        let b = label("There", LAT, LON + 0.00001);
        assert_eq!(place(&view, &[(&a, DVec2::new(40.0, 16.0)), (&b, DVec2::new(40.0, 16.0))]).len(), 1);
    }

    #[test]
    fn the_flat_map_leaves_out_small_places_when_zoomed_out() {
        let village = Label { style: LabelStyle::Village, ..label("Dorf", LAT, LON) };
        let size = DVec2::new(40.0, 16.0);
        let far = flat_map(2.0e6);
        assert!(place(&FlatView { map: &far, reserved: Vec::new() }, &[(&village, size)]).is_empty());
        let near = flat_map(2.0e4);
        assert_eq!(place(&FlatView { map: &near, reserved: Vec::new() }, &[(&village, size)]).len(), 1);
    }

    #[test]
    fn view_point_projects_to_the_centre() {
        let s = screen(100_000.0, 0.5);
        let c = geodetic_to_ecef(LAT, LON, 0.0);
        let p = s.project(c).unwrap();
        assert!((p - DVec2::new(600.0, 400.0)).length() < 1e-6);
    }

    #[test]
    fn east_appears_to_the_right_and_north_upwards() {
        let s = screen(100_000.0, 0.0);
        let east = s.project(geodetic_to_ecef(LAT, LON + 0.01, 0.0)).unwrap();
        let north = s.project(geodetic_to_ecef(LAT + 0.01, LON, 0.0)).unwrap();
        assert!(east.x > 600.0);
        assert!(north.y < 400.0);
    }

    #[test]
    fn overlapping_labels_keep_the_first() {
        let s = screen(100_000.0, 0.0);
        let (a, b) = (label("First", LAT, LON), label("Second", LAT, LON + 0.0001));
        let far = label("Far", LAT + 0.005, LON);
        let size = DVec2::new(80.0, 20.0);
        let placed = place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, size), (&b, size), (&far, size)]);
        let chosen: Vec<_> = placed.iter().map(|p| p.index).collect();
        assert_eq!(chosen, [0, 2]);
    }

    #[test]
    fn labels_stay_clear_of_reserved_areas_and_screen_edges() {
        let mut s = screen(100_000.0, 0.0);
        let a = label("A", LAT, LON);
        let size = DVec2::new(40.0, 20.0);
        assert_eq!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, size)]).len(), 1);
        s.reserved.push([500.0, 350.0, 700.0, 450.0]);
        assert!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, size)]).is_empty());
        s.reserved.clear();
        assert!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, DVec2::new(1300.0, 20.0))]).is_empty());
    }

    #[test]
    fn far_side_and_off_screen_labels_are_skipped() {
        let s = screen(1_000_000.0, 0.0);
        let far_side = label("Far", -LAT, LON + std::f64::consts::PI);
        assert!(!may_show(&s, &far_side));
        let close = screen(1_000.0, 0.0);
        assert!(!may_show(&close, &label("Off", LAT + 0.2, LON)));
        assert!(may_show(&close, &label("Here", LAT, LON)));
    }

    #[test]
    fn from_afar_only_countries_states_and_capitals_are_named() {
        let s = screen(7.0e5, 0.0);
        let with = |style, text: &str| Label { style, ..label(text, LAT, LON) };
        assert!(may_show(&s, &with(LabelStyle::Country, "Germany")));
        assert!(may_show(&s, &with(LabelStyle::State, "Bavaria")));
        assert!(may_show(&s, &with(LabelStyle::City, "\u{2605} Berlin")));
        assert!(may_show(&s, &with(LabelStyle::City, "\u{2b51} Munich")));
        assert!(!may_show(&s, &with(LabelStyle::City, "Celle")));
        assert!(!may_show(&s, &with(LabelStyle::Town, "Stendal")));
        let close = screen(1.0e5, 0.0);
        assert!(may_show(&close, &with(LabelStyle::City, "Celle")));
        assert!(may_show(&close, &with(LabelStyle::Town, "Stendal")));
        assert!(!may_show(&close, &with(LabelStyle::Village, "Hof")));
        let globe = screen(2.0e7, 0.0);
        assert!(may_show(&globe, &with(LabelStyle::City, "\u{2605} Berlin")));
        assert!(!may_show(&globe, &with(LabelStyle::City, "\u{2b51} Munich")));
    }

    #[test]
    fn labels_at_the_globes_edge_must_fit_on_it() {
        let s = screen(2.2e7, 0.0);
        let size = DVec2::new(160.0, 20.0);
        // Far to the east of the view point, so close to the globe's right-hand edge.
        let country = |text: &str, lat, lon| Label { style: LabelStyle::Country, ..label(text, lat, lon) };
        let near_edge = country("Edge", LAT - 0.4, LON + 1.45);
        assert!(may_show(&s, &near_edge), "the place itself is in view");
        assert!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&near_edge, size)]).is_empty());
        let middle = country("Middle", LAT, LON);
        assert_eq!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&middle, size)]).len(), 1);
    }

    #[test]
    fn same_name_is_not_repeated_nearby() {
        let s = screen(100_000.0, 0.0);
        let (a, b) = (label("Inn", LAT, LON), label("Inn", LAT, LON + 0.002));
        let size = DVec2::new(30.0, 16.0);
        assert_eq!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, size), (&b, size)]).len(), 1);
    }

    #[test]
    fn mountains_hide_what_is_behind_them() {
        let s = screen(20_000.0, 1.3);
        let a = label("Valley", LAT, LON);
        let size = DVec2::new(40.0, 16.0);
        assert_eq!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, size)]).len(), 1);
        // A wall between the label and the camera, which stands to the south.
        let wall: &dyn Fn(f64, f64) -> Option<f64> =
            &|lat, _| Some(if lat < LAT - 0.0002 && lat > LAT - 0.01 { 9000.0 } else { 0.0 });
        assert!(place(&GlobeView { screen: &s, ground: wall }, &[(&a, size)]).is_empty());
    }

    fn road_label(a: (f64, f64), b: (f64, f64)) -> Label {
        Label {
            text: "Road".into(),
            style: LabelStyle::Road,
            priority: 0,
            lat: (a.0 + b.0) / 2.0,
            lon: (a.1 + b.1) / 2.0,
            along: Some([a, b]),
        }
    }

    #[test]
    fn road_labels_follow_the_road_and_stay_upright() {
        let s = screen(5_000.0, 0.0);
        // North-east going, seen north up, so rising to the right.
        let l = road_label((LAT - 0.0005, LON - 0.0005), (LAT + 0.0005, LON + 0.0005));
        let p = place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&l, DVec2::new(30.0, 12.0))]);
        assert_eq!(p.len(), 1);
        assert!(p[0].angle < 0.0 && p[0].angle > -std::f64::consts::FRAC_PI_2, "{}", p[0].angle);
        // The same road described in the opposite direction gets the same angle.
        let r = road_label((LAT + 0.0005, LON + 0.0005), (LAT - 0.0005, LON - 0.0005));
        let q = place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&r, DVec2::new(30.0, 12.0))]);
        assert!((q[0].angle - p[0].angle).abs() < 1e-9);
    }

    #[test]
    fn road_labels_longer_than_the_road_are_dropped() {
        let s = screen(100_000.0, 0.0);
        let l = road_label((LAT, LON), (LAT, LON + 0.00001));
        assert!(place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&l, DVec2::new(100.0, 12.0))]).is_empty());
    }

    #[test]
    fn horizontal_labels_land_on_whole_pixels() {
        let s = screen(100_000.0, 0.0);
        let a = label("A", LAT + 0.001, LON + 0.001);
        let size = DVec2::new(41.0, 17.0);
        let p = place(&GlobeView { screen: &s, ground: NO_GROUND }, &[(&a, size)])[0];
        let corner = p.center - size / 2.0;
        assert_eq!(corner, corner.round());
    }

    #[test]
    fn shelves_fill_rows_and_report_when_full() {
        let mut s = Shelves::new(100);
        assert_eq!(s.alloc(60, 10), Some((0, 0)));
        assert_eq!(s.alloc(30, 20), Some((60, 0)));
        assert_eq!(s.alloc(30, 10), Some((0, 20)), "next row starts below the tallest");
        assert_eq!(s.alloc(101, 1), None);
        assert_eq!(s.alloc(90, 80), None);
        s.clear();
        assert_eq!(s.alloc(90, 80), Some((0, 0)));
    }

    #[test]
    fn halo_surrounds_the_text() {
        let (w, h) = (9, 9);
        let mut coverage = vec![0u8; w * h];
        coverage[4 * w + 4] = 200;
        let out = with_halo(w, h, &coverage);
        assert_eq!(out[(4 * w + 4) * 2], 200);
        assert_eq!(out[(4 * w + 6) * 2 + 1], 200, "within the halo");
        assert_eq!(out[(4 * w + 6) * 2], 0, "but not text");
        assert_eq!(out[(4 * w + 8) * 2 + 1], 0, "beyond the halo");
    }

    #[test]
    fn long_and_empty_text() {
        assert_eq!(usable_text("   "), None);
        assert_eq!(usable_text(" Wien ").as_deref(), Some("Wien"));
        assert_eq!(usable_text(&"ä".repeat(100)).unwrap().chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn real_text_rasterises_with_system_fonts() {
        let mut fonts = FontSystem::new();
        let mut cache = SwashCache::new();
        for text in ["Innsbruck", "東京", "القاهرة", "Москва", "▲ Olperer 3476 m"] {
            let key = TextKey { text: text.into(), style: LabelStyle::City, scale: 1 };
            let Some(mask) = rasterize(&mut fonts, &mut cache, key) else {
                // A machine without fonts can show no labels, which is not an error here.
                assert!(fonts.db().is_empty(), "{text} produced nothing");
                continue;
            };
            assert_eq!(mask.pixels.len(), (mask.width * mask.height * 2) as usize);
            assert!(mask.pixels.chunks(2).any(|p| p[0] > 128), "{text} has no visible pixels");
        }
    }
}
