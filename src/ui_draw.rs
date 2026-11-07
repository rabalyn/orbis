//! Paints a panel of the interface into an RGBA image, with the built-in bitmap font.

use crate::text::glyph;
use crate::ui::{Kind, Panel, Rect, Tone, Ui, Widget, WidgetId};

type Color = [u8; 4];

const BACKGROUND: Color = [8, 10, 16, 255];
const BORDER: Color = [255, 255, 255, 80];
const BUTTON: Color = [255, 255, 255, 45];
const BUTTON_HOVER: Color = [255, 255, 255, 100];
const BUTTON_PRESSED: Color = [255, 255, 255, 150];
const TRACK: Color = [255, 255, 255, 35];
const THUMB: Color = [255, 255, 255, 120];
const THUMB_HELD: Color = [255, 255, 255, 190];

fn tone(t: Tone) -> Color {
    match t {
        Tone::Normal => [255, 255, 255, 255],
        Tone::Heading => [255, 215, 120, 255],
        Tone::Key => [150, 210, 255, 255],
        Tone::Dim => [180, 180, 180, 255],
    }
}

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u8>,
    clip: [i64; 4],
}

impl Canvas {
    fn put(&mut self, x: i64, y: i64, c: Color) {
        let [x0, y0, x1, y1] = self.clip;
        if x < x0.max(0) || y < y0.max(0) || x >= x1.min(self.w as i64) || y >= y1.min(self.h as i64) {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        let dst = &mut self.px[i..i + 4];
        // Straight-alpha "over" on the panel's own pixels.
        let a = c[3] as u32;
        let da = dst[3] as u32;
        let out_a = a + da * (255 - a) / 255;
        if out_a == 0 {
            return;
        }
        for k in 0..3 {
            dst[k] = ((c[k] as u32 * a + dst[k] as u32 * da * (255 - a) / 255) / out_a) as u8;
        }
        dst[3] = out_a as u8;
    }

    fn fill(&mut self, r: Rect, c: Color) {
        for y in r.y.round() as i64..(r.y + r.h).round() as i64 {
            for x in r.x.round() as i64..(r.x + r.w).round() as i64 {
                self.put(x, y, c);
            }
        }
    }

    /// A ring, or a disc when `filled`, inside the square `r`.
    fn disc(&mut self, r: Rect, filled: bool, c: Color) {
        let (cx, cy, rad) = (r.x + r.w / 2.0, r.y + r.h / 2.0, r.w.min(r.h) / 2.0);
        for y in r.y.floor() as i64..(r.y + r.h).ceil() as i64 {
            for x in r.x.floor() as i64..(r.x + r.w).ceil() as i64 {
                let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
                if d <= rad && (filled || d > rad - 1.2) {
                    self.put(x, y, c);
                }
            }
        }
    }

    fn text(&mut self, x: f64, y: f64, text: &str, scale: usize, c: Color) {
        for (col, ch) in text.chars().enumerate() {
            for (gy, byte) in glyph(ch).iter().enumerate() {
                for gx in 0..8 {
                    if byte >> gx & 1 == 0 {
                        continue;
                    }
                    let x0 = x.round() as i64 + ((col * 8 + gx) * scale) as i64;
                    let y0 = y.round() as i64 + (gy * scale) as i64;
                    for dy in 0..scale as i64 {
                        for dx in 0..scale as i64 {
                            self.put(x0 + dx, y0 + dy, c);
                        }
                    }
                }
            }
        }
    }
}

/// The row of a checkbox or a choice: the mark in front, then the label.
struct Row<'a> {
    label: &'a str,
    scale: usize,
    enabled: bool,
    hover: bool,
    pressed: bool,
}

fn row(cv: &mut Canvas, r: Rect, w: Row, mark: impl Fn(&mut Canvas, Rect, Color)) {
    let Row { label, scale, enabled, hover, pressed } = w;
    if enabled && (hover || pressed) {
        cv.fill(r, if pressed { BUTTON_PRESSED } else { BUTTON });
    }
    let c = if enabled { tone(Tone::Normal) } else { [110, 110, 110, 255] };
    let side = (8 * scale) as f64;
    let top = r.y + (r.h - side) / 2.0;
    mark(cv, Rect::new(r.x + 2.0 * scale as f64, top, side, side), c);
    cv.text(r.x + side + 6.0 * scale as f64, r.y + (r.h - side) / 2.0, label, scale, c);
}

fn widget(cv: &mut Canvas, w: &Widget, origin: (f64, f64), scale: usize, hover: bool, pressed: bool) {
    let r = Rect::new(origin.0 + w.rect.x, origin.1 + w.rect.y, w.rect.w, w.rect.h);
    match &w.kind {
        Kind::Checkbox { label, checked, enabled } => {
            let checked = *checked;
            row(cv, r, Row { label, scale, enabled: *enabled, hover, pressed }, |cv, b, c| {
                let t = scale.max(1) as f64;
                for edge in [
                    Rect::new(b.x, b.y, b.w, t),
                    Rect::new(b.x, b.y + b.h - t, b.w, t),
                    Rect::new(b.x, b.y, t, b.h),
                    Rect::new(b.x + b.w - t, b.y, t, b.h),
                ] {
                    cv.fill(edge, c);
                }
                if checked {
                    cv.fill(Rect::new(b.x + 2.0 * t, b.y + 2.0 * t, b.w - 4.0 * t, b.h - 4.0 * t), c);
                }
            });
        }
        Kind::Choice { label, chosen, enabled } => {
            let chosen = *chosen;
            row(cv, r, Row { label, scale, enabled: *enabled, hover, pressed }, |cv, b, c| {
                cv.disc(b, false, c);
                if chosen {
                    let q = b.w / 4.0;
                    cv.disc(Rect::new(b.x + q, b.y + q, b.w - 2.0 * q, b.h - 2.0 * q), true, c);
                }
            });
        }
        Kind::Label { text, tone: t } => cv.text(r.x, r.y, text, scale, tone(*t)),
        Kind::Button { label, text_mult } => {
            let bg = if pressed { BUTTON_PRESSED } else if hover { BUTTON_HOVER } else { BUTTON };
            cv.fill(r, bg);
            let s = scale * text_mult;
            let tw = (label.chars().count() * 8 * s) as f64;
            cv.text(r.x + (r.w - tw) / 2.0, r.y + (r.h - (8 * s) as f64) / 2.0, label, s, tone(Tone::Normal));
        }
    }
}

