//! The help icon and the window listing the keyboard and mouse controls.
//! The list is one table, which a test compares with the key table in docs/FEATURES.md.

use crate::gui::{self, Gui};

pub const WINDOW: &str = "help-window";

/// Groups of (keys as written in the key table of docs/FEATURES.md, what they do).
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Mouse",
        &[
            ("Drag", "Move the globe; it glides on after release"),
            ("Right-drag or Alt-drag", "Turn around the view point; tilt"),
            ("Shift-drag", "Climb or sink; look around from the same spot"),
            ("Scroll", "Zoom around the point under the pointer"),
            ("Alt-scroll / Shift-scroll", "Tilt / look around"),
            ("Click", "Information about the label or place under the pointer; another click closes it"),
            ("Double-click, Shift-double-click", "Fly smoothly to the point or place; one level closer / farther"),
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
            ("Escape", "Stop a flight or glide; close the information box, the help or options window"),
        ],
    ),
    (
        "View",
        &[
            ("R", "All map lines (streets, trails, trains, boundaries) on or off"),
            ("L", "All names on or off"),
            ("B", "3D buildings on or off (globe)"),
            ("M", "Switch between globe and flat map"),
            ("P, Shift-P", "Map: next or previous projection"),
            ("[ and ]", "Map: central meridian west or east (1 degree with Shift)"),
            ("S", "Sunlight or flat, always-lit imagery"),
            ("O, W", "Stars and planets; energy layer: bars on the globe, renewable share on the map"),
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
            ("C", "Time window: sliders for the year, the day and the time of day; eclipse buttons"),
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
            ("V", "View options: choose lines, names, scene, building colours, projection, imagery"),
            ("H, ?, F1", "Show or hide this window"),
            ("F12", "Technical figures: tiles, frame rate, downloads"),
        ],
    ),
    ("Program", &[("Ctrl-Q", "Quit")]),
];

/// The help window, while it is open. `scroll` moves its list down that many points once.
pub fn show(ctx: &egui::Context, gui: &mut Gui, scroll: Option<f32>) {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return;
    }
    let screen = ctx.content_rect();
    let start = (screen.width() - 40.0).clamp(260.0, 600.0);
    let shown = gui::window("Keyboard and mouse", WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.right() - start - 30.0, screen.top() + 60.0))
        .default_width(start)
        .default_height((screen.height() - 200.0).max(150.0))
        .show(ctx, |ui| {
            let width = ui.available_width();
            let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if let Some(dy) = scroll {
                area = area.vertical_scroll_offset(dy);
            }
            area.show(ui, |ui| {
                gui::note(ui, "Esc closes this window.");
                let key_width = if width > 420.0 { 170.0 } else { width - 40.0 };
                for (name, items) in SHORTCUTS {
                    gui::section(ui, name);
                    for (keys, what) in *items {
                        if width > 420.0 {
                            ui.horizontal_top(|ui| {
                                // The keys in a column of their own, left aligned.
                                ui.allocate_ui_with_layout(egui::vec2(key_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_width(key_width);
                                    ui.add(egui::Label::new(egui::RichText::new(*keys).color(gui::accent())).wrap());
                                });
                                ui.add(egui::Label::new(*what).wrap());
                            });
                        } else {
                            ui.label(egui::RichText::new(*keys).color(gui::accent()));
                            ui.add(egui::Label::new(*what).wrap());
                        }
                    }
                }
            });
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    gui.set_visible(WINDOW, open);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first cell of each row of the key table in docs/FEATURES.md.
    fn spec_keys() -> Vec<String> {
        let doc = include_str!("../docs/FEATURES.md");
        let controls = doc.split("\n## Controls\n").nth(1).expect("Controls section");
        let controls = controls.split("\n## ").next().unwrap();
        controls
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
            assert!(listed.contains(&k.as_str()), "{k:?} is in docs/FEATURES.md but not in the help table");
        }
    }

    #[test]
    fn help_table_has_no_stale_or_duplicate_entries() {
        let keys = spec_keys();
        let mut seen = Vec::new();
        for (_, items) in SHORTCUTS {
            for (k, what) in *items {
                assert!(keys.iter().any(|s| s == k), "{k:?} is not in docs/FEATURES.md");
                assert!(!seen.contains(k), "{k:?} twice");
                assert!(!what.is_empty());
                seen.push(k);
            }
        }
    }
}
