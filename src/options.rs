//! The view options icon and dialog: checkboxes and choices over the saved settings.
//!
//! The dialog shows a [`Model`] shared with the scene, which fills it from the live state; a
//! click comes back from [`show`] as an [`Action`] for the scene to carry out. To add a
//! checkbox, add a line to [`TOGGLES`]. To add a group of choices, add rows to `show`.

use crate::atlas::Projection;
use crate::pacing::FrameLimit;
use crate::settings::{BuildingColour, Settings, ViewKind};
use crate::gui::{self, Gui};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "options-window";

type Toggle = (&'static str, fn(&Settings) -> bool, fn(&mut Settings, bool));

/// The checkboxes, in groups of (heading, rows).
const TOGGLES: &[(&str, &[Toggle])] = &[
    (
        "Map lines",
        &[
            ("Streets", |s| s.streets, |s, v| s.streets = v),
            ("Trails", |s| s.trails, |s, v| s.trails = v),
            ("Trains", |s| s.trains, |s, v| s.trains = v),
            ("Boundaries", |s| s.boundaries, |s, v| s.boundaries = v),
        ],
    ),
    (
        "Names",
        &[
            ("Cities and towns", |s| s.cities, |s, v| s.cities = v),
            ("Other names", |s| s.other_names, |s, v| s.other_names = v),
        ],
    ),
    (
        "Scene",
        &[
            ("3D buildings", |s| s.buildings, |s, v| s.buildings = v),
            ("Sunlight", |s| s.sunlight, |s, v| s.sunlight = v),
            ("Cast shadows", |s| s.cast_shadows, |s, v| s.cast_shadows = v),
            ("Stars and planets", |s| s.stars, |s, v| s.stars = v),
            ("Event markers", |s| s.event_markers, |s, v| s.event_markers = v),
            ("Rover and Apollo tracks", |s| s.missions, |s, v| s.missions = v),
            ("Information on click", |s| s.info_on_click, |s, v| s.info_on_click = v),
        ],
    ),
    (
        "Renewable energy, Germany",
        &[
            ("Capacity bars", |s| s.energy, |s, v| s.energy = v),
            ("Output at the clock's time", |s| s.energy_output, |s, v| s.energy_output = v),
            ("Include generation for own use", |s| s.energy_own_use, |s, v| s.energy_own_use = v),
            ("Day chart of output and use", |s| s.energy_chart, |s, v| s.energy_chart = v),
            ("Estimated use beside the bars", |s| s.energy_demand, |s, v| s.energy_demand = v),
            ("Solar", |s| s.energy_solar, |s, v| s.energy_solar = v),
            ("Wind on land", |s| s.energy_wind_onshore, |s, v| s.energy_wind_onshore = v),
            ("Wind at sea", |s| s.energy_wind_offshore, |s, v| s.energy_wind_offshore = v),
            ("Hydro", |s| s.energy_hydro, |s, v| s.energy_hydro = v),
            ("Biomass", |s| s.energy_biomass, |s, v| s.energy_biomass = v),
        ],
    ),
    (
        "For comparison: fossil and nuclear",
        &[
            ("Lignite", |s| s.energy_lignite, |s, v| s.energy_lignite = v),
            ("Hard coal", |s| s.energy_hard_coal, |s, v| s.energy_hard_coal = v),
            ("Natural gas", |s| s.energy_gas, |s, v| s.energy_gas = v),
            ("Oil", |s| s.energy_oil, |s, v| s.energy_oil = v),
            ("Other fossil (gases, waste)", |s| s.energy_other_fossil, |s, v| s.energy_other_fossil = v),
            ("Nuclear", |s| s.energy_nuclear, |s, v| s.energy_nuclear = v),
        ],
    ),
];

/// Sets the checkbox number `index` of [`TOGGLES`].
pub fn set_toggle(s: &mut Settings, index: usize, value: bool) {
    if let Some(t) = TOGGLES.iter().flat_map(|g| g.1.iter()).nth(index) {
        (t.2)(s, value);
    }
}

/// What the dialog shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub settings: Settings,
    /// Services can be shown on Earth only.
    pub earth: bool,
    /// The library's services, as (identifier, name).
    pub library: Vec<(String, String)>,
}

impl Default for Model {
    fn default() -> Self {
        Self { settings: Settings::default(), earth: true, library: Vec::new() }
    }
}

pub type Shared = Rc<RefCell<Model>>;