/// The image of panel `i`: its size and straight-alpha RGBA8.
pub fn render(ui: &Ui, i: usize, scale: usize) -> (u32, u32, Vec<u8>) {
    let p: &Panel = &ui.panels()[i];
    let (w, h) = (p.rect.w.round() as usize, p.rect.h.round() as usize);
    let mut cv = Canvas { w, h, px: vec![0; w * h * 4], clip: [0, 0, w as i64, h as i64] };
    if p.frame {
        cv.fill(Rect::new(0.0, 0.0, w as f64, h as f64), BACKGROUND);
        let b = scale.max(1) as f64;
        let (wf, hf) = (w as f64, h as f64);
        for r in [
            Rect::new(0.0, 0.0, wf, b),
            Rect::new(0.0, hf - b, wf, b),
            Rect::new(0.0, 0.0, b, hf),
            Rect::new(wf - b, 0.0, b, hf),
        ] {
            cv.fill(r, BORDER);
        }
    }
    let (hover, pressed): (Option<WidgetId>, Option<WidgetId>) = (ui.hovered(i), ui.pressed(i));
    if let Some(body) = &p.body {
        let v = body.viewport;
        cv.clip = [v.x as i64, v.y as i64, (v.x + v.w) as i64, (v.y + v.h) as i64];
        for wd in &body.widgets {
            let y = v.y - body.offset;
            // Skip what is plainly out of view.
            if y + wd.rect.y + wd.rect.h < v.y || y + wd.rect.y > v.y + v.h {
                continue;
            }
            widget(&mut cv, wd, (v.x, y), scale, hover == Some(wd.id), pressed == Some(wd.id));
        }
        if body.scrolls() {
            let (track, thumb) = body.bar();
            cv.fill(track, TRACK);
            cv.fill(thumb, if ui.thumb_held(i) { THUMB_HELD } else { THUMB });
        }
        cv.clip = [0, 0, w as i64, h as i64];
    }
    for wd in &p.fixed {
        widget(&mut cv, wd, (0.0, 0.0), scale, hover == Some(wd.id), pressed == Some(wd.id));
    }
    (w as u32, h as u32, cv.px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Metrics, Panel};
    use glam::DVec2;

    fn layout(p: &mut Panel, _: &Metrics) {
        p.rect = Rect::new(10.0, 10.0, 30.0, 20.0);
        p.fixed = vec![Widget::button(1, Rect::new(2.0, 2.0, 12.0, 12.0), "x", 1)];
    }

    #[test]
    fn button_and_frame_are_painted_and_hover_changes_them() {
        let mut ui = Ui::new();
        ui.add(Panel::new("t", true, layout));
        ui.set_visible("t", true);
        let (w, h, a) = render(&ui, 0, 1);
        assert_eq!((w, h, a.len()), (30, 20, 30 * 20 * 4));
        assert!(a[3] == 255, "the border sits over the background");
        ui.pointer_moved(DVec2::new(15.0, 15.0));
        let (_, _, b) = render(&ui, 0, 1);
        assert_ne!(a, b);
    }

    fn boxes(p: &mut Panel, _: &Metrics) {
        p.rect = Rect::new(0.0, 0.0, 120.0, 40.0);
        p.fixed = vec![
            Widget::checkbox(1, Rect::new(0.0, 0.0, 120.0, 12.0), "a", false, true),
            Widget::checkbox(2, Rect::new(0.0, 14.0, 120.0, 12.0), "b", true, true),
            Widget::choice(3, Rect::new(0.0, 28.0, 120.0, 12.0), "c", true, false),
        ];
    }

    #[test]
    fn a_ticked_box_differs_from_an_empty_one_and_a_disabled_row_cannot_be_hit() {
        let mut ui = Ui::new();
        ui.add(Panel::new("t", false, boxes));
        ui.set_visible("t", true);
        let (w, _, px) = render(&ui, 0, 1);
        let at = |x: usize, y: usize| px[(y * w as usize + x) * 4 + 3];
        // The middle of the 8x8 box is filled when ticked and clear when not.
        assert_eq!(at(6, 6), 0);
        assert_eq!(at(6, 20), 255);
        assert!(ui.press(crate::nav::Button::Left, DVec2::new(5.0, 6.0)).consumed);
        let r = ui.release(crate::nav::Button::Left, DVec2::new(5.0, 6.0));
        assert!(r.event.is_some());
        ui.press(crate::nav::Button::Left, DVec2::new(5.0, 34.0));
        assert_eq!(ui.release(crate::nav::Button::Left, DVec2::new(5.0, 34.0)).event, None);
    }
}
