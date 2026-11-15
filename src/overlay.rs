//! Draws roads and boundaries into a transparent image for one globe tile. The image is
//! laid over the tile's imagery, so lines follow the terrain without any 3D geometry.

use crate::geo::TileKey;
use crate::import::Imported;
use crate::import_draw;
use crate::layers::LineKinds;
use crate::overlay_gpu::{self, LineMesh, Parts};
use crate::vector::{Line, RoadClass, VectorTile};
use std::sync::Arc;
use tiny_skia::{
    Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform,
};

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
    let color = if tunnel {
        [color[0], color[1], color[2], 120]
    } else {
        color
    };
    Some(LineStyle {
        width,
        color,
        casing: casing && !tunnel,
        dash,
    })
}

/// How a boundary looks at a tile zoom level. `None` hides it.
pub fn boundary_style(admin_level: i64, z: u8, disputed: bool) -> Option<LineStyle> {
    let (width, color) = match admin_level {
        ..=2 => (1.1, [255, 255, 255, 215]),
        3..=4 if z >= 5 => (0.8, [235, 225, 255, 165]),
        _ => return None,
    };
    let dash = (disputed || admin_level > 2).then_some([4.0, 3.0]);
    Some(LineStyle {
        width,
        color,
        casing: false,
        dash,
    })
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
        let Some((x, y)) = points.next() else {
            continue;
        };
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
        line_cap: if dash.is_some() {
            LineCap::Butt
        } else {
            LineCap::Round
        },
        line_join: LineJoin::Round,
        dash: dash.and_then(|d| StrokeDash::new(vec![d[0] * width, d[1] * width], 0.0)),
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

/// A group of lines stroked as one: where to place it, its style and its lines.
type Group<'a> = (i32, LineStyle, Vec<&'a Line>);

/// The lines to draw on `tile` in drawing order, with the transform to overlay texels.
fn groups<'a>(
    data: &'a VectorTile,
    tile: TileKey,
    source: TileKey,
    kinds: LineKinds,
) -> ((f32, f32, f32), Vec<Group<'a>>) {
    let place = placement(tile, source);
    let margin = 32.0;
    let z = tile.z;

    let mut groups: Vec<Group> = Vec::new();
    for road in data.roads.iter().filter(|r| kinds.road(r.class)) {
        let Some(style) = road_style(road.class, z, road.tunnel) else {
            continue;
        };
        if !visible(&road.line, place, margin) {
            continue;
        }
        let order = (road.class as i32) * 2 + i32::from(!road.tunnel);
        push(&mut groups, order, style, &road.line);
    }
    for boundary in data.boundaries.iter().filter(|_| kinds.boundaries) {
        let Some(style) = boundary_style(boundary.admin_level, z, boundary.disputed) else {
            continue;
        };
        if !visible(&boundary.line, place, margin) {
            continue;
        }
        push(
            &mut groups,
            1000 - boundary.admin_level as i32,
            style,
            &boundary.line,
        );
    }
    groups.sort_by_key(|g| g.0);
    (place, groups)
}

/// The strokes that draw `groups`, in order: lines, width in texels, colour and dash.
/// Casings come first, so that where roads join their fills connect.
fn strokes<'a, 'b>(
    groups: &'b [Group<'a>],
) -> impl Iterator<Item = (&'b [&'a Line], f32, [u8; 4], Option<[f32; 2]>)> {
    let casings = groups
        .iter()
        .filter(|g| g.1.casing)
        .map(|(_, style, lines)| {
            let width = (style.width + 0.8) * TEXELS_PER_PIXEL;
            (lines.as_slice(), width, [20, 20, 30, 170], None)
        });
    let fills = groups.iter().map(|(_, style, lines)| {
        (
            lines.as_slice(),
            style.width * TEXELS_PER_PIXEL,
            style.color,
            style.dash,
        )
    });
    casings.chain(fills)
}

/// Draws the lines for `tile`, whose data comes from the `source` vector tile. Returns
/// premultiplied RGBA, or `None` when nothing was drawn.
pub fn render(
    data: &VectorTile,
    tile: TileKey,
    source: TileKey,
    imports: &[Arc<Imported>],
    kinds: LineKinds,
) -> Option<Vec<u8>> {
    let (place, groups) = groups(data, tile, source, kinds);
    let imported = import_draw::touches(imports, tile);
    if groups.is_empty() && !imported {
        return None;
    }
    let mut pixmap = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE)?;
    for (lines, width, color, dash) in strokes(&groups) {
        stroke(&mut pixmap, lines, place, width, color, dash);
    }
    // What was imported lies over the roads.
    if imported {
        import_draw::draw(&mut pixmap, imports, tile);
    }
    Some(pixmap.take())
}

