//! Draws imported lines, areas and points into a tile's overlay image, so that they follow
//! the terrain like the roads do. Works on Web Mercator tiles, which is what Earth uses.

use crate::geo::{lat_to_merc_v, lon_to_merc_u, TileKey};
use crate::import::{Imported, Item, Mark, Outline, Pos, Shape};
use crate::overlay::OVERLAY_SIZE;
use std::sync::Arc;
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

/// Overlay texels per screen pixel where a tile is about to split, as for the roads.
const TEXELS_PER_PIXEL: f32 = 2.0;
/// Where drawing is cut off, in texels beyond the tile's edge.
const MARGIN: f64 = 64.0;
/// Points closer together than this, in texels, are drawn as one.
const MIN_STEP: f64 = 0.6;
/// The Mercator map ends here.
const MAX_LAT: f64 = 1.484_422_229_745_332_4; // 85.0511 degrees

const LINE: [u8; 4] = [255, 72, 56, 240];
const AREA_EDGE: [u8; 4] = [255, 205, 60, 240];
const AREA_FILL: [u8; 4] = [255, 205, 60, 70];
const DOT: [u8; 4] = [255, 255, 255, 255];
const DOT_RING: [u8; 4] = [210, 40, 40, 255];

/// Whether anything imported can show in the tile.
pub fn touches(docs: &[Arc<Imported>], tile: TileKey) -> bool {
    docs.iter().flat_map(|d| d.items.iter()).any(|i| item_touches(i, tile))
}

fn item_touches(item: &Item, tile: TileKey) -> bool {
    let [west, east, north, south] = tile_box(tile);
    let [s, w, n, e] = item.extent;
    let (u0, u1) = (lon_to_merc_u(w), lon_to_merc_u(e));
    let (v0, v1) = (lat_to_merc_v(n.min(MAX_LAT)), lat_to_merc_v(s.max(-MAX_LAT)));
    // An item that spans most of the world may wrap round the back.
    ((u1 - u0 > 0.5) || (u1 >= west && u0 <= east)) && v1 >= north && v0 <= south
}

/// The tile's west, east, north and south edges in map units (0 to 1), widened a little.
fn tile_box(tile: TileKey) -> [f64; 4] {
    let n = (1u64 << tile.z) as f64;
    let pad = MARGIN / OVERLAY_SIZE as f64 / n;
    [tile.x as f64 / n - pad, (tile.x + 1) as f64 / n + pad, tile.y as f64 / n - pad, (tile.y + 1) as f64 / n + pad]
}

/// Texel coordinates in `tile`, with longitude unwrapped along the sequence so that a line
/// over the antimeridian does not run round the world. Reports whether it did.
fn project(tile: TileKey, points: &[Pos]) -> (Vec<(f64, f64)>, bool) {
    let n = (1u64 << tile.z) as f64;
    let mut out = Vec::with_capacity(points.len());
    let mut wrapped = false;
    let (mut offset, mut last_u) = (0.0, None::<f64>);
    for p in points {
        let u = lon_to_merc_u(p.lon);
        if let Some(l) = last_u {
            if u + offset - l > 0.5 {
                offset -= 1.0;
                wrapped = true;
            } else if u + offset - l < -0.5 {
                offset += 1.0;
                wrapped = true;
            }
        }
        last_u = Some(u + offset);
        let v = lat_to_merc_v(p.lat.clamp(-MAX_LAT, MAX_LAT));
        out.push((((u + offset) * n - tile.x as f64) * OVERLAY_SIZE as f64, (v * n - tile.y as f64) * OVERLAY_SIZE as f64));
    }
    (out, wrapped)
}

/// Drops points that land on the previous one, which matters for long tracks seen from afar.
fn thin(points: &[(f64, f64)], keep_last: bool) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len().min(4096));
    for &p in points {
        if out.last().is_none_or(|l| (l.0 - p.0).hypot(l.1 - p.1) >= MIN_STEP) {
            out.push(p);
        }
    }
    if keep_last {
        if let (Some(&last), Some(l)) = (points.last(), out.last_mut()) {
            *l = last;
        }
    }
    out
}

/// Cuts a segment to the square from -MARGIN to OVERLAY_SIZE + MARGIN (Liang and Barsky).
fn clip_segment(a: (f64, f64), b: (f64, f64)) -> Option<((f64, f64), (f64, f64))> {
    let (lo, hi) = (-MARGIN, OVERLAY_SIZE as f64 + MARGIN);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0 - lo), (dx, hi - a.0), (-dy, a.1 - lo), (dy, hi - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some(((a.0 + t0 * dx, a.1 + t0 * dy), (a.0 + t1 * dx, a.1 + t1 * dy)))
}

