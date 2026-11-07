//! Draws roads and boundaries into a transparent image for one globe tile. The image is
//! laid over the tile's imagery, so lines follow the terrain without any 3D geometry.

use crate::geo::TileKey;
use crate::import::Imported;
use crate::import_draw;
use std::sync::Arc;
use crate::vector::{Line, RoadClass, VectorTile};
use tiny_skia::{Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

pub const OVERLAY_SIZE: u32 = 512;
/// Overlay texels per screen pixel at the point where a tile is about to split in two.
const TEXELS_PER_PIXEL: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineStyle {
    /// Width on screen, in pixels.
    pub width: f32,
    pub color: [u8; 4],
    pub casing: bool,
    pub dash: Option<[f32; 2]>,
}

/// How a road looks at a tile zoom level. `None` hides it.
pub fn road_style(class: RoadClass, z: u8, tunnel: bool) -> Option<LineStyle> {
    let (from_z, base, color, casing) = match class {
        RoadClass::Motorway => (5, 2.0, [240, 130, 70, 235], true),
        RoadClass::Trunk => (6, 1.8, [245, 165, 85, 230], true),
        RoadClass::Primary => (8, 1.6, [250, 205, 100, 230], true),
        RoadClass::Secondary => (9, 1.3, [250, 230, 140, 225], true),
        RoadClass::Tertiary => (10, 1.1, [250, 245, 200, 215], true),
        RoadClass::Minor => (12, 0.9, [255, 255, 255, 200], false),
        RoadClass::Service => (15, 0.6, [255, 255, 255, 170], false),
        RoadClass::Path => (15, 0.6, [255, 235, 200, 150], false),
        RoadClass::Rail => (10, 0.8, [215, 215, 225, 215], false),
        RoadClass::Ferry => (8, 0.8, [130, 190, 255, 200], false),
    };
    if z < from_z {
        return None;
    }
    let width = base * ((z as f32 - 4.0) / 8.0).clamp(0.5, 1.6);
    let dash = match class {
        RoadClass::Path => Some([3.0, 2.0]),
        RoadClass::Rail => Some([4.0, 2.5]),
        RoadClass::Ferry => Some([5.0, 4.0]),
        _ if tunnel => Some([3.0, 2.0]),
        _ => None,
    };
    // Tunnels run underground, so they stay quieter than the roads above them.
    let color = if tunnel { [color[0], color[1], color[2], 120] } else { color };
    Some(LineStyle { width, color, casing: casing && !tunnel, dash })
}

/// How a boundary looks at a tile zoom level. `None` hides it.
pub fn boundary_style(admin_level: i64, z: u8, disputed: bool) -> Option<LineStyle> {
    let (width, color) = match admin_level {
        ..=2 => (1.1, [255, 255, 255, 215]),
        3..=4 if z >= 5 => (0.8, [235, 225, 255, 165]),
        _ => return None,
    };
    let dash = (disputed || admin_level > 2).then_some([4.0, 3.0]);
    Some(LineStyle { width, color, casing: false, dash })
}

/// The transform from vector tile coordinates to overlay texels.
fn placement(tile: TileKey, source: TileKey) -> (f32, f32, f32) {
    let (u, v, size) = tile.rect_in(source);
    let scale = OVERLAY_SIZE as f64 / size;
    (scale as f32, (-u * scale) as f32, (-v * scale) as f32)
}

fn visible(line: &Line, (scale, dx, dy): (f32, f32, f32), margin: f32) -> bool {
    let size = OVERLAY_SIZE as f32 + margin;
    let mut sides = [true; 4];
    for p in line {
        let (x, y) = (p[0] * scale + dx, p[1] * scale + dy);
        sides[0] &= x < -margin;
        sides[1] &= x > size;
        sides[2] &= y < -margin;
        sides[3] &= y > size;
    }
    !sides.iter().any(|s| *s)
}

fn stroke(
    pixmap: &mut Pixmap,
    lines: &[&Line],
    place: (f32, f32, f32),
    width: f32,
    color: [u8; 4],
    dash: Option<[f32; 2]>,
) {
    let (scale, dx, dy) = place;
    let mut builder = PathBuilder::new();
    for line in lines {
        let mut points = line.iter().map(|p| (p[0] * scale + dx, p[1] * scale + dy));
        let Some((x, y)) = points.next() else { continue };
        builder.move_to(x, y);
        for (x, y) in points {
            builder.line_to(x, y);
        }
    }
    let Some(path) = builder.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(color[0], color[1], color[2], color[3]));
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        line_cap: if dash.is_some() { LineCap::Butt } else { LineCap::Round },
        line_join: LineJoin::Round,
        dash: dash.and_then(|d| StrokeDash::new(vec![d[0] * width, d[1] * width], 0.0)),
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// Draws the lines for `tile`, whose data comes from the `source` vector tile. Returns
/// premultiplied RGBA, or `None` when nothing was drawn.
pub fn render(data: &VectorTile, tile: TileKey, source: TileKey, imports: &[Arc<Imported>]) -> Option<Vec<u8>> {
    let place = placement(tile, source);
    let margin = 32.0;
    let z = tile.z;

    let mut groups: Vec<(i32, LineStyle, Vec<&Line>)> = Vec::new();
    for road in &data.roads {
        let Some(style) = road_style(road.class, z, road.tunnel) else { continue };
        if !visible(&road.line, place, margin) {
            continue;
        }
        let order = (road.class as i32) * 2 + i32::from(!road.tunnel);
        push(&mut groups, order, style, &road.line);
    }
    for boundary in &data.boundaries {
        let Some(style) = boundary_style(boundary.admin_level, z, boundary.disputed) else { continue };
        if !visible(&boundary.line, place, margin) {
            continue;
        }
        push(&mut groups, 1000 - boundary.admin_level as i32, style, &boundary.line);
    }
    let imported = import_draw::touches(imports, tile);
    if groups.is_empty() && !imported {
        return None;
    }
    groups.sort_by_key(|g| g.0);

    let mut pixmap = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE)?;
    // Casings first, so that where roads join their fills connect.
    for (_, style, lines) in &groups {
        if style.casing {
            let width = (style.width + 0.8) * TEXELS_PER_PIXEL;
            stroke(&mut pixmap, lines, place, width, [20, 20, 30, 170], None);
        }
    }
    for (_, style, lines) in &groups {
        stroke(&mut pixmap, lines, place, style.width * TEXELS_PER_PIXEL, style.color, style.dash);
    }
    // What was imported lies over the roads.
    if imported {
        import_draw::draw(&mut pixmap, imports, tile);
    }
    Some(pixmap.take())
}