/// What a click asks the scene to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The checkbox with this index in [`TOGGLES`] to this value.
    Set(usize, bool),
    View(ViewKind),
    BuildingColour(BuildingColour),
    Projection(Projection),
    /// Another body to show.
    Body(&'static str),
    /// A library service as the base, or the usual imagery.
    Base(Option<String>),
    FrameRate(FrameLimit),
    /// Open or close the energy display window.
    EnergyStyle,
}

/// The dialog, while it is open; a click comes back as an action.
pub fn show(ctx: &egui::Context, gui: &mut Gui, model: &Model) -> Option<Action> {
    let mut open = gui.is_visible(WINDOW);
    if !open {
        return None;
    }
    let mut action = None;
    let screen = ctx.content_rect();
    let st = &model.settings;
    let shown = gui::window("View options", WINDOW)
        .open(&mut open)
        .default_pos(egui::pos2(screen.left() + 12.0, screen.top() + 60.0))
        .default_width(300.0)
        .default_height((screen.height() - 160.0).max(150.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut n = 0;
                for (name, group) in TOGGLES {
                    gui::section(ui, name);
                    for (label, get, _) in *group {
                        let mut on = get(st);
                        if ui.checkbox(&mut on, *label).changed() {
                            action = Some(Action::Set(n, on));
                        }
                        n += 1;
                    }
                    if *name == "Scene" {
                        gui::section(ui, "Building colours");
                        ui.horizontal(|ui| {
                            for (label, c) in [("Aerial roofs", BuildingColour::Aerial), ("Zones", BuildingColour::Zones)] {
                                if ui.radio(st.buildings_colour == c, label).clicked() {
                                    action = Some(Action::BuildingColour(c));
                                }
                            }
                        });
                    }
                    if name.starts_with("For comparison") && ui.button("Colours and bar size\u{2026}").clicked() {
                        action = Some(Action::EnergyStyle);
                    }
                }

                gui::section(ui, "View");
                ui.horizontal(|ui| {
                    for (label, kind) in [("Globe", ViewKind::Globe), ("Flat map", ViewKind::Map)] {
                        if ui.radio(st.view == kind, label).clicked() {
                            action = Some(Action::View(kind));
                        }
                    }
                });

                gui::section(ui, "Map projection");
                let map = st.view == ViewKind::Map;
                if !map {
                    gui::note(ui, "Used on the flat map. Choose it above to change these.");
                }
                ui.add_enabled_ui(map, |ui| {
                    for p in Projection::ALL {
                        if ui.radio(st.projection == p, p.name()).clicked() {
                            action = Some(Action::Projection(p));
                        }
                    }
                });

                gui::section(ui, "The globe shows");
                for b in crate::body::all() {
                    if ui.radio(st.body == b.id, b.name).clicked() {
                        action = Some(Action::Body(b.id));
                    }
                }

                gui::section(ui, "Earth imagery");
                if !model.earth {
                    gui::note(ui, "Map services are shown on Earth only.");
                }
                ui.add_enabled_ui(model.earth, |ui| {
                    if ui.radio(st.base_service.is_empty(), "Usual imagery").clicked() {
                        action = Some(Action::Base(None));
                    }
                    for (id, name) in &model.library {
                        if ui.radio(st.base_service == *id, name.as_str()).clicked() {
                            action = Some(Action::Base(Some(id.clone())));
                        }
                    }
                });
                if model.library.is_empty() {
                    gui::note(ui, "Services you add to your library (key G) are listed here.");
                }

                gui::section(ui, "Frame rate limit");
                ui.horizontal(|ui| {
                    for (label, limit) in [("120 fps", FrameLimit::Fps120), ("90 fps", FrameLimit::Fps90)] {
                        if ui.radio(st.frame_rate == limit, label).clicked() {
                            action = Some(Action::FrameRate(limit));
                        }
                    }
                });
                ui.add_space(6.0);
                gui::note(ui, "Changes are saved as you make them. Esc closes.");
            });
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    gui.set_visible(WINDOW, open);
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_toggle_sets_its_own_setting() {
        let n = TOGGLES.iter().map(|g| g.1.len()).sum::<usize>();
        for i in 0..n {
            let mut s = Settings::default();
            let t = TOGGLES.iter().flat_map(|g| g.1.iter()).nth(i).unwrap();
            let before = (t.1)(&s);
            set_toggle(&mut s, i, !before);
            assert_eq!((t.1)(&s), !before);
            let mut other = Settings::default();
            set_toggle(&mut other, i, before);
            assert_eq!(other, Settings::default(), "setting a toggle to its value changes nothing else");
        }
    }
}