/// Cuts a ring to the same square (Sutherland and Hodgman).
fn clip_ring(ring: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let (lo, hi) = (-MARGIN, OVERLAY_SIZE as f64 + MARGIN);
    let mut points = ring.to_vec();
    for edge in 0..4 {
        let inside = |p: (f64, f64)| match edge {
            0 => p.0 >= lo,
            1 => p.0 <= hi,
            2 => p.1 >= lo,
            _ => p.1 <= hi,
        };
        let cut = |a: (f64, f64), b: (f64, f64)| {
            let (limit, horizontal) = match edge {
                0 => (lo, false),
                1 => (hi, false),
                2 => (lo, true),
                _ => (hi, true),
            };
            if horizontal {
                let t = (limit - a.1) / (b.1 - a.1);
                (a.0 + t * (b.0 - a.0), limit)
            } else {
                let t = (limit - a.0) / (b.0 - a.0);
                (limit, a.1 + t * (b.1 - a.1))
            }
        };
        let mut next = Vec::with_capacity(points.len());
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            match (inside(a), inside(b)) {
                (true, true) => next.push(b),
                (true, false) => next.push(cut(a, b)),
                (false, true) => {
                    next.push(cut(a, b));
                    next.push(b);
                }
                (false, false) => {}
            }
        }
        points = next;
        if points.is_empty() {
            break;
        }
    }
    points
}

fn paint(color: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(color[0], color[1], color[2], color[3]));
    p.anti_alias = true;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() }
}

fn draw_line(pixmap: &mut Pixmap, points: &[(f64, f64)]) {
    let mut path = PathBuilder::new();
    let mut pen: Option<(f64, f64)> = None;
    for w in thin(points, true).windows(2) {
        let Some((a, b)) = clip_segment(w[0], w[1]) else {
            pen = None;
            continue;
        };
        if pen != Some(a) {
            path.move_to(a.0 as f32, a.1 as f32);
        }
        path.line_to(b.0 as f32, b.1 as f32);
        pen = Some(b);
    }
    let Some(path) = path.finish() else { return };
    let width = 2.4 * TEXELS_PER_PIXEL;
    pixmap.stroke_path(&path, &paint([20, 20, 30, 190]), &stroke(width + 1.6 * TEXELS_PER_PIXEL), Transform::identity(), None);
    pixmap.stroke_path(&path, &paint(LINE), &stroke(width), Transform::identity(), None);
}

fn draw_area(pixmap: &mut Pixmap, rings: &[Vec<(f64, f64)>], style: Option<Outline>) {
    let mut path = PathBuilder::new();
    for ring in rings {
        let ring = clip_ring(&thin(ring, false));
        let Some((first, rest)) = ring.split_first() else { continue };
        if rest.len() < 2 {
            continue;
        }
        path.move_to(first.0 as f32, first.1 as f32);
        for p in rest {
            path.line_to(p.0 as f32, p.1 as f32);
        }
        path.close();
    }
    let Some(path) = path.finish() else { return };
    let style = style.unwrap_or(Outline { edge: AREA_EDGE, fill: AREA_FILL, width: 1.8, dashed: false });
    if style.fill[3] > 0 {
        if style.dashed {
            hatch(pixmap, &path, style.fill);
        } else {
            pixmap.fill_path(&path, &paint(style.fill), FillRule::EvenOdd, Transform::identity(), None);
        }
    }
    let width = style.width * TEXELS_PER_PIXEL;
    let halo = [20, 20, 30, (style.edge[3] as f32 * 0.7) as u8];
    let mut edge = stroke(width);
    let mut under = stroke(width + 1.2 * TEXELS_PER_PIXEL);
    if style.dashed {
        let dash = StrokeDash::new(vec![DASH * TEXELS_PER_PIXEL, GAP * TEXELS_PER_PIXEL], 0.0);
        edge.line_cap = LineCap::Butt;
        under.line_cap = LineCap::Butt;
        (edge.dash, under.dash) = (dash.clone(), dash);
    }
    pixmap.stroke_path(&path, &paint(halo), &under, Transform::identity(), None);
    pixmap.stroke_path(&path, &paint(style.edge), &edge, Transform::identity(), None);
}

/// Dashes and gaps of a dashed edge, in pixels on the screen.
const DASH: f32 = 7.0;
const GAP: f32 = 4.0;
/// Hatching: the distance between its lines and their width, in pixels on the screen.
const HATCH_STEP: f32 = 7.0;
const HATCH_WIDTH: f32 = 2.6;

