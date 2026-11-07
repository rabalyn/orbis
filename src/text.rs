//! Rasterises short text with a built-in bitmap font.

use font8x8::legacy::{BASIC_LEGACY, LATIN_LEGACY};

const GLYPH: usize = 8;
const PAD: usize = 4;
const LINE_GAP: usize = 2;

fn glyph(c: char) -> [u8; 8] {
    let code = c as usize;
    match code {
        0..=127 => BASIC_LEGACY[code],
        0xA0..=0xFF => LATIN_LEGACY[code - 0xA0],
        _ => BASIC_LEGACY[b'?' as usize],
    }
}

/// Breaks text at spaces so no line exceeds `max_cols` characters.
pub fn wrap(text: &str, max_cols: usize) -> Vec<String> {
    let max_cols = max_cols.max(1);
    let mut lines = Vec::new();
    for source in text.lines() {
        let mut line = String::new();
        let mut len = 0;
        for word in source.split(' ') {
            let n = word.chars().count();
            if len > 0 && len + 1 + n > max_cols {
                lines.push(std::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += n;
        }
        lines.push(line);
    }
    lines
}

/// White text on a translucent dark panel, as straight-alpha RGBA8.
pub fn rasterize(lines: &[String], scale: usize) -> (u32, u32, Vec<u8>) {
    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let step = (GLYPH + LINE_GAP) * scale;
    let w = cols * GLYPH * scale + 2 * PAD * scale;
    let h = lines.len() * step + 2 * PAD * scale - LINE_GAP * scale;
    let mut px = [0u8, 0, 0, 170].repeat(w * h);
    for (row, line) in lines.iter().enumerate() {
        for (col, c) in line.chars().enumerate() {
            let bits = glyph(c);
            for (gy, byte) in bits.iter().enumerate() {
                for gx in 0..GLYPH {
                    if byte >> gx & 1 == 0 {
                        continue;
                    }
                    let x0 = PAD * scale + col * GLYPH * scale + gx * scale;
                    let y0 = PAD * scale + row * step + gy * scale;
                    for y in y0..y0 + scale {
                        for x in x0..x0 + scale {
                            px[(y * w + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
    (w as u32, h as u32, px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_keeps_all_words_within_width() {
        let lines = wrap("one two three four\nfive", 9);
        assert_eq!(lines, ["one two", "three", "four", "five"]);
    }

    #[test]
    fn long_word_stays_whole() {
        assert_eq!(wrap("https://example.org x", 5), ["https://example.org", "x"]);
    }

    #[test]
    fn panel_size_follows_the_text() {
        let (w, h, px) = rasterize(&["ab".to_string()], 2);
        assert_eq!((w, h), (2 * 16 + 16, 16 + 16));
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(px.chunks(4).any(|p| p == [255, 255, 255, 255]));
    }

    #[test]
    fn copyright_sign_has_a_glyph() {
        assert_ne!(glyph('©'), [0; 8]);
    }
}
