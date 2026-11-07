//! The help icon and the window listing the keyboard and mouse controls.
//! The list is one table, which a test compares with the key table in docs/SPEC.md.

use crate::text;
use crate::ui::{Body, Metrics, Panel, Rect, Tone, Ui, UiEvent, Widget, WidgetId};

pub const ICON: &str = "help-icon";
pub const WINDOW: &str = "help-window";
const OPEN_BUTTON: WidgetId = 1;
const CLOSE_BUTTON: WidgetId = 2;

/// Groups of (keys as written in the key table of docs/SPEC.md, what they do).
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Mouse",
        &[
            ("Drag", "Move the globe; it glides on after release"),
            ("Right-drag or Alt-drag", "Turn around the view point; tilt"),
            ("Shift-drag", "Climb or sink; look around from the same spot"),
            ("Scroll", "Zoom around the point under the pointer"),
            ("Alt-scroll / Shift-scroll", "Tilt / look around"),
            ("Double-click, Shift-double-click", "Fly one level closer / farther"),
        ],
    ),
    (
        "Flying",
        &[
            ("Arrow keys", "Fly forward, back and sideways"),
            ("Alt-arrows", "Tilt and turn around the view point"),
            ("Shift-arrows", "Climb, sink and look around"),
            ("Ctrl-Up, Ctrl-Down", "One level closer / farther"),
            ("Ctrl-0", "Whole Earth, or the whole map"),
            ("Ctrl-Alt-N", "Straighten up, one step per press"),
            ("SpaceMouse", "Climb, slide, twist and tilt; button 1 whole body, button 2 straighten"),
            ("Escape", "Stop a flight or glide; close the help window"),
        ],
    ),
    (
        "View",
        &[
            ("R", "Roads and boundaries on or off"),
            ("L", "Labels on or off"),
            ("B", "3D buildings on or off (globe)"),
            ("M", "Switch between globe and flat map"),
            ("P, Shift-P", "Map: next or previous projection"),
            ("[ and ]", "Map: central meridian west or east (1 degree with Shift)"),
            ("S", "Sunlight or flat, always-lit imagery"),
            ("O", "Stars, planets and the Moon on or off"),
            ("F", "Frame rate limit 120 or 90 fps"),
        ],
    ),
    (
        "Time",
        &[
            ("Space", "Pause or resume the clock"),
            ("- and =", "Slow down or speed up the clock"),
            (", and .", "Step the clock back or forward an hour (a day with Shift)"),
            ("T", "Back to the real time at normal speed"),
        ],
    ),
    (
        "Lists and boxes",
        &[
            ("Ctrl-F", "Search for a place"),
            ("Ctrl-S", "Name and save the view as a place"),
            ("K", "List of saved places"),
            ("I", "List of imported items"),
            ("E", "Eclipses and events; 1 to 5 filter the feeds"),
            ("G", "List of map services"),
            ("H, ?, F1", "Show or hide this window"),
        ],
    ),
    ("Program", &[("Ctrl-Q", "Quit")]),
];

pub fn icon_panel() -> Panel {
    let mut p = Panel::new(ICON, false, |p, m| {
        p.rect = m.icon_rect(0);
        p.fixed = vec![Widget::button(OPEN_BUTTON, Rect::new(0.0, 0.0, p.rect.w, p.rect.h), "?", 2)];
        p.visible = true;
    });
    p.in_band = true;
    p
}

pub fn window_panel() -> Panel {
    Panel::new(WINDOW, true, layout)
}

/// A pointer click on the icon or the close button toggles the window. True if it did.
pub fn handle(ui: &mut Ui, event: UiEvent) -> bool {
    match event {
        UiEvent::Clicked { panel: ICON, widget: OPEN_BUTTON } => ui.toggle(WINDOW),
        UiEvent::Clicked { panel: WINDOW, widget: CLOSE_BUTTON } => ui.set_visible(WINDOW, false),
        _ => return false,
    }
    true
}

/// How many text columns the list gets in a window of this width.
const WIDE_COLS: usize = 64;
const KEY_COLS: usize = 18;