/// Fills an area with diagonal lines instead of a flat colour. The lines are a little stronger
/// than the flat fill would be, since they cover only part of the area; the rest gets a faint
/// wash of the colour, so that the area still reads as one.
fn hatch(pixmap: &mut Pixmap, path: &tiny_skia::Path, fill: [u8; 4]) {
    let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) else { return };
    mask.fill_path(path, FillRule::EvenOdd, true, Transform::identity());
    let wash = [fill[0], fill[1], fill[2], fill[3] / 3];
    pixmap.fill_path(path, &paint(wash), FillRule::EvenOdd, Transform::identity(), None);
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let step = HATCH_STEP * TEXELS_PER_PIXEL;
    let mut lines = PathBuilder::new();
    // Lines running up to the right, placed by x + y so that neighbouring tiles of a zoom meet.
    let mut k = 0.0;
    while k <= w + h {
        lines.move_to(k, 0.0);
        lines.line_to(k - h, h);
        k += step;
    }
    let Some(lines) = lines.finish() else { return };
    let strong = [fill[0], fill[1], fill[2], (fill[3] as u16 * 2).min(255) as u8];
    let mut s = stroke(HATCH_WIDTH * TEXELS_PER_PIXEL);
    s.line_cap = LineCap::Butt;
    pixmap.stroke_path(&lines, &paint(strong), &s, Transform::identity(), Some(&mask));
}

fn draw_dot(pixmap: &mut Pixmap, (x, y): (f64, f64), mark: Option<Mark>) {
    let size = OVERLAY_SIZE as f64 + MARGIN;
    if x < -MARGIN || y < -MARGIN || x > size || y > size {
        return;
    }
    let mut path = PathBuilder::new();
    let (outer, inner, ring, fill) = match mark {
        // A sized marker is a translucent disc with a dark ring and a bright centre.
        Some(m) => (m.radius * TEXELS_PER_PIXEL, m.radius * 0.55 * TEXELS_PER_PIXEL, [20, 20, 30, 230], [m.colour[0], m.colour[1], m.colour[2], m.colour[3]]),
        None => (4.5 * TEXELS_PER_PIXEL, 2.8 * TEXELS_PER_PIXEL, DOT_RING, DOT),
    };
    path.push_circle(x as f32, y as f32, outer);
    let Some(path) = path.finish() else { return };
    pixmap.fill_path(&path, &paint(ring), FillRule::Winding, Transform::identity(), None);
    let mut body = PathBuilder::new();
    body.push_circle(x as f32, y as f32, if mark.is_some() { (outer - 1.2 * TEXELS_PER_PIXEL).max(1.0) } else { 0.0 });
    if mark.is_some() {
        if let Some(body) = body.finish() {
            let mut p = paint(fill);
            p.set_color(Color::from_rgba8(fill[0], fill[1], fill[2], fill[3] / 2));
            pixmap.fill_path(&body, &p, FillRule::Winding, Transform::identity(), None);
        }
    }
    let mut centre = PathBuilder::new();
    centre.push_circle(x as f32, y as f32, inner);
    if let Some(centre) = centre.finish() {
        pixmap.fill_path(&centre, &paint(fill), FillRule::Winding, Transform::identity(), None);
    }
}

/// Draws what is imported onto `pixmap`, which covers `tile`. Areas first, then lines, then
/// points, so that points stay on top.
pub fn draw(pixmap: &mut Pixmap, docs: &[Arc<Imported>], tile: TileKey) {
    let items = || docs.iter().flat_map(|d| d.items.iter()).filter(|i| item_touches(i, tile));
    for item in items() {
        if let Shape::Polygon(rings) = &item.shape {
            draw_projected(pixmap, tile, rings.iter().map(Vec::as_slice), |p, r| draw_area(p, r, item.outline));
        }
    }
    for item in items() {
        if let Shape::Line(line) = &item.shape {
            draw_projected(pixmap, tile, std::iter::once(line.as_slice()), |p, r| {
                for l in r {
                    draw_line(p, l);
                }
            });
        }
    }
    for item in items() {
        if let Shape::Point(p) = &item.shape {
            draw_projected(pixmap, tile, std::iter::once(std::slice::from_ref(p)), |pm, r| draw_dot(pm, r[0][0], item.mark));
        }
    }
}

