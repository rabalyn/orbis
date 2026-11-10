//! Turns building footprints into a 3D mesh standing on the terrain.

use crate::geo::{geodetic_to_ecef, TileKey};
use crate::vector::{Building, Zone};
use bytemuck::{Pod, Zeroable};
use earcut::Earcut;
use glam::DVec3;

const DEFAULT_COLOUR: [u8; 3] = [222, 216, 205];
/// Colours in the data are often pure and loud, so they are mixed towards the default.
const COLOUR_STRENGTH: f32 = 0.6;
/// Walls reach this far below the lowest corner, to meet the ground on slopes.
const FOUNDATION: f32 = 3.0;
/// A safeguard against a pathological tile.
const MAX_VERTICES: usize = 1_500_000;
/// Walls are a little darker than roofs before sunlight is applied; the direction of
/// the light comes from the shader.
const WALL_SHADE: f32 = 0.85;
/// Darkening where walls meet the ground, which helps buildings look grounded.
const WALL_BASE_SHADE: f32 = 0.72;
/// With aerial roofs, walls take this much of the roof's average colour, the rest being the
/// light plaster of `DEFAULT_COLOUR`. Roofs seen from above are mostly dark grey (tar, gravel,
/// solar panels); walls the roof's colour alone came out as heavy dark blocks, while real
/// façades are mostly light. A red-tiled house still gets warm walls.
const WALL_FROM_ROOF: f32 = 0.4;

fn wall_from_roof(roof: f32, plaster: u8) -> u8 {
    (roof * WALL_FROM_ROOF + plaster as f32 * (1.0 - WALL_FROM_ROOF)).round() as u8
}
/// The wall colours of a tile are a texture this many buildings wide.
pub const WALL_COLUMNS: u32 = 256;
/// Samples taken from the aerial picture across one footprint, at most, in each direction.
const ROOF_SAMPLES: usize = 12;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct BuildingVertex {
    pub pos: [f32; 3],
    /// sRGB colour with shading applied. On buildings, full alpha marks a roof, and on walls
    /// alpha is the shading, so that walls coloured from the aerial picture get it too. The
    /// energy bars use it for their opacity.
    pub colour: [u8; 4],
    /// On a roof, its position in the tile, from 0 to 65535, where the aerial picture is
    /// sampled. On a wall, the index of its building in the tile, low half first, where the
    /// wall colour is looked up.
    pub roof: [u16; 2],
}

pub struct BuildingMesh {
    pub center: DVec3,
    pub vertices: Vec<BuildingVertex>,
    pub indices: Vec<u32>,
}

/// The colours of zoned buildings, as a city builder shows its zones: green for living, blue
/// for business, yellow for industry. Each from the light shade of low buildings to the dark one
/// of the tallest.
const ZONE_COLOURS: [(Zone, [u8; 3], [u8; 3]); 3] = [
    (Zone::Residential, [178, 228, 150], [36, 118, 50]),
    (Zone::Commercial, [164, 202, 246], [30, 76, 172]),
    (Zone::Industrial, [250, 232, 140], [196, 146, 16]),
];
/// Heights in metres that get the lightest and the darkest shade: a house of two floors, and a
/// tower of about thirteen.
const LOW_RISE: f32 = 5.0;
const HIGH_RISE: f32 = 40.0;

/// How densely a building is used, from 0 to 1, by its height: the data has no floor area or
/// residents, and height follows them closely enough. On a log scale, as the step from a house to
/// a block of flats is as telling as that from a block to a tower.
fn density(height: f32) -> f32 {
    ((height.max(1.0).ln() - LOW_RISE.ln()) / (HIGH_RISE.ln() - LOW_RISE.ln())).clamp(0.0, 1.0)
}

