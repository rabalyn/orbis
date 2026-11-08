//! Paints a panel of the interface into an RGBA image, with the built-in bitmap font.

use crate::text::glyph;
use crate::ui::{slider_thumb, Chart, Kind, Panel, Rect, Tone, Ui, Widget, WidgetId, CHART_LEGEND_COLS};

type Color = [u8; 4];

const BACKGROUND: Color = [8, 10, 16, 255];
const BORDER: Color = [255, 255, 255, 80];
const BUTTON: Color = [255, 255, 255, 45];
const BUTTON_HOVER: Color = [255, 255, 255, 100];
const BUTTON_PRESSED: Color = [255, 255, 255, 150];
/// The north end of the compass needle.
const NEEDLE_NORTH: Color = [230, 60, 50, 255];
const TRACK: Color = [255, 255, 255, 35];
const THUMB: Color = [255, 255, 255, 120];
const THUMB_HELD: Color = [255, 255, 255, 190];

fn tone(t: Tone) -> Color {
    match t {
        Tone::Normal => [255, 255, 255, 255],
        Tone::Heading => [255, 215, 120, 255],
        Tone::Key => [150, 210, 255, 255],
        Tone::Dim => [180, 180, 180, 255],
        Tone::Warn => [255, 150, 90, 255],
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

    /// A filled triangle, its edges smoothed by sampling each pixel 4 by 4.
    fn triangle(&mut self, p: [(f64, f64); 3], c: Color) {
        let edge = |a: (f64, f64), b: (f64, f64), x: f64, y: f64| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
        let area = edge(p[0], p[1], p[2].0, p[2].1);
        if area.abs() < 1e-9 {
            return;
        }
        let xs = p.map(|q| q.0);
        let ys = p.map(|q| q.1);
        let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
        let (y0, y1) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
        for y in y0.floor() as i64..y1.ceil() as i64 {
            for x in x0.floor() as i64..x1.ceil() as i64 {
                let mut inside = 0u32;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (fx, fy) = (x as f64 + (sx as f64 + 0.5) / 4.0, y as f64 + (sy as f64 + 0.5) / 4.0);
                        let w = [edge(p[1], p[2], fx, fy), edge(p[2], p[0], fx, fy), edge(p[0], p[1], fx, fy)];
                        if w.iter().all(|w| w * area >= 0.0) {
                            inside += 1;
                        }
                    }
                }
                if inside > 0 {
                    self.put(x, y, [c[0], c[1], c[2], (c[3] as u32 * inside / 16) as u8]);
                }
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
        Kind::Slider { pos, thumb_w } => {
            let t = scale.max(1) as f64;
            cv.fill(Rect::new(r.x, r.y + (r.h - 2.0 * t) / 2.0, r.w, 2.0 * t), [255, 255, 255, 70]);
            let c = if pressed { THUMB_HELD } else if hover { [255, 255, 255, 160] } else { THUMB };
            cv.fill(slider_thumb(r, *thumb_w, *pos), c);
        }
        Kind::ClockFace => {
            let bg = if pressed { BUTTON_PRESSED } else if hover { BUTTON_HOVER } else { BUTTON };
            cv.fill(r, bg);
            let t = scale.max(1) as f64;
            let white = tone(Tone::Normal);
            let face = Rect::new(r.x + 4.0 * t, r.y + 4.0 * t, r.w - 8.0 * t, r.h - 8.0 * t);
            cv.disc(face, false, white);
            let (cx, cy, rad) = (face.x + face.w / 2.0, face.y + face.h / 2.0, face.w / 2.0);
            // Minute hand up, hour hand towards four o'clock, and a dot in the middle.
            cv.fill(Rect::new(cx - t / 2.0, cy - rad * 0.7, t, rad * 0.7), white);
            cv.fill(Rect::new(cx, cy, rad * 0.45, t), white);
            cv.fill(Rect::new(cx - t, cy - t, 2.0 * t, 2.0 * t), white);
        }
        Kind::Compass { heading } => {
            let bg = if pressed { BUTTON_PRESSED } else if hover { BUTTON_HOVER } else { BUTTON };
            cv.fill(r, bg);
            let t = scale.max(1) as f64;
            let face = Rect::new(r.x + 3.0 * t, r.y + 3.0 * t, r.w - 6.0 * t, r.h - 6.0 * t);
            cv.disc(face, false, tone(Tone::Dim));
            let (cx, cy, rad) = (face.x + face.w / 2.0, face.y + face.h / 2.0, face.w / 2.0);
            // North on the screen, and the needle's side; y grows downwards.
            let (s, c) = heading.sin_cos();
            let north = (-s, -c);
            let side = (c, -s);
            let at = |along: f64, across: f64| (cx + north.0 * along + side.0 * across, cy + north.1 * along + side.1 * across);
            let (tip, half) = (rad * 0.8, rad * 0.26);
            cv.triangle([at(tip, 0.0), at(0.0, half), at(0.0, -half)], NEEDLE_NORTH);
            cv.triangle([at(-tip, 0.0), at(0.0, -half), at(0.0, half)], tone(Tone::Normal));
        }
        Kind::Label { text, tone: t } => cv.text(r.x, r.y, text, scale, tone(*t)),
        Kind::Chart(c) => chart(cv, r, c, scale),
        Kind::Swatch { colour, hatched } => {
            cv.fill(r, *colour);
            if *hatched {
                hatch(cv, r, [colour[0], colour[1], colour[2]], scale.max(1) as f64);
            }
        }
        Kind::Button { label, text_mult } => {
            let bg = if pressed { BUTTON_PRESSED } else if hover { BUTTON_HOVER } else { BUTTON };
            cv.fill(r, bg);
            let s = scale * text_mult;
            let tw = (label.chars().count() * 8 * s) as f64;
            cv.text(r.x + (r.w - tw) / 2.0, r.y + (r.h - (8 * s) as f64) / 2.0, label, s, tone(Tone::Normal));
        }
    }
}

/// A day chart: the axis on the left, the plot, the hours below it, then the legend.
fn chart(cv: &mut Canvas, r: Rect, c: &Chart, scale: usize) {
    let s = scale.max(1) as f64;
    let glyph = 8.0 * s;
    let dim = tone(Tone::Dim);
    let (per_row, rows) = c.legend_rows(r.w, s);
    let legend_h = rows as f64 * 11.0 * s;
    let left = 5.0 * glyph;
    let plot = Rect::new(r.x + left, r.y + glyph / 2.0, r.w - left - glyph, r.h - legend_h - 2.5 * glyph);
    let steps = c.line.1.len().max(c.layers.iter().map(|l| l.2.len()).max().unwrap_or(0)).max(1);
    let stack = |i: usize| c.layers.iter().map(|l| l.2.get(i).copied().flatten().unwrap_or(0.0)).sum::<f32>();
    let top = (0..steps).map(|i| stack(i).max(c.line.1.get(i).copied().flatten().unwrap_or(0.0))).fold(0.0f32, f32::max);
    // The axis goes up in whole steps of 10 units.
    let tick = 10.0 * c.unit.1;
    let max = ((top / tick).ceil().max(1.0) * tick) as f64;
    let y_of = |v: f64| plot.y + plot.h - v / max * plot.h;
    cv.fill(plot, [255, 255, 255, 12]);
    let mut v = 0.0;
    while v <= max + 1.0 {
        let y = y_of(v);
        cv.fill(Rect::new(plot.x, y, plot.w, 1.0), [255, 255, 255, 30]);
        let label = format!("{:>3.0}", v / c.unit.1 as f64);
        cv.text(r.x, y - glyph / 2.0, &label, scale, dim);
        v += tick as f64;
    }
    // The areas, one pixel column at a time.
    let px_w = plot.w.round().max(1.0) as i64;
    for px in 0..px_w {
        let i = ((px as f64 + 0.5) / plot.w * steps as f64) as usize;
        let mut base = 0.0f64;
        for (colour, _, values, hatched) in &c.layers {
            let Some(v) = values.get(i).copied().flatten() else { continue };
            let (y0, y1) = (y_of(base + v as f64), y_of(base));
            let x = plot.x + px as f64;
            cv.fill(Rect::new(x, y0, 1.0, (y1 - y0).max(0.0)), [colour[0], colour[1], colour[2], 235]);
            if *hatched {
                hatch(cv, Rect::new(x, y0, 1.0, (y1 - y0).max(0.0)), *colour, s);
            }
            base += v as f64;
        }
    }
    // The line, two pixels thick and joined between columns.
    let mut last: Option<f64> = None;
    for px in 0..px_w {
        let i = ((px as f64 + 0.5) / plot.w * steps as f64) as usize;
        match c.line.1.get(i).copied().flatten() {
            Some(v) => {
                let y = y_of(v as f64);
                let (a, b) = last.map_or((y, y), |l| (l.min(y), l.max(y)));
                cv.fill(Rect::new(plot.x + px as f64, a - s, 1.0, b - a + 2.0 * s), [255, 255, 255, 255]);
                last = Some(y);
            }
            None => last = None,
        }
    }
    if let Some(m) = c.marker {
        let x = plot.x + m.clamp(0.0, 1.0) * plot.w;
        cv.fill(Rect::new(x - s / 2.0, plot.y, s, plot.h), [255, 230, 90, 255]);
    }
    for h in (0..=24).step_by(3) {
        let x = plot.x + plot.w * h as f64 / 24.0;
        cv.fill(Rect::new(x, plot.y + plot.h, 1.0, 3.0 * s), dim);
        let label = format!("{h:02}");
        cv.text(x - glyph, plot.y + plot.h + 4.0 * s, &label, scale, dim);
    }
    // The legend: the line first, then the areas from the top of the stack down.
    let mut items: Vec<([u8; 4], &str, bool)> = vec![([255, 255, 255, 255], c.line.0.as_str(), false)];
    items.extend(c.layers.iter().rev().map(|l| ([l.0[0], l.0[1], l.0[2], 255], l.1.as_str(), l.3)));
    let ly = r.y + r.h - legend_h;
    for (k, (colour, name, hatched)) in items.iter().enumerate() {
        let x = r.x + (k % per_row) as f64 * CHART_LEGEND_COLS as f64 * glyph;
        let y = ly + (k / per_row) as f64 * 11.0 * s;
        if k == 0 {
            cv.fill(Rect::new(x, y + glyph / 2.0 - s, glyph, 2.0 * s), *colour);
        } else {
            cv.fill(Rect::new(x, y, glyph, glyph), *colour);
            if *hatched {
                hatch(cv, Rect::new(x, y, glyph, glyph), [colour[0], colour[1], colour[2]], s);
            }
        }
        let name: String = name.chars().take(CHART_LEGEND_COLS - 2).collect();
        cv.text(x + glyph * 1.5, y, &name, scale, tone(Tone::Normal));
    }
}

/// Darkens diagonal stripes of `r`, as the hatching of a fossil area: the stripes lie on lines of
/// equal x + y on the canvas, so neighbouring columns join up into one pattern.
fn hatch(cv: &mut Canvas, r: Rect, colour: [u8; 3], s: f64) {
    let step = (5.0 * s).round().max(3.0) as i64;
    let on = (2.0 * s).round().max(1.0) as i64;
    let dark = colour.map(|c| (c as f32 * 0.4) as u8);
    let (x0, x1) = (r.x.round() as i64, (r.x + r.w).round() as i64);
    let (y0, y1) = (r.y.round() as i64, (r.y + r.h).round() as i64);
    for x in x0..x1 {
        let mut y = y0;
        while y < y1 {
            // Runs of `on` pixels out of each `step`, down the column.
            let phase = (x + y).rem_euclid(step);
            if phase < on {
                let run = (on - phase).min(y1 - y);
                cv.fill(Rect::new(x as f64, y as f64, 1.0, run as f64), [dark[0], dark[1], dark[2], 255]);
                y += run;
            } else {
                y += step - phase;
            }
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