/// Projects each sequence, and calls `f` with the result once for the plain position and,
/// for sequences that cross the antimeridian, for the copies a world to either side.
fn draw_projected<'a>(
    pixmap: &mut Pixmap,
    tile: TileKey,
    sequences: impl Iterator<Item = &'a [Pos]>,
    f: impl Fn(&mut Pixmap, &[Vec<(f64, f64)>]),
) {
    let mut projected = Vec::new();
    let mut wrapped = false;
    for s in sequences {
        let (p, w) = project(tile, s);
        wrapped |= w;
        projected.push(p);
    }
    f(pixmap, &projected);
    if wrapped {
        let world = (1u64 << tile.z) as f64 * OVERLAY_SIZE as f64;
        for shift in [-world, world] {
            let moved: Vec<Vec<(f64, f64)>> = projected.iter().map(|s| s.iter().map(|p| (p.0 + shift, p.1)).collect()).collect();
            f(pixmap, &moved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import;

    fn pos(lat: f64, lon: f64) -> Pos {
        Pos { lat: lat.to_radians(), lon: lon.to_radians(), alt: None }
    }

    fn docs(items: Vec<(&str, Shape)>) -> Vec<Arc<Imported>> {
        let items = items.into_iter().map(|(n, s)| import::Item::new(n, s)).collect();
        vec![Arc::new(Imported { name: "t".into(), items, truncated: false })]
    }

    fn alpha(p: &Pixmap, x: u32, y: u32) -> u8 {
        p.data()[((y * OVERLAY_SIZE + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_line_is_drawn_where_it_runs() {
        // The tile at zoom 3 covering 0..45 degrees east and the equator northwards.
        let tile = TileKey::new(3, 4, 3);
        let d = docs(vec![("l", Shape::Line(vec![pos(1.0, 1.0), pos(1.0, 44.0)]))]);
        assert!(touches(&d, tile));
        let mut pm = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE).unwrap();
        draw(&mut pm, &d, tile);
        // Close to the bottom edge of the tile, where latitude 1 lies.
        let y = (0..OVERLAY_SIZE).find(|y| alpha(&pm, 256, *y) > 200).expect("drawn");
        assert!(y > 480, "{y}");
        assert_eq!(alpha(&pm, 256, 100), 0);
        assert!(!touches(&d, TileKey::new(3, 0, 3)), "a tile elsewhere has nothing");
    }

    #[test]
    fn an_area_is_filled_except_for_its_hole() {
        let tile = TileKey::new(3, 4, 3);
        let ring = |a: f64, b: f64| vec![pos(a, a), pos(a, b), pos(b, b), pos(b, a), pos(a, a)];
        let d = docs(vec![("a", Shape::Polygon(vec![ring(5.0, 40.0), ring(15.0, 30.0)]))]);
        let mut pm = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE).unwrap();
        draw(&mut pm, &d, tile);
        let inside = alpha(&pm, 100, 420);
        let hole = alpha(&pm, 256, 250);
        assert!(inside > 30 && hole == 0, "{inside} {hole}");
    }

    #[test]
    fn a_point_is_a_dot() {
        let tile = TileKey::new(3, 4, 3);
        let d = docs(vec![("p", Shape::Point(pos(22.5, 22.5)))]);
        let mut pm = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE).unwrap();
        draw(&mut pm, &d, tile);
        let n = pm.data().chunks(4).filter(|p| p[3] > 0).count();
        assert!(n > 100 && n < 3000, "{n}");
    }

    #[test]
    fn a_line_over_the_antimeridian_is_cut_not_wrapped_round_the_world() {
        let d = docs(vec![("l", Shape::Line(vec![pos(0.5, 179.0), pos(0.5, -179.0)]))]);
        // The easternmost tile at zoom 2 has the first part, the westernmost the second.
        let (east, west) = (TileKey::new(2, 3, 2), TileKey::new(2, 0, 2));
        let middle = TileKey::new(2, 2, 2);
        for (tile, expect) in [(east, true), (west, true), (middle, false)] {
            let mut pm = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE).unwrap();
            draw(&mut pm, &d, tile);
            assert_eq!(pm.data().chunks(4).any(|p| p[3] > 0), expect, "{tile:?}");
        }
    }

    #[test]
    fn segments_and_rings_are_cut_at_the_edge() {
        assert!(clip_segment((-1000.0, 10.0), (-900.0, 20.0)).is_none());
        let (a, b) = clip_segment((-1000.0, 256.0), (2000.0, 256.0)).unwrap();
        assert!(a.0 >= -MARGIN - 1e-9 && b.0 <= OVERLAY_SIZE as f64 + MARGIN + 1e-9);
        let ring = clip_ring(&[(-1e7, -1e7), (1e7, -1e7), (1e7, 1e7), (-1e7, 1e7)]);
        assert!(ring.iter().all(|p| p.0.abs() < 1e4 && p.1.abs() < 1e4) && ring.len() >= 4);
        assert!(clip_ring(&[(-1e7, 0.0), (-1e7, 5.0), (-9e6, 5.0)]).is_empty());
    }

    #[test]
    fn thinning_keeps_the_ends() {
        let pts: Vec<_> = (0..1000).map(|i| (i as f64 * 0.01, 0.0)).collect();
        let t = thin(&pts, true);
        assert!(t.len() < 30 && t[0] == pts[0] && *t.last().unwrap() == *pts.last().unwrap());
    }

    #[test]
    fn the_whole_world_wrap_does_not_hide_items_from_distant_tiles() {
        let d = docs(vec![("w", Shape::Line(vec![pos(0.0, -170.0), pos(0.0, 170.0)]))]);
        assert!(touches(&d, TileKey::new(1, 0, 0)) || touches(&d, TileKey::new(1, 0, 1)));
    }
}