/// A tile's overlay as the loader hands it on.
pub enum Picture {
    /// Drawn here, by the processor: premultiplied RGBA.
    Drawn(Vec<u8>),
    /// To be painted by the graphics card.
    Parts(Parts),
}

/// As `render`, but with `gpu` the lines are only cut into triangles for the graphics card to
/// paint; what was imported is still drawn here and laid over them there. A tile without
/// lines gains nothing from that and is drawn here.
pub fn render_for(
    data: &VectorTile,
    tile: TileKey,
    source: TileKey,
    imports: &[Arc<Imported>],
    kinds: LineKinds,
    gpu: bool,
) -> Option<Picture> {
    if !gpu {
        return render(data, tile, source, imports, kinds).map(Picture::Drawn);
    }
    let (place, groups) = groups(data, tile, source, kinds);
    if groups.is_empty() {
        return render(data, tile, source, imports, kinds).map(Picture::Drawn);
    }
    let mut lines = LineMesh::default();
    for (group, width, color, dash) in strokes(&groups) {
        overlay_gpu::stroke(&mut lines, group, place, width, color, dash);
    }
    let top = if import_draw::touches(imports, tile) {
        let mut pixmap = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE)?;
        import_draw::draw(&mut pixmap, imports, tile);
        Some(pixmap.take())
    } else {
        None
    };
    Some(Picture::Parts(Parts { lines, top }))
}