fn base_colour(building: &Building) -> [f32; 3] {
    if let Some((_, light, dark)) = ZONE_COLOURS.iter().find(|z| Some(z.0) == building.zone) {
        let t = density(building.height);
        return [0, 1, 2].map(|i| light[i] as f32 + (dark[i] as f32 - light[i] as f32) * t);
    }
    let d = DEFAULT_COLOUR.map(|c| c as f32);
    match building.colour {
        Some(c) => [0, 1, 2].map(|i| d[i] + (c[i] as f32 - d[i]) * COLOUR_STRENGTH),
        None => d,
    }
}

fn shaded(colour: [f32; 3], shade: f32) -> [u8; 4] {
    let c = colour.map(|v| (v * shade).clamp(0.0, 255.0) as u8);
    [c[0], c[1], c[2], 255]
}

/// A wall's colour: shaded, with the shading kept in alpha.
fn wall_shaded(colour: [f32; 3], shade: f32) -> [u8; 4] {
    let mut c = shaded(colour, shade);
    c[3] = (shade * 255.0).round() as u8;
    c
}

/// Where a roof corner lies in the tile's aerial picture. Footprints cut at the tile's edge
/// end on it, so clamping only catches rounding.
pub fn roof_uv(p: [f32; 2]) -> [u16; 2] {
    p.map(|c| (c.clamp(0.0, 1.0) * 65535.0).round() as u16)
}

/// The index of a building, as its walls carry it.
fn wall_index(i: usize) -> [u16; 2] {
    [(i & 0xffff) as u16, (i >> 16) as u16]
}

/// Whether a tile position lies inside a footprint and outside its courtyards.
fn inside(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
    let mut odd = false;
    for ring in rings {
        for pair in ring.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                odd = !odd;
            }
        }
    }
    odd
}

/// The wall colour of each building, in the order of `buildings`: the average of its roof in the
/// tile's aerial picture, darkened, as sRGB. Alpha is 0 where the picture has nothing under the
/// roof, and the walls then keep their zone colour.
pub fn wall_colours(buildings: &[Building], picture: &image::RgbaImage) -> Vec<[u8; 4]> {
    let (w, h) = picture.dimensions();
    let pixel = |p: [f32; 2]| {
        let x = ((p[0].clamp(0.0, 1.0) * w as f32) as u32).min(w - 1);
        let y = ((p[1].clamp(0.0, 1.0) * h as f32) as u32).min(h - 1);
        picture.get_pixel(x, y).0
    };
    buildings
        .iter()
        .map(|b| {
            let Some(outer) = b.rings.first().filter(|r| r.len() >= 4) else { return [0; 4] };
            let (lo, hi) = outer.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| {
                ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])])
            });
            // Red, green and blue summed, and the count of samples with imagery.
            let mut sum = [0u32; 4];
            let add = |sum: &mut [u32; 4], c: [u8; 4]| {
                if c[3] >= 128 {
                    (0..3).for_each(|i| sum[i] += c[i] as u32);
                    sum[3] += 1;
                }
            };
            for i in 0..ROOF_SAMPLES {
                for j in 0..ROOF_SAMPLES {
                    let p = [
                        lo[0] + (hi[0] - lo[0]) * (i as f32 + 0.5) / ROOF_SAMPLES as f32,
                        lo[1] + (hi[1] - lo[1]) * (j as f32 + 0.5) / ROOF_SAMPLES as f32,
                    ];
                    if inside(&b.rings, p) {
                        add(&mut sum, pixel(p));
                    }
                }
            }
            if sum[3] == 0 {
                // Too thin for the samples to land in it.
                add(&mut sum, pixel([(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]));
            }
            if sum[3] == 0 {
                return [0; 4];
            }
            let c = [0, 1, 2].map(|i| wall_from_roof(sum[i] as f32 / sum[3] as f32, DEFAULT_COLOUR[i]));
            [c[0], c[1], c[2], 255]
        })
        .collect()
}

/// Whether a wall lies along the cut where a footprint was clipped at the tile's edge. The
/// neighbouring tile continues the building there, so no wall should stand on that line.
fn on_cut(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] <= 0.0 && b[0] <= 0.0)
        || (a[0] >= 1.0 && b[0] >= 1.0)
        || (a[1] <= 0.0 && b[1] <= 0.0)
        || (a[1] >= 1.0 && b[1] >= 1.0)
}