fn layout(p: &mut Panel, m: &Metrics) {
    let s = m.scale;
    let (glyph, step, pad) = (8.0 * s, 10.0 * s, 6.0 * s);
    let margin = 8.0 * s;
    let bar_w = 6.0 * s;
    let header = 18.0 * s;
    let w = (m.screen.x - 2.0 * margin).min(WIDE_COLS as f64 * glyph + 2.0 * pad + bar_w).max(10.0 * glyph);
    let cols = (((w - 2.0 * pad - bar_w) / glyph) as usize).max(8);

    let mut widgets = Vec::new();
    let mut y = pad;
    let mut add = |x_col: usize, y: f64, text: &str, tone: Tone| {
        widgets.push(Widget::label(Rect::new(pad + x_col as f64 * glyph, y, text.chars().count() as f64 * glyph, glyph), text, tone));
    };
    for line in text::wrap("Scroll with the wheel. Esc closes.", cols) {
        add(0, y, &line, Tone::Dim);
        y += step;
    }
    let two_columns = cols >= KEY_COLS + 24;
    for (name, items) in SHORTCUTS {
        y += step * 0.6;
        add(0, y, name, Tone::Heading);
        y += step;
        for (keys, what) in *items {
            let (key_lines, what_lines) = if two_columns {
                (text::wrap(keys, KEY_COLS), text::wrap(what, cols - KEY_COLS - 1))
            } else {
                (text::wrap(keys, cols), text::wrap(what, cols - 2))
            };
            for (i, l) in key_lines.iter().enumerate() {
                add(0, y + i as f64 * step, l, Tone::Key);
            }
            let (x, top) = if two_columns {
                (KEY_COLS + 1, y)
            } else {
                (2, y + key_lines.len() as f64 * step)
            };
            for (i, l) in what_lines.iter().enumerate() {
                add(x, top + i as f64 * step, l, Tone::Normal);
            }
            let rows = if two_columns { key_lines.len().max(what_lines.len()) } else { key_lines.len() + what_lines.len() };
            y += rows as f64 * step + 0.3 * step;
        }
    }
    let content_h = y + pad;

    // Above the credits and icons, at the bottom right; shorter when the screen is.
    let bottom = m.floor() - 4.0 * s;
    let max_h = (bottom - margin).max(header + 4.0 * step);
    let h = (header + content_h).min(max_h);
    p.rect = Rect::new(m.screen.x - margin - w, bottom - h, w, h);
    p.fixed = vec![
        Widget::label(Rect::new(pad, (header - glyph) / 2.0, 20.0 * glyph, glyph), "Keyboard shortcuts", Tone::Heading),
        Widget::button(CLOSE_BUTTON, Rect::new(w - pad - 12.0 * s, 3.0 * s, 12.0 * s, 12.0 * s), "x", 1),
    ];
    let offset = p.body.as_ref().map_or(0.0, |b| b.offset);
    p.body = Some(Body {
        viewport: Rect::new(0.0, header, w, h - header),
        content_h,
        offset,
        step: 3.0 * step,
        bar_w,
        widgets,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    /// The first cell of each row of the key table in docs/SPEC.md.
    fn spec_keys() -> Vec<String> {
        let spec = include_str!("../docs/SPEC.md");
        let navigation = spec.split("### Navigation").nth(1).expect("Navigation section");
        let navigation = navigation.split("\n## ").next().unwrap();
        navigation
            .lines()
            .filter(|l| l.starts_with("| "))
            .filter_map(|l| l[2..].split(" | ").next())
            .map(|k| k.trim().to_string())
            .filter(|k| k != "Input" && !k.starts_with("---"))
            .collect()
    }

    #[test]
    fn every_key_of_the_spec_table_is_listed() {
        let keys = spec_keys();
        assert!(keys.len() > 30, "the table was found: {keys:?}");
        let listed: Vec<&str> = SHORTCUTS.iter().flat_map(|g| g.1.iter().map(|i| i.0)).collect();
        for k in &keys {
            assert!(listed.contains(&k.as_str()), "{k:?} is in docs/SPEC.md but not in the help table");
        }
    }

    #[test]
    fn help_table_has_no_stale_or_duplicate_entries() {
        let keys = spec_keys();
        let mut seen = Vec::new();
        for (_, items) in SHORTCUTS {
            for (k, what) in *items {
                assert!(keys.iter().any(|s| s == k), "{k:?} is not in docs/SPEC.md");
                assert!(!seen.contains(k), "{k:?} twice");
                assert!(!what.is_empty());
                seen.push(k);
            }
        }
    }

    fn ui_at(w: f64, h: f64) -> Ui {
        let mut ui = Ui::new();
        ui.resize(DVec2::new(w, h), 1.0);
        ui.add(icon_panel());
        ui.add(window_panel());
        ui
    }

    #[test]
    fn icon_toggles_the_window_and_the_close_button_hides_it() {
        use crate::nav::Button;
        let mut ui = ui_at(1280.0, 800.0);
        assert!(ui.is_visible(ICON) && !ui.is_visible(WINDOW));
        let icon = DVec2::new(1280.0 - 8.0 - 10.0, 800.0 - 8.0 - 10.0);
        ui.press(Button::Left, icon);
        let e = ui.release(Button::Left, icon).event.unwrap();
        assert!(handle(&mut ui, e));
        assert!(ui.is_visible(WINDOW));
        let r = ui.panels()[1].rect;
        let close = DVec2::new(r.x + r.w - 6.0 - 6.0, r.y + 9.0);
        ui.press(Button::Left, close);
        let e = ui.release(Button::Left, close).event.unwrap();
        assert!(handle(&mut ui, e));
        assert!(!ui.is_visible(WINDOW));
    }

    #[test]
    fn window_fits_and_scrolls_at_every_size() {
        for (w, h) in [(1280.0, 800.0), (500.0, 350.0), (240.0, 200.0)] {
            let mut ui = ui_at(w, h);
            ui.set_visible(WINDOW, true);
            let (win, icon) = (ui.panels()[1].rect, ui.panels()[0].rect);
            assert!(win.x >= 0.0 && win.y >= 0.0 && win.x + win.w <= w && win.y + win.h <= h, "{w}x{h}: {win:?}");
            assert!(win.y + win.h <= icon.y, "{w}x{h}: window sits above the icon");
            let body = ui.panels()[1].body.as_ref().unwrap();
            assert_eq!(body.scrolls(), h < 800.0, "{w}x{h}: scrolls only when the list does not fit");
            // Tall credits push the window up and make it scroll, and never lie over them.
            ui.set_credit_band(90.0);
            assert!(ui.covering_credits().is_empty(), "{w}x{h}");
            if h >= 350.0 {
                assert!(ui.panels()[1].rect.y + ui.panels()[1].rect.h <= h - 90.0, "{w}x{h}");
            }
        }
    }

    #[test]
    fn narrow_windows_stack_keys_over_descriptions() {
        let wide = ui_at(1280.0, 800.0);
        let narrow = ui_at(300.0, 800.0);
        let rows = |ui: &Ui| ui.panels()[1].body.as_ref().unwrap().content_h;
        assert!(rows(&narrow) > rows(&wide));
    }
}