fn push<'a>(
    groups: &mut Vec<(i32, LineStyle, Vec<&'a Line>)>,
    order: i32,
    style: LineStyle,
    line: &'a Line,
) {
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
        Road {
            class,
            tunnel: false,
            line,
        }
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

    const ALL: LineKinds = LineKinds {
        streets: true,
        trails: true,
        trains: true,
        boundaries: true,
    };

    #[test]
    fn only_the_kinds_chosen_are_drawn() {
        let key = TileKey::new(15, 100, 100);
        let data = VectorTile {
            roads: vec![
                road(RoadClass::Primary, vec![[0.0, 0.2], [1.0, 0.2]]),
                road(RoadClass::Path, vec![[0.0, 0.5], [1.0, 0.5]]),
                road(RoadClass::Rail, vec![[0.0, 0.8], [1.0, 0.8]]),
            ],
            ..Default::default()
        };
        let none = LineKinds {
            streets: false,
            trails: false,
            trains: false,
            boundaries: false,
        };
        assert!(render(&data, key, key, &[], none).is_none());
        let only = |k: LineKinds| render(&data, key, key, &[], k).unwrap();
        // Trails and rails are dashed, so look along the row.
        let rows =
            |img: &[u8]| [102, 256, 410].map(|y| (0..512).any(|x| alpha_at(img, x, y) > 100));
        assert_eq!(
            rows(&only(LineKinds {
                streets: true,
                ..none
            })),
            [true, false, false]
        );
        assert_eq!(
            rows(&only(LineKinds {
                trails: true,
                ..none
            })),
            [false, true, false]
        );
        assert_eq!(
            rows(&only(LineKinds {
                trains: true,
                ..none
            })),
            [false, false, true]
        );
        assert_eq!(rows(&only(ALL)), [true, true, true]);
    }

    #[test]
    fn empty_tiles_produce_no_image() {
        let key = TileKey::new(12, 100, 100);
        assert!(render(&VectorTile::default(), key, key, &[], ALL).is_none());
    }

    #[test]
    fn road_is_drawn_where_it_runs() {
        let key = TileKey::new(12, 100, 100);
        let data = VectorTile {
            roads: vec![road(RoadClass::Motorway, vec![[0.0, 0.5], [1.0, 0.5]])],
            ..Default::default()
        };
        let image = render(&data, key, key, &[], ALL).unwrap();
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
        let image = render(&data, child, source, &[], ALL).unwrap();
        assert!(
            alpha_at(&image, 256, 256) > 200,
            "runs through the child's middle"
        );
        assert_eq!(alpha_at(&image, 256, 20), 0);
        // The other child sees none of it.
        assert!(render(&data, TileKey::new(15, 2000, 2000), source, &[], ALL).is_none());
    }

    #[test]
    fn country_boundaries_show_from_far_away() {
        let key = TileKey::new(3, 4, 2);
        let data = VectorTile {
            boundaries: vec![
                Boundary {
                    admin_level: 2,
                    disputed: false,
                    line: vec![[0.0, 0.5], [1.0, 0.5]],
                },
                Boundary {
                    admin_level: 8,
                    disputed: false,
                    line: vec![[0.5, 0.0], [0.5, 1.0]],
                },
            ],
            ..Default::default()
        };
        let image = render(&data, key, key, &[], ALL).unwrap();
        assert!(alpha_at(&image, 100, 256) > 100);
        assert_eq!(
            alpha_at(&image, 256, 100),
            0,
            "local boundaries stay hidden"
        );
    }

    #[test]
    fn output_is_premultiplied() {
        let key = TileKey::new(12, 100, 100);
        let data = VectorTile {
            roads: vec![road(RoadClass::Minor, vec![[0.0, 0.5], [1.0, 0.5]])],
            ..Default::default()
        };
        let image = render(&data, key, key, &[], ALL).unwrap();
        assert!(image
            .chunks(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }

    fn tile(roads: Vec<Road>) -> VectorTile {
        VectorTile {
            roads,
            ..Default::default()
        }
    }

    /// A road with sharp and gentle bends, in tile coordinates.
    fn winding() -> Line {
        vec![
            [0.1, 0.1],
            [0.5, 0.15],
            [0.55, 0.6],
            [0.2, 0.3],
            [0.25, 0.8],
            [0.9, 0.85],
            [0.92, 0.4],
        ]
    }

    #[test]
    fn without_the_graphics_card_the_picture_is_the_processors() {
        let key = TileKey::new(10, 5, 5);
        let data = tile(vec![road(RoadClass::Motorway, winding())]);
        let Some(Picture::Drawn(rgba)) = render_for(&data, key, key, &[], ALL, false) else {
            panic!("a drawn picture");
        };
        assert_eq!(Some(rgba), render(&data, key, key, &[], ALL));
        assert!(render_for(&tile(Vec::new()), key, key, &[], ALL, false).is_none());
        assert!(render_for(&tile(Vec::new()), key, key, &[], ALL, true).is_none());
    }

    #[test]
    fn for_the_graphics_card_lines_become_groups_in_drawing_order() {
        let key = TileKey::new(10, 5, 5);
        let data = tile(vec![
            road(RoadClass::Motorway, winding()),
            road(RoadClass::Minor, vec![[0.0, 0.5], [1.0, 0.5]]),
        ]);
        let Some(Picture::Parts(parts)) = render_for(&data, key, key, &[], ALL, true) else {
            panic!("parts for the graphics card");
        };
        assert!(parts.top.is_none());
        // z10 hides minor roads: the motorway's casing, then its fill.
        assert_eq!(parts.lines.groups.len(), 2);
        let colour = |g: usize| parts.lines.vertices[parts.lines.groups[g].start as usize].color;
        assert_eq!(colour(0)[3], 170);
        assert_eq!(colour(1)[3], 235);
    }

    #[test]
    fn imports_are_drawn_by_the_processor_and_laid_over_the_lines() {
        use crate::import::{Item, Pos, Shape};
        let key = TileKey::new(0, 0, 0);
        let pos = |lat: f64, lon: f64| Pos {
            lat: lat.to_radians(),
            lon: lon.to_radians(),
            alt: None,
        };
        let docs = vec![Arc::new(Imported {
            name: "t".into(),
            items: vec![Item::new("p", Shape::Point(pos(0.0, 0.0)))],
            truncated: false,
        })];
        let data = VectorTile {
            boundaries: vec![Boundary {
                admin_level: 2,
                disputed: false,
                line: vec![[0.0, 0.25], [1.0, 0.25]],
            }],
            ..Default::default()
        };
        let Some(Picture::Parts(parts)) = render_for(&data, key, key, &docs, ALL, true) else {
            panic!("parts for the graphics card");
        };
        let top = parts.top.expect("the imported point");
        assert!(alpha_at(&top, 256, 256) > 200, "the point is in the middle");
        assert_eq!(alpha_at(&top, 256, 128), 0, "the boundary is not in it");
        // With nothing but imports there is nothing for the card to do.
        let only = render_for(&tile(Vec::new()), key, key, &docs, ALL, true);
        assert!(matches!(only, Some(Picture::Drawn(_))));
    }

    /// The triangles made for the graphics card cover the texels the processor paints: checked
    /// without a card, by testing texel centres against the triangles.
    #[test]
    fn the_triangles_cover_what_the_processor_strokes() {
        let place = (OVERLAY_SIZE as f32, 0.0, 0.0);
        let (line, straight) = (winding(), vec![[0.05, 0.95], [0.95, 0.9]]);
        let lines = [&line, &straight];
        for (width, dash) in [(6.0, None), (2.4, None), (3.0, Some([3.0, 2.0]))] {
            let mut pixmap = Pixmap::new(OVERLAY_SIZE, OVERLAY_SIZE).unwrap();
            stroke(&mut pixmap, &lines, place, width, [255; 4], dash);
            let image = pixmap.take();
            let mut mesh = LineMesh::default();
            overlay_gpu::stroke(&mut mesh, &lines, place, width, [255; 4], dash);
            let (mut painted, mut differ) = (0, 0);
            for y in 0..OVERLAY_SIZE {
                for x in 0..OVERLAY_SIZE {
                    let alpha = alpha_at(&image, x, y);
                    let inside = overlay_gpu::tests::covered(
                        &mesh,
                        &mesh.groups[0],
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                    );
                    painted += usize::from(alpha >= 128);
                    // Texels the edge runs through may fall either way.
                    differ += usize::from((alpha > 230 && !inside) || (alpha < 25 && inside));
                }
            }
            assert!(painted > 2000, "{width} {dash:?}: {painted}");
            assert!(
                differ * 200 < painted,
                "{width} {dash:?}: {differ} of {painted}"
            );
        }
    }

    /// The same comparison on a real graphics card, where the machine has one: the painted
    /// image against the processor's, texel by texel.
    #[test]
    fn the_graphics_card_paints_what_the_processor_draws() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("no graphics adapter here: the painted overlay was not compared");
            return;
        };
        let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
        else {
            eprintln!("no graphics device here: the painted overlay was not compared");
            return;
        };
        let key = TileKey::new(10, 5, 5);
        let data = VectorTile {
            roads: vec![
                road(RoadClass::Motorway, winding()),
                road(RoadClass::Primary, vec![[0.0, 0.5], [1.0, 0.45]]),
                road(RoadClass::Rail, vec![[0.3, 0.0], [0.35, 1.0]]),
            ],
            boundaries: vec![Boundary {
                admin_level: 2,
                disputed: true,
                line: vec![[0.0, 0.7], [1.0, 0.72]],
            }],
            ..Default::default()
        };
        let drawn = render(&data, key, key, &[], ALL).unwrap();
        let Some(Picture::Parts(parts)) = render_for(&data, key, key, &[], ALL, true) else {
            panic!("parts for the graphics card");
        };
        let painter = overlay_gpu::Painter::new(&device);
        let (texture, commands) = painter.paint(&device, &queue, &parts);
        let row = OVERLAY_SIZE * 4;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * OVERLAY_SIZE) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(OVERLAY_SIZE),
                },
            },
            wgpu::Extent3d {
                width: OVERLAY_SIZE,
                height: OVERLAY_SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([commands, encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let painted = buffer.slice(..).get_mapped_range().unwrap().to_vec();

        let (mut far, mut sum, mut inked) = (0usize, 0u64, 0usize);
        for (a, b) in drawn.chunks_exact(4).zip(painted.chunks_exact(4)) {
            let worst = (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap();
            far += usize::from(worst > 96);
            sum += worst as u64;
            inked += usize::from(a[3] > 0);
        }
        assert!(inked > 5000, "{inked}");
        // Edges are smoothed differently, so texels on them differ a little; none of the
        // picture is missing or misplaced.
        assert!(far * 100 < inked, "{far} of {inked} texels are far off");
        assert!(
            sum / (inked as u64) < 12,
            "mean difference {}",
            sum / inked as u64
        );
    }
}