fn push<'a>(groups: &mut Vec<(i32, LineStyle, Vec<&'a Line>)>, order: i32, style: LineStyle, line: &'a Line) {
    match groups.iter_mut().find(|g| g.0 == order && g.1 == style) {
        Some(group) => group.2.push(line),
        None => groups.push((order, style, vec![line])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::{Boundary, Road};

    fn road(class: RoadClass, line: Line) -> Road {
        Road { class, tunnel: false, line }
    }

    fn alpha_at(rgba: &[u8], x: u32, y: u32) -> u8 {
        rgba[((y * OVERLAY_SIZE + x) * 4 + 3) as usize]
    }

    #[test]
    fn minor_roads_appear_only_when_close() {
        assert!(road_style(RoadClass::Minor, 11, false).is_none());
        assert!(road_style(RoadClass::Minor, 12, false).is_some());
        assert!(road_style(RoadClass::Motorway, 5, false).is_some());
        assert!(road_style(RoadClass::Motorway, 4, false).is_none());
    }

    #[test]
    fn major_roads_are_wider_and_widen_with_zoom() {
        let w = |c, z| road_style(c, z, false).unwrap().width;
        assert!(w(RoadClass::Motorway, 12) > w(RoadClass::Primary, 12));
        assert!(w(RoadClass::Primary, 12) > w(RoadClass::Minor, 12));
        assert!(w(RoadClass::Motorway, 14) > w(RoadClass::Motorway, 8));
    }

    #[test]
    fn tunnels_are_dashed_and_quieter() {
        let open = road_style(RoadClass::Primary, 12, false).unwrap();
        let tunnel = road_style(RoadClass::Primary, 12, true).unwrap();
        assert!(open.dash.is_none() && tunnel.dash.is_some());
        assert!(tunnel.color[3] < open.color[3]);
        assert!(!tunnel.casing);
    }

    #[test]
    fn empty_tiles_produce_no_image() {
        let key = TileKey::new(12, 100, 100);
        assert!(render(&VectorTile::default(), key, key, &[]).is_none());
    }

    #[test]
    fn road_is_drawn_where_it_runs() {
        let key = TileKey::new(12, 100, 100);
        let data = VectorTile {
            roads: vec![road(RoadClass::Motorway, vec![[0.0, 0.5], [1.0, 0.5]])],
            ..Default::default()
        };
        let image = render(&data, key, key, &[]).unwrap();
        assert_eq!(image.len(), (OVERLAY_SIZE * OVERLAY_SIZE * 4) as usize);
        assert!(alpha_at(&image, 256, 256) > 200);
        assert_eq!(alpha_at(&image, 256, 50), 0);
    }

    #[test]
    fn child_tile_shows_its_part_of_the_source_magnified() {
        let source = TileKey::new(14, 1000, 1000);
        // The child in the bottom-right quarter of the source.
        let child = TileKey::new(15, 2001, 2001);
        let data = VectorTile {
            roads: vec![road(RoadClass::Motorway, vec![[0.5, 0.75], [1.0, 0.75]])],
            ..Default::default()
        };
        let image = render(&data, child, source, &[]).unwrap();
        assert!(alpha_at(&image, 256, 256) > 200, "runs through the child's middle");
        assert_eq!(alpha_at(&image, 256, 20), 0);
        // The other child sees none of it.
        assert!(render(&data, TileKey::new(15, 2000, 2000), source, &[]).is_none());
    }

    #[test]
    fn country_boundaries_show_from_far_away() {
        let key = TileKey::new(3, 4, 2);
        let data = VectorTile {
            boundaries: vec![
                Boundary { admin_level: 2, disputed: false, line: vec![[0.0, 0.5], [1.0, 0.5]] },
                Boundary { admin_level: 8, disputed: false, line: vec![[0.5, 0.0], [0.5, 1.0]] },
            ],
            ..Default::default()
        };
        let image = render(&data, key, key, &[]).unwrap();
        assert!(alpha_at(&image, 100, 256) > 100);
        assert_eq!(alpha_at(&image, 256, 100), 0, "local boundaries stay hidden");
    }

    #[test]
    fn output_is_premultiplied() {
        let key = TileKey::new(12, 100, 100);
        let data = VectorTile {
            roads: vec![road(RoadClass::Minor, vec![[0.0, 0.5], [1.0, 0.5]])],
            ..Default::default()
        };
        let image = render(&data, key, key, &[]).unwrap();
        assert!(image.chunks(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }
}
