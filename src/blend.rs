//! Lays one picture of a tile over another, for map services shown over the base imagery.

use image::RgbaImage;

/// The position of a tile within an image, as (left, top, size) in 0..1, as `rect_in` gives.
pub type Rect = (f64, f64, f64);

/// Colour at a position within the tile, by bilinear sampling. The edges stay where they are.
fn sample(img: &RgbaImage, rect: Rect, fx: f64, fy: f64) -> [f32; 4] {
    let (w, h) = img.dimensions();
    let x = ((rect.0 + rect.2 * fx) * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
    let y = ((rect.1 + rect.2 * fy) * h as f64 - 0.5).clamp(0.0, (h - 1) as f64);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let at = |px: u32, py: u32| img.get_pixel(px, py).0;
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mut out = [0.0; 4];
    for (i, o) in out.iter_mut().enumerate() {
        let top = a[i] as f32 + (b[i] as f32 - a[i] as f32) * tx;
        let bottom = c[i] as f32 + (d[i] as f32 - c[i] as f32) * tx;
        *o = top + (bottom - top) * ty;
    }
    out
}

/// The tile's area of `over` laid on that of `base`, `opacity` times the overlay's own
/// transparency. The result covers exactly the tile and has the finer of the two resolutions,
/// at most 512 pixels, so that a detailed overlay is not blurred by a coarse base.
pub fn composite(
    base: &RgbaImage,
    base_rect: Rect,
    over: &RgbaImage,
    over_rect: Rect,
    opacity: f32,
) -> RgbaImage {
    let finest = (base.width() as f64 * base_rect.2).max(over.width() as f64 * over_rect.2);
    let n: u32 = if finest > 256.0 { 512 } else { 256 };
    let opacity = opacity.clamp(0.0, 1.0);
    let mut out = RgbaImage::new(n, n);
    for py in 0..n {
        for px in 0..n {
            let (fx, fy) = ((px as f64 + 0.5) / n as f64, (py as f64 + 0.5) / n as f64);
            let b = sample(base, base_rect, fx, fy);
            let o = sample(over, over_rect, fx, fy);
            let a = o[3] / 255.0 * opacity;
            let mix = |i: usize| (b[i] * (1.0 - a) + o[i] * a).round().clamp(0.0, 255.0) as u8;
            out.put_pixel(px, py, image::Rgba([mix(0), mix(1), mix(2), 255]));
        }
    }
    out
}

/// Whether any part of a picture lets what is under it show through.
pub fn has_holes(img: &RgbaImage) -> bool {
    img.as_raw().chunks_exact(4).any(|p| p[3] < 255)
}

/// `base` laid under `top` where `top` is transparent, for a finer layer with gaps over a
/// coarser one. Unlike [`composite`] the result keeps what transparency both leave, so that a
/// third layer can fill it in turn.
pub fn under(top: &RgbaImage, top_rect: Rect, base: &RgbaImage, base_rect: Rect) -> RgbaImage {
    let finest = (base.width() as f64 * base_rect.2).max(top.width() as f64 * top_rect.2);
    let n: u32 = if finest > 256.0 { 512 } else { 256 };
    let mut out = RgbaImage::new(n, n);
    for py in 0..n {
        for px in 0..n {
            let (fx, fy) = ((px as f64 + 0.5) / n as f64, (py as f64 + 0.5) / n as f64);
            let t = sample(top, top_rect, fx, fy);
            let b = sample(base, base_rect, fx, fy);
            let (ta, ba) = (t[3] / 255.0, b[3] / 255.0);
            let a = ta + ba * (1.0 - ta);
            let mix = |i: usize| {
                if a <= 0.0 {
                    0
                } else {
                    ((t[i] * ta + b[i] * ba * (1.0 - ta)) / a)
                        .round()
                        .clamp(0.0, 255.0) as u8
                }
            };
            out.put_pixel(
                px,
                py,
                image::Rgba([mix(0), mix(1), mix(2), (a * 255.0).round() as u8]),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_with_holes_shows_the_one_under_it_there() {
        let mut top = flat(256, [200, 200, 200, 255]);
        for y in 0..256 {
            for x in 128..256 {
                top.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
        let base = flat(256, [10, 20, 30, 255]);
        assert!(has_holes(&top) && !has_holes(&base));
        let out = under(&top, WHOLE, &base, WHOLE);
        assert_eq!(out.get_pixel(10, 10).0, [200, 200, 200, 255]);
        assert_eq!(out.get_pixel(250, 10).0, [10, 20, 30, 255]);
        assert!(!has_holes(&out));
        // Two layers with holes leave a hole where neither has a picture.
        let empty = flat(256, [0, 0, 0, 0]);
        assert!(has_holes(&under(&top, WHOLE, &empty, WHOLE)));
    }

    fn flat(n: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(n, n, image::Rgba(c))
    }

    const WHOLE: Rect = (0.0, 0.0, 1.0);

    #[test]
    fn zero_opacity_keeps_the_base_and_full_opacity_takes_the_overlay() {
        let (b, o) = (flat(256, [10, 20, 30, 255]), flat(256, [200, 100, 50, 255]));
        assert_eq!(
            composite(&b, WHOLE, &o, WHOLE, 0.0).get_pixel(5, 5).0,
            [10, 20, 30, 255]
        );
        assert_eq!(
            composite(&b, WHOLE, &o, WHOLE, 1.0).get_pixel(5, 5).0,
            [200, 100, 50, 255]
        );
        assert_eq!(
            composite(&b, WHOLE, &o, WHOLE, 0.5).get_pixel(5, 5).0,
            [105, 60, 40, 255]
        );
    }

    #[test]
    fn the_overlays_own_transparency_counts() {
        let (b, o) = (flat(256, [0, 0, 0, 255]), flat(256, [200, 200, 200, 0]));
        assert_eq!(
            composite(&b, WHOLE, &o, WHOLE, 1.0).get_pixel(9, 9).0,
            [0, 0, 0, 255]
        );
        let half = flat(256, [200, 200, 200, 128]);
        let v = composite(&b, WHOLE, &half, WHOLE, 1.0).get_pixel(9, 9).0[0];
        assert!((99..=101).contains(&v), "{v}");
    }

    #[test]
    fn the_part_of_a_coarser_image_that_the_tile_shows_is_used() {
        // The left half of the overlay is white, the right half black: the tile is its right half.
        let mut o = flat(256, [255, 255, 255, 255]);
        for y in 0..256 {
            for x in 128..256 {
                o.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let b = flat(256, [9, 9, 9, 255]);
        let out = composite(&b, WHOLE, &o, (0.5, 0.0, 0.5), 1.0);
        assert_eq!(out.get_pixel(128, 128).0[0], 0);
        let out = composite(&b, WHOLE, &o, (0.0, 0.0, 0.5), 1.0);
        assert_eq!(out.get_pixel(128, 128).0[0], 255);
    }

    #[test]
    fn detail_in_either_picture_raises_the_resolution() {
        let (small, big) = (flat(256, [1, 1, 1, 255]), flat(512, [1, 1, 1, 255]));
        assert_eq!(
            composite(&small, WHOLE, &small, WHOLE, 0.5).dimensions(),
            (256, 256)
        );
        assert_eq!(
            composite(&small, WHOLE, &big, WHOLE, 0.5).dimensions(),
            (512, 512)
        );
        assert_eq!(
            composite(&small, (0.0, 0.0, 0.25), &small, WHOLE, 0.5).dimensions(),
            (256, 256)
        );
    }
}