/// Builds one tile's mesh. `ground` gives the terrain height at a tile position. Returns
/// `None` when there is nothing to draw.
/// The smallest footprint drawn, in tile units squared: far below a garden shed even at the
/// finest building zoom, so it only drops rings that enclose nothing.
const MIN_FOOTPRINT_AREA: f64 = 1e-12;

/// The area a ring of corners encloses, by the shoelace formula, whichever way it winds.
fn footprint_area(corners: &[[f32; 2]]) -> f64 {
    let twice: f64 = corners
        .iter()
        .zip(corners.iter().cycle().skip(1))
        .map(|(a, b)| a[0] as f64 * b[1] as f64 - b[0] as f64 * a[1] as f64)
        .sum();
    twice.abs() / 2.0
}

pub fn build_mesh(
    key: TileKey,
    buildings: &[Building],
    ground: &dyn Fn(f64, f64) -> f32,
) -> Option<BuildingMesh> {
    let (clat, clon) = key.lat_lon(0.5, 0.5);
    let center = geodetic_to_ecef(clat, clon, 0.0);
    let mut vertices: Vec<BuildingVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut earcut: Earcut<f64> = Earcut::new();
    let mut triangles: Vec<u32> = Vec::new();
    let mut flat: Vec<[f64; 2]> = Vec::new();
    let mut holes: Vec<u32> = Vec::new();

    for (index, building) in buildings.iter().enumerate() {
        if vertices.len() >= MAX_VERTICES {
            break;
        }
        let Some(outer) = building.rings.first() else { continue };
        // Rings are closed, so the repeated last point does not count.
        let corners = &outer[..outer.len().saturating_sub(1)];
        if corners.len() < 3 || footprint_area(corners) < MIN_FOOTPRINT_AREA {
            // Repeated or collinear points enclose nothing: no roof, and walls of no width.
            continue;
        }
        // The roof height comes from the middle, so that buildings on slopes stay level.
        let count = corners.len() as f64;
        let (sx, sy) = corners.iter().fold((0.0, 0.0), |s, p| (s.0 + p[0] as f64, s.1 + p[1] as f64));
        let middle = ground((sx / count).clamp(0.0, 1.0), (sy / count).clamp(0.0, 1.0));
        let step = (corners.len() / 8).max(1);
        let lowest = corners
            .iter()
            .step_by(step)
            .map(|p| ground((p[0] as f64).clamp(0.0, 1.0), (p[1] as f64).clamp(0.0, 1.0)))
            .fold(middle, f32::min);
        let top = middle + building.height;
        let bottom = if building.min_height > 0.0 {
            middle + building.min_height
        } else {
            lowest - FOUNDATION
        };

        let colour = base_colour(building);
        let position = |p: [f32; 2], height: f32| -> [f32; 3] {
            let (lat, lon) = key.lat_lon(p[0] as f64, p[1] as f64);
            (geodetic_to_ecef(lat, lon, height as f64) - center).as_vec3().to_array()
        };

        // The roof.
        flat.clear();
        holes.clear();
        let roof_start = vertices.len() as u32;
        let roof_colour = shaded(colour, 1.0);
        for (i, ring) in building.rings.iter().enumerate() {
            if ring.len() < 4 {
                continue;
            }
            if i > 0 {
                holes.push(flat.len() as u32);
            }
            for p in &ring[..ring.len() - 1] {
                flat.push([p[0] as f64, p[1] as f64]);
                vertices.push(BuildingVertex { pos: position(*p, top), colour: roof_colour, roof: roof_uv(*p) });
            }
        }
        earcut.earcut(flat.iter().copied(), &holes, &mut triangles);
        for t in triangles.chunks_exact(3) {
            // In tile coordinates y runs south, so a positive area here means the triangle
            // winds counter-clockwise seen from above, which makes it face upwards.
            let (a, b, c) = (flat[t[0] as usize], flat[t[1] as usize], flat[t[2] as usize]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
            let [i, j, k] = [t[0], t[1], t[2]].map(|n| roof_start + n);
            if area < 0.0 {
                indices.extend([i, j, k]);
            } else {
                indices.extend([i, k, j]);
            }
        }

        // The walls, outer ring and holes alike.
        for ring in &building.rings {
            for pair in ring.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if on_cut(a, b) {
                    continue;
                }
                let (up, down) = (wall_shaded(colour, WALL_SHADE), wall_shaded(colour, WALL_SHADE * WALL_BASE_SHADE));
                let start = vertices.len() as u32;
                let roof = wall_index(index);
                vertices.push(BuildingVertex { pos: position(a, top), colour: up, roof });
                vertices.push(BuildingVertex { pos: position(b, top), colour: up, roof });
                vertices.push(BuildingVertex { pos: position(a, bottom), colour: down, roof });
                vertices.push(BuildingVertex { pos: position(b, bottom), colour: down, roof });
                // Wound to face outwards, so that a camera inside a building can see out.
                indices.extend([start, start + 1, start + 2, start + 1, start + 3, start + 2]);
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    Some(BuildingMesh { center, vertices, indices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::ecef_to_geodetic;
    use glam::Vec3;

    const KEY: TileKey = TileKey::new(14, 8712, 5750);
    const FLAT: &dyn Fn(f64, f64) -> f32 = &|_, _| 600.0;

    fn square(x: f32, y: f32, size: f32) -> Vec<[f32; 2]> {
        vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size], [x, y]]
    }

    fn building(rings: Vec<Vec<[f32; 2]>>) -> Building {
        Building { rings, height: 20.0, min_height: 0.0, colour: None, zone: None }
    }

    fn heights(mesh: &BuildingMesh) -> Vec<f64> {
        mesh.vertices
            .iter()
            .map(|v| ecef_to_geodetic(mesh.center + Vec3::from_array(v.pos).as_dvec3()).2)
            .collect()
    }

    #[test]
    fn a_box_has_a_roof_and_four_walls() {
        let mesh = build_mesh(KEY, &[building(vec![square(0.4, 0.4, 0.1)])], FLAT).unwrap();
        assert_eq!(mesh.vertices.len(), 4 + 4 * 4);
        assert_eq!(mesh.indices.len(), 2 * 3 + 4 * 6);
        assert!(mesh.indices.iter().all(|i| (*i as usize) < mesh.vertices.len()));
        let h = heights(&mesh);
        let (lo, hi) = h.iter().fold((f64::MAX, f64::MIN), |r, v| (r.0.min(*v), r.1.max(*v)));
        assert!((hi - 620.0).abs() < 0.1, "roof at {hi}");
        assert!((lo - 597.0).abs() < 0.1, "base reaches below ground, at {lo}");
    }

    /// Each triangle's front-face normal, and the triangle's middle, in world coordinates.
    fn faces(mesh: &BuildingMesh) -> Vec<(DVec3, DVec3)> {
        let at = |i: u32| mesh.center + Vec3::from_array(mesh.vertices[i as usize].pos).as_dvec3();
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (at(t[0]), at(t[1]), at(t[2]));
                ((b - a).cross(c - a).normalize(), (a + b + c) / 3.0)
            })
            .collect()
    }

    #[test]
    fn every_face_points_out_of_the_building() {
        // A courtyard, whose walls must face into the open space.
        let hole = vec![[0.44, 0.44], [0.44, 0.46], [0.46, 0.46], [0.46, 0.44], [0.44, 0.44]];
        let mesh = build_mesh(KEY, &[building(vec![square(0.4, 0.4, 0.1), hole])], FLAT).unwrap();
        let (lat, lon) = KEY.lat_lon(0.45, 0.45);
        let middle = geodetic_to_ecef(lat, lon, 610.0);
        let up = middle.normalize();
        let hole_half = {
            let (la, lo) = KEY.lat_lon(0.46, 0.45);
            (geodetic_to_ecef(la, lo, 610.0) - middle).length()
        };
        let (mut roofs, mut outer, mut inner) = (0, 0, 0);
        for (normal, at) in faces(&mesh) {
            if normal.dot(up) > 0.9 {
                roofs += 1;
                continue;
            }
            assert!(normal.dot(up).abs() < 0.1, "walls stand upright");
            // Horizontal offset from the middle of the building.
            let mut off = at - middle;
            off -= up * off.dot(up);
            if off.length() < hole_half * 1.5 {
                assert!(normal.dot(off) < 0.0, "a courtyard wall faces into the courtyard");
                inner += 1;
            } else {
                assert!(normal.dot(off) > 0.0, "an outer wall faces away from the building");
                outer += 1;
            }
        }
        assert_eq!((roofs, outer, inner), (8, 8, 8));
    }

    #[test]
    fn raised_parts_float_at_their_height() {
        let b = Building { min_height: 12.0, ..building(vec![square(0.4, 0.4, 0.1)]) };
        let mesh = build_mesh(KEY, &[b], FLAT).unwrap();
        let lowest = heights(&mesh).into_iter().fold(f64::MAX, f64::min);
        assert!((lowest - 612.0).abs() < 0.1, "{lowest}");
    }

    #[test]
    fn walls_reach_the_low_side_of_a_slope_and_the_roof_stays_level() {
        let slope: &dyn Fn(f64, f64) -> f32 = &|x, _| (x * 1000.0) as f32;
        let mesh = build_mesh(KEY, &[building(vec![square(0.4, 0.4, 0.1)])], slope).unwrap();
        let h = heights(&mesh);
        // Middle at 450 m, lowest corner at 400 m.
        let roof = &h[..4];
        assert!(roof.iter().all(|v| (*v - 470.0).abs() < 0.2), "{roof:?}");
        let lowest = h.iter().copied().fold(f64::MAX, f64::min);
        assert!((lowest - 397.0).abs() < 0.2, "{lowest}");
    }

    #[test]
    fn a_courtyard_stays_open() {
        let hole = vec![[0.44, 0.44], [0.44, 0.46], [0.46, 0.46], [0.46, 0.44], [0.44, 0.44]];
        let mesh = build_mesh(KEY, &[building(vec![square(0.4, 0.4, 0.1), hole])], FLAT).unwrap();
        // 8 roof corners, 8 walls with 4 corners each.
        assert_eq!(mesh.vertices.len(), 8 + 8 * 4);
        // A square with a square hole takes 8 triangles.
        assert_eq!(mesh.indices.len(), 8 * 3 + 8 * 6);
    }

    #[test]
    fn no_wall_stands_where_a_footprint_was_cut_at_the_tile_edge() {
        let cut = vec![[0.9, 0.4], [1.02, 0.4], [1.02, 0.5], [0.9, 0.5], [0.9, 0.4]];
        let mesh = build_mesh(KEY, &[building(vec![cut])], FLAT).unwrap();
        assert_eq!(mesh.vertices.len(), 4 + 3 * 4, "the east side is not walled");
    }

    #[test]
    fn colours_are_softened_and_missing_ones_defaulted() {
        let plain = building(vec![square(0.4, 0.4, 0.1)]);
        assert_eq!(base_colour(&plain), DEFAULT_COLOUR.map(|c| c as f32));
        let red = Building { colour: Some([255, 0, 0]), ..plain };
        let c = base_colour(&red);
        assert!(c[0] > c[1] && c[1] > 0.0 && c[0] < 255.0, "{c:?}");
    }

    #[test]
    fn zoned_buildings_take_their_zones_colour_darker_the_taller() {
        let at = |zone, height| base_colour(&Building { zone: Some(zone), height, colour: Some([255, 0, 0]), ..building(vec![square(0.4, 0.4, 0.1)]) });
        let (house, tower) = (at(Zone::Residential, 6.0), at(Zone::Residential, 80.0));
        assert!(house[1] > house[0] && house[1] > house[2], "green, not the data's red: {house:?}");
        assert!(tower.iter().sum::<f32>() < house.iter().sum::<f32>() * 0.5, "{house:?} {tower:?}");
        let shop = at(Zone::Commercial, 10.0);
        assert!(shop[2] > shop[0] && shop[2] > shop[1], "blue: {shop:?}");
        let works = at(Zone::Industrial, 10.0);
        assert!(works[0] > works[2] && works[1] > works[2] * 1.3, "yellow: {works:?}");
        assert_eq!(density(3.0), 0.0);
        assert_eq!(density(500.0), 1.0);
        assert!((density(LOW_RISE * (HIGH_RISE / LOW_RISE).sqrt()) - 0.5).abs() < 1e-4);
    }

    fn roof_vertices(mesh: &BuildingMesh) -> Vec<&BuildingVertex> {
        mesh.vertices.iter().filter(|v| v.colour[3] == 255).collect()
    }

    #[test]
    fn roofs_carry_their_place_in_the_tile() {
        let mesh = build_mesh(TileKey::new(14, 8000, 5000), &[building(vec![square(0.25, 0.5, 0.1)])], &|_, _| 0.0).unwrap();
        let roof = roof_vertices(&mesh);
        assert_eq!(roof.len(), 4);
        let us: Vec<u16> = roof.iter().map(|v| v.roof[0]).collect();
        let vs: Vec<u16> = roof.iter().map(|v| v.roof[1]).collect();
        let at = |f: f32| (f * 65535.0).round() as u16;
        assert_eq!((*us.iter().min().unwrap(), *us.iter().max().unwrap()), (at(0.25), at(0.35)));
        assert_eq!((*vs.iter().min().unwrap(), *vs.iter().max().unwrap()), (at(0.5), at(0.6)));
        // A footprint cut at the tile's corner reaches the very edge of the picture and stays in it.
        let cut = building(vec![vec![[-0.0001, -0.0001], [0.1, -0.0001], [0.1, 0.1], [-0.0001, 0.1], [-0.0001, -0.0001]]]);
        let mesh = build_mesh(TileKey::new(14, 8000, 5000), &[cut], &|_, _| 0.0).unwrap();
        let corners: Vec<[u16; 2]> = roof_vertices(&mesh).iter().map(|v| v.roof).collect();
        assert!(corners.contains(&[0, 0]), "{corners:?}");
        assert_eq!(roof_uv([1.2, 0.5]), [65535, at(0.5)]);
    }

    #[test]
    fn walls_carry_their_building_and_shading() {
        let houses: Vec<Building> = (0..3).map(|i| building(vec![square(0.1 + 0.2 * i as f32, 0.4, 0.1)])).collect();
        let mesh = build_mesh(TileKey::new(14, 8000, 5000), &houses, &|_, _| 0.0).unwrap();
        let walls: Vec<&BuildingVertex> = mesh.vertices.iter().filter(|v| v.colour[3] < 255).collect();
        assert_eq!(walls.len(), 3 * 4 * 4);
        for (n, w) in walls.iter().enumerate() {
            assert_eq!(w.roof, [(n / 16) as u16, 0], "the building's index");
        }
        let shades: Vec<u8> = walls.iter().map(|w| w.colour[3]).collect();
        assert!(shades.contains(&217) && shades.contains(&156), "top and base: {shades:?}");
        assert_eq!(wall_index(70_000), [4464, 1]);
    }

    #[test]
    fn walls_take_the_average_of_their_roof_lightened_towards_plaster() {
        // The left half red, the right half grey with a stripe of white; the far right has no imagery.
        let picture = image::RgbaImage::from_fn(100, 100, |x, y| {
            image::Rgba(match x {
                0..50 => [200, 0, 0, 255],
                50..80 if y < 50 => [100, 100, 100, 255],
                50..80 => [200, 200, 200, 255],
                _ => [0, 0, 0, 0],
            })
        });
        let red = building(vec![square(0.1, 0.1, 0.2)]);
        let grey = building(vec![square(0.55, 0.4, 0.2)]);
        let dark = building(vec![square(0.85, 0.1, 0.1)]);
        let walls = wall_colours(&[red, grey, dark, building(vec![])], &picture);
        let p = DEFAULT_COLOUR;
        let wall = |c: [f32; 3]| {
            let w = [0, 1, 2].map(|i| wall_from_roof(c[i], p[i]));
            [w[0], w[1], w[2], 255]
        };
        assert_eq!(walls[0], wall([200.0, 0.0, 0.0]));
        assert!(walls[0][0] > walls[0][1] + 50, "a red roof gives warm walls");
        assert_eq!(walls[1], wall([150.0; 3]), "half grey, half white");
        assert!(walls[1][0] > 150, "a grey roof gives walls lighter than the roof");
        assert_eq!(walls[2][3], 0, "no imagery: the walls keep their zone colour");
        assert_eq!(walls[3], [0; 4]);
    }

    #[test]
    fn nothing_to_draw_gives_no_mesh() {
        assert!(build_mesh(KEY, &[], FLAT).is_none());
        let degenerate = building(vec![vec![[0.1, 0.1], [0.2, 0.2]]]);
        assert!(build_mesh(KEY, &[degenerate], FLAT).is_none());
    }

    #[test]
    fn odd_shapes_do_not_panic() {
        // Self-crossing, repeated and collinear points, as bad data may contain.
        let bowtie = vec![[0.1, 0.1], [0.2, 0.2], [0.2, 0.1], [0.1, 0.2], [0.1, 0.1]];
        let repeated = vec![[0.3, 0.3], [0.3, 0.3], [0.3, 0.3], [0.3, 0.3], [0.3, 0.3]];
        let line = vec![[0.5, 0.5], [0.6, 0.5], [0.7, 0.5], [0.5, 0.5]];
        let huge = vec![[-1e9, -1e9], [1e9, -1e9], [1e9, 1e9], [-1e9, 1e9], [-1e9, -1e9]];
        // Rings that enclose no area (the bowtie's two halves cancel) give nothing to draw.
        for (name, ring) in [("bowtie", bowtie), ("repeated", repeated), ("line", line)] {
            assert!(build_mesh(KEY, &[building(vec![ring])], FLAT).is_none(), "{name}");
        }
        // A footprint far beyond the tile still gives a sound mesh: a roof, and no walls at the cut edges.
        let mesh = build_mesh(KEY, &[building(vec![huge])], FLAT).expect("a roof");
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), (4, 6));
        assert!(mesh.indices.iter().all(|i| (*i as usize) < mesh.vertices.len()));
        assert!(mesh.vertices.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
        // A degenerate ring among good ones does not disturb them.
        let good = build_mesh(KEY, &[building(vec![square(0.4, 0.4, 0.1)])], FLAT).unwrap();
        let mixed = build_mesh(KEY, &[building(vec![vec![[0.3, 0.3]; 5]]), building(vec![square(0.4, 0.4, 0.1)])], FLAT).unwrap();
        assert_eq!((mixed.vertices.len(), mixed.indices.len()), (good.vertices.len(), good.indices.len()));
        assert!(footprint_area(&square(0.4, 0.4, 0.1)[..4]) > 0.0099 && footprint_area(&square(0.4, 0.4, 0.1)[..4]) < 0.0101);
    }

    #[test]
    fn vertex_count_has_a_ceiling() {
        let many: Vec<Building> = (0..200_000)
            .map(|i| {
                let f = i as f32 / 200_000.0 * 0.9;
                building(vec![square(f, f, 0.001)])
            })
            .collect();
        let mesh = build_mesh(KEY, &many, FLAT).unwrap();
        assert!(mesh.vertices.len() < MAX_VERTICES + 100);
    }
}
