//! The view options icon and dialog: checkboxes and choices over the saved settings.
//!
//! The dialog shows a [`Model`] shared with the scene, which fills it from the live state; a
//! click comes back from [`handle`] as an [`Action`] for the scene to carry out. To add a
//! checkbox, add a line to [`TOGGLES`]. To add a group of choices, give it an id range, rows in
//! `layout`, and an arm in `action_for`.

use crate::atlas::Projection;
use crate::pacing::FrameLimit;
use crate::settings::{Settings, ViewKind};
use crate::text;
use crate::ui::{Body, Metrics, Panel, Rect, Tone, Ui, UiEvent, Widget, WidgetId};
use std::cell::RefCell;
use std::rc::Rc;

pub const ICON: &str = "options-icon";
pub const WINDOW: &str = "options-window";
const OPEN_BUTTON: WidgetId = 1;
const CLOSE_BUTTON: WidgetId = 2;
const TOGGLE_BASE: WidgetId = 10;
const VIEW_BASE: WidgetId = 100;
const PROJECTION_BASE: WidgetId = 200;
const BODY_BASE: WidgetId = 300;
/// The usual imagery; the library's services follow.
const IMAGERY_BASE: WidgetId = 400;
const FPS_BASE: WidgetId = 500;
const ENERGY_STYLE_BUTTON: WidgetId = 600;

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
            ("Information on click", |s| s.info_on_click, |s, v| s.info_on_click = v),
        ],
    ),
    (
        "Renewable energy, Germany",
        &[
            ("Capacity bars", |s| s.energy, |s, v| s.energy = v),
            ("Output at the clock's time", |s| s.energy_output, |s, v| s.energy_output = v),
            ("Day chart of output and use", |s| s.energy_chart, |s, v| s.energy_chart = v),
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
    /// The body of this run. The settings may name another, for the next start.
    pub running_body: &'static str,
    /// Services can be shown on Earth only.
    pub earth: bool,
    /// The library's services, as (identifier, name).
    pub library: Vec<(String, String)>,
}

impl Default for Model {
    fn default() -> Self {
        Self { settings: Settings::default(), running_body: "earth", earth: true, library: Vec::new() }
    }
}

pub type Shared = Rc<RefCell<Model>>;

/// What a click asks the scene to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The checkbox with this index in [`TOGGLES`] to this value.
    Set(usize, bool),
    View(ViewKind),
    Projection(Projection),
    /// The body for the next start.
    Body(&'static str),
    /// A library service as the base, or the usual imagery.
    Base(Option<String>),
    FrameRate(FrameLimit),
    /// Open or close the energy display window.
    EnergyStyle,
}

pub fn icon_panel() -> Panel {
    let mut p = Panel::new(ICON, false, |p, m| {
        p.rect = m.icon_rect(1);
        p.fixed = vec![Widget::button(OPEN_BUTTON, Rect::new(0.0, 0.0, p.rect.w, p.rect.h), "=", 2)];
        p.visible = true;
    });
    p.in_band = true;
    p
}

pub fn window_panel(model: Shared) -> Panel {
    Panel::new(WINDOW, true, move |p, m| layout(p, m, &model.borrow()))
}

/// Opens or closes the dialog for a click on the icon or its close button. Anything else that
/// the dialog was clicked for is returned as an action.
pub fn handle(ui: &mut Ui, model: &Shared, event: UiEvent) -> (bool, Option<Action>) {
    let UiEvent::Clicked { panel, widget } = event else { return (false, None) };
    match (panel, widget) {
        (ICON, OPEN_BUTTON) => ui.toggle(WINDOW),
        (WINDOW, CLOSE_BUTTON) => ui.set_visible(WINDOW, false),
        (WINDOW, id) => return (true, action_for(id, &model.borrow())),
        _ => return (false, None),
    }
    (true, None)
}

fn action_for(id: WidgetId, m: &Model) -> Option<Action> {
    let toggles: Vec<&Toggle> = TOGGLES.iter().flat_map(|g| g.1.iter()).collect();
    match id {
        i if (TOGGLE_BASE..TOGGLE_BASE + toggles.len() as u32).contains(&i) => {
            let t = toggles[(i - TOGGLE_BASE) as usize];
            Some(Action::Set((i - TOGGLE_BASE) as usize, !(t.1)(&m.settings)))
        }
        VIEW_BASE => Some(Action::View(ViewKind::Globe)),
        i if i == VIEW_BASE + 1 => Some(Action::View(ViewKind::Map)),
        i if (PROJECTION_BASE..PROJECTION_BASE + Projection::ALL.len() as u32).contains(&i) => {
            Some(Action::Projection(Projection::ALL[(i - PROJECTION_BASE) as usize]))
        }
        i if (BODY_BASE..BODY_BASE + crate::body::all().len() as u32).contains(&i) => {
            Some(Action::Body(crate::body::all()[(i - BODY_BASE) as usize].id))
        }
        IMAGERY_BASE => Some(Action::Base(None)),
        i if (IMAGERY_BASE + 1..IMAGERY_BASE + 1 + m.library.len() as u32).contains(&i) => {
            Some(Action::Base(Some(m.library[(i - IMAGERY_BASE - 1) as usize].0.clone())))
        }
        ENERGY_STYLE_BUTTON => Some(Action::EnergyStyle),
        FPS_BASE => Some(Action::FrameRate(FrameLimit::Fps120)),
        i if i == FPS_BASE + 1 => Some(Action::FrameRate(FrameLimit::Fps90)),
        _ => None,
    }
}

const COLS: usize = 40;

/// Width of the dialog, which other dialogs beside it need to know.
pub fn window_width(m: &Metrics) -> f64 {
    let (glyph, pad, margin, bar_w) = (8.0 * m.scale, 6.0 * m.scale, 8.0 * m.scale, 6.0 * m.scale);
    (m.screen.x - 2.0 * margin).min(COLS as f64 * glyph + 2.0 * pad + bar_w).max(14.0 * glyph)
}

fn layout(p: &mut Panel, m: &Metrics, model: &Model) {
    let s = m.scale;
    let (glyph, step, pad) = (8.0 * s, 10.0 * s, 6.0 * s);
    let (row, margin, bar_w, header) = (12.0 * s, 8.0 * s, 6.0 * s, 18.0 * s);
    let w = window_width(m);
    let cols = (((w - 2.0 * pad - bar_w) / glyph) as usize).max(10);
    let inner = w - 2.0 * pad - bar_w;
    let st = &model.settings;

    let mut widgets = Vec::new();
    let mut y = pad;
    let heading = |widgets: &mut Vec<Widget>, y: &mut f64, text: &str| {
        *y += step * 0.6;
        widgets.push(Widget::label(Rect::new(pad, *y, text.chars().count() as f64 * glyph, glyph), text, Tone::Heading));
        *y += step * 1.2;
    };
    let note = |widgets: &mut Vec<Widget>, y: &mut f64, text: &str| {
        for line in text::wrap(text, cols) {
            widgets.push(Widget::label(Rect::new(pad, *y, line.chars().count() as f64 * glyph, glyph), &line, Tone::Dim));
            *y += step;
        }
    };
    let cell = |y: f64| Rect::new(pad, y, inner, row);

    let mut n = 0;
    for (name, group) in TOGGLES {
        heading(&mut widgets, &mut y, name);
        for (label, get, _) in *group {
            widgets.push(Widget::checkbox(TOGGLE_BASE + n, cell(y), label, get(st), true));
            n += 1;
            y += row;
        }
        if name.starts_with("For comparison") {
            y += step * 0.4;
            let text = "Colours and bar size...";
            widgets.push(Widget::button(ENERGY_STYLE_BUTTON, Rect::new(pad, y, (text.chars().count() + 2) as f64 * glyph, 14.0 * s), text, 1));
            y += 14.0 * s;
        }
    }

    heading(&mut widgets, &mut y, "View");
    let map = st.view == ViewKind::Map;
    for (i, (label, kind)) in [("Globe", ViewKind::Globe), ("Flat map", ViewKind::Map)].into_iter().enumerate() {
        widgets.push(Widget::choice(VIEW_BASE + i as u32, cell(y), label, st.view == kind, true));
        y += row;
    }

    heading(&mut widgets, &mut y, "Map projection");
    if !map {
        note(&mut widgets, &mut y, "Used on the flat map. Choose it above to change these.");
    }
    for (i, p) in Projection::ALL.iter().enumerate() {
        widgets.push(Widget::choice(PROJECTION_BASE + i as u32, cell(y), p.name(), st.projection == *p, map));
        y += row;
    }

    heading(&mut widgets, &mut y, "The globe shows");
    for (i, b) in crate::body::all().iter().enumerate() {
        widgets.push(Widget::choice(BODY_BASE + i as u32, cell(y), b.name, st.body == b.id, true));
        y += row;
    }
    if st.body != model.running_body {
        note(&mut widgets, &mut y, "This takes effect the next time Orbis starts.");
    }

    heading(&mut widgets, &mut y, "Earth imagery");
    if !model.earth {
        note(&mut widgets, &mut y, "Map services are shown on Earth only.");
    }
    widgets.push(Widget::choice(IMAGERY_BASE, cell(y), "Usual imagery", st.base_service.is_empty(), model.earth));
    y += row;
    for (i, (id, name)) in model.library.iter().enumerate() {
        let label: String = name.chars().take(cols.saturating_sub(4)).collect();
        widgets.push(Widget::choice(IMAGERY_BASE + 1 + i as u32, cell(y), &label, st.base_service == *id, model.earth));
        y += row;
    }
    if model.library.is_empty() {
        note(&mut widgets, &mut y, "Services you add to your library (key G) are listed here.");
    }

    heading(&mut widgets, &mut y, "Frame rate limit");
    for (i, (label, limit)) in [("120 frames per second", FrameLimit::Fps120), ("90 frames per second", FrameLimit::Fps90)].into_iter().enumerate() {
        widgets.push(Widget::choice(FPS_BASE + i as u32, cell(y), label, st.frame_rate == limit, true));
        y += row;
    }
    y += step * 0.6;
    note(&mut widgets, &mut y, "Changes are saved as you make them. Esc closes.");
    let content_h = y + pad;

    // Bottom left above the credits, away from the help window at the bottom right.
    let bottom = m.floor() - 4.0 * s;
    let max_h = (bottom - margin).max(header + 4.0 * step);
    let h = (header + content_h).min(max_h);
    p.rect = Rect::new(margin, bottom - h, w, h);
    p.fixed = vec![
        Widget::label(Rect::new(pad, (header - glyph) / 2.0, 20.0 * glyph, glyph), "View options", Tone::Heading),
        Widget::button(CLOSE_BUTTON, Rect::new(w - pad - 12.0 * s, 3.0 * s, 12.0 * s, 12.0 * s), "x", 1),
    ];
    let offset = p.body.as_ref().map_or(0.0, |b| b.offset);
    p.body = Some(Body { viewport: Rect::new(0.0, header, w, h - header), content_h, offset, step: 3.0 * row, bar_w, widgets });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::Button;
    use crate::ui::Kind;
    use glam::DVec2;

    fn setup(w: f64, h: f64, model: Model) -> (Ui, Shared) {
        let shared: Shared = Rc::new(RefCell::new(model));
        let mut ui = Ui::new();
        ui.resize(DVec2::new(w, h), 1.0);
        ui.add(crate::help::icon_panel());
        ui.add(crate::help::window_panel());
        ui.add(icon_panel());
        ui.add(window_panel(shared.clone()));
        (ui, shared)
    }

    fn click(ui: &mut Ui, shared: &Shared, p: DVec2) -> (bool, Option<Action>) {
        ui.pointer_moved(p);
        ui.press(Button::Left, p);
        match ui.release(Button::Left, p).event {
            Some(e) => handle(ui, shared, e),
            None => (false, None),
        }
    }

    fn widgets(ui: &Ui) -> &[Widget] {
        &ui.panels().iter().find(|p| p.id == WINDOW).unwrap().body.as_ref().unwrap().widgets
    }

    /// Scrolls the dialog until the row is in view, then gives where to click it.
    fn reveal(ui: &mut Ui, text: &str) -> DVec2 {
        for _ in 0..200 {
            let at = row_of(ui, text);
            let p = ui.panels().iter().find(|p| p.id == WINDOW).unwrap();
            let v = p.body.as_ref().unwrap().viewport;
            let (top, bottom) = (p.rect.y + v.y, p.rect.y + v.y + v.h);
            if at.y > top + 2.0 && at.y < bottom - 2.0 {
                return at;
            }
            let middle = DVec2::new(p.rect.x + p.rect.w / 2.0, (top + bottom) / 2.0);
            ui.wheel(if at.y >= bottom - 2.0 { -1.0 } else { 1.0 }, middle);
        }
        panic!("{text} does not come into view");
    }

    fn row_of(ui: &Ui, text: &str) -> DVec2 {
        let p = ui.panels().iter().find(|p| p.id == WINDOW).unwrap();
        let b = p.body.as_ref().unwrap();
        let w = b.widgets.iter().find(|w| matches!(&w.kind, Kind::Checkbox { label, .. } | Kind::Choice { label, .. } if label == text)).unwrap();
        DVec2::new(p.rect.x + b.viewport.x + w.rect.x + 10.0, p.rect.y + b.viewport.y + w.rect.y - b.offset + 5.0)
    }

    #[test]
    fn the_icon_opens_and_the_close_button_closes() {
        let (mut ui, shared) = setup(1280.0, 800.0, Model::default());
        let icon = ui.panels()[2].rect;
        let (handled, action) = click(&mut ui, &shared, DVec2::new(icon.x + 5.0, icon.y + 5.0));
        assert!(handled && action.is_none() && ui.is_visible(WINDOW));
        let r = ui.panels()[3].rect;
        click(&mut ui, &shared, DVec2::new(r.x + r.w - 12.0, r.y + 9.0));
        assert!(!ui.is_visible(WINDOW));
    }

    #[test]
    fn the_dialog_shows_the_state_and_a_click_asks_for_the_opposite() {
        let mut model = Model::default();
        model.settings.trails = false;
        let (mut ui, shared) = setup(1280.0, 800.0, model);
        ui.set_visible(WINDOW, true);
        let state = |ui: &Ui, text: &str| widgets(ui).iter().find_map(|w| match &w.kind {
            Kind::Checkbox { label, checked, .. } if label == text => Some(*checked),
            _ => None,
        });
        assert_eq!((state(&ui, "Trails"), state(&ui, "Streets")), (Some(false), Some(true)));
        let at = row_of(&ui, "Trails");
        let (_, action) = click(&mut ui, &shared, at);
        let Some(Action::Set(index, value)) = action else { panic!("{action:?}") };
        assert!(value);
        let mut s = Settings { trails: false, ..Settings::default() };
        set_toggle(&mut s, index, value);
        assert!(s.trails && s.streets);
        let at = row_of(&ui, "Streets");
        let (_, action) = click(&mut ui, &shared, at);
        let Some(Action::Set(index, value)) = action else { panic!() };
        let mut s = Settings::default();
        set_toggle(&mut s, index, value);
        assert!(!s.streets && !value);
    }

    #[test]
    fn projections_are_usable_only_on_the_map() {
        let (mut ui, shared) = setup(1280.0, 800.0, Model::default());
        ui.set_visible(WINDOW, true);
        let enabled = |ui: &Ui| widgets(ui).iter().find_map(|w| match &w.kind {
            Kind::Choice { label, enabled, .. } if label == "Mercator" => Some(*enabled),
            _ => None,
        });
        assert_eq!(enabled(&ui), Some(false));
        let at = row_of(&ui, "Mercator");
        assert_eq!(click(&mut ui, &shared, at).1, None, "a disabled row asks for nothing");
        shared.borrow_mut().settings.view = ViewKind::Map;
        ui.relayout_for_test();
        assert_eq!(enabled(&ui), Some(true));
        let at = row_of(&ui, "Mercator");
        assert_eq!(click(&mut ui, &shared, at).1, Some(Action::Projection(Projection::Mercator)));
    }

    #[test]
    fn choices_name_the_body_the_imagery_and_the_frame_rate() {
        let model = Model { library: vec![("osm".into(), "OpenStreetMap".into())], ..Model::default() };
        let (mut ui, shared) = setup(1280.0, 800.0, model);
        ui.set_visible(WINDOW, true);
        for (label, want) in [
            ("Mars", Some(Action::Body("mars"))),
            ("Usual imagery", Some(Action::Base(None))),
            ("OpenStreetMap", Some(Action::Base(Some("osm".into())))),
            ("90 frames per second", Some(Action::FrameRate(FrameLimit::Fps90))),
            ("Flat map", Some(Action::View(ViewKind::Map))),
        ] {
            let at = reveal(&mut ui, label);
            assert_eq!(click(&mut ui, &shared, at).1, want, "{label}");
        }
    }

    #[test]
    fn a_button_opens_the_energy_display_window() {
        let (mut ui, shared) = setup(1280.0, 800.0, Model::default());
        ui.set_visible(WINDOW, true);
        // Where the button is, and the bottom of the dialog's view.
        let place = |ui: &Ui| {
            let p = ui.panels().iter().find(|p| p.id == WINDOW).unwrap();
            let b = p.body.as_ref().unwrap();
            let w = b.widgets.iter().find(|w| w.id == ENERGY_STYLE_BUTTON).unwrap();
            let at = DVec2::new(p.rect.x + b.viewport.x + w.rect.x + 10.0, p.rect.y + b.viewport.y + w.rect.y - b.offset + 5.0);
            (at, p.rect.y + p.rect.h, DVec2::new(p.rect.x + 20.0, p.rect.y + p.rect.h - 20.0))
        };
        for _ in 0..100 {
            let (at, bottom, wheel_at) = place(&ui);
            if at.y < bottom - 4.0 {
                break;
            }
            ui.wheel(-1.0, wheel_at);
        }
        let (at, ..) = place(&ui);
        assert_eq!(click(&mut ui, &shared, at).1, Some(Action::EnergyStyle));
    }

    #[test]
    fn a_body_for_the_next_start_says_so() {
        let mut model = Model::default();
        let texts = |ui: &Ui| widgets(ui).iter().filter_map(|w| match &w.kind {
            Kind::Label { text, .. } => Some(text.clone()),
            _ => None,
        }).collect::<Vec<_>>().join(" ");
        let (mut ui, shared) = setup(1280.0, 800.0, model.clone());
        assert!(!texts(&ui).contains("next time"));
        model.settings.body = "moon".into();
        *shared.borrow_mut() = model;
        ui.relayout_for_test();
        assert!(texts(&ui).contains("next time"));
    }

    #[test]
    fn fits_scrolls_and_leaves_the_credits_at_every_size() {
        for (w, h) in [(1280.0, 800.0), (1024.0, 600.0), (500.0, 350.0)] {
            let (mut ui, _) = setup(w, h, Model::default());
            ui.set_visible(WINDOW, true);
            ui.set_credit_band(70.0);
            let r = ui.panels()[3].rect;
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= w && r.y + r.h <= h - 70.0, "{w}x{h}: {r:?}");
            assert!(ui.covering_credits().is_empty());
            assert!(ui.panels()[3].body.as_ref().unwrap().scrolls() || h >= 800.0, "{w}x{h}");
        }
    }

    #[test]
    fn help_and_options_do_not_overlap_at_usual_sizes() {
        for (w, h) in [(1280.0, 800.0), (1024.0, 700.0), (900.0, 600.0)] {
            let (mut ui, _) = setup(w, h, Model::default());
            ui.set_visible(WINDOW, true);
            ui.set_visible(crate::help::WINDOW, true);
            let (a, b) = (ui.panels()[1].rect, ui.panels()[3].rect);
            assert!(!a.intersects(&b), "{w}x{h}: {a:?} {b:?}");
        }
        // On a small screen they cannot both fit: opening one closes the other.
        let (mut ui, _) = setup(500.0, 350.0, Model::default());
        ui.set_visible(crate::help::WINDOW, true);
        ui.set_visible(WINDOW, true);
        ui.close_overlapping(WINDOW, &[crate::help::WINDOW]);
        assert!(ui.is_visible(WINDOW) && !ui.is_visible(crate::help::WINDOW));
    }

    #[test]
    fn the_icons_sit_side_by_side() {
        let (ui, _) = setup(1280.0, 800.0, Model::default());
        let (help, options) = (ui.panels()[0].rect, ui.panels()[2].rect);
        assert!(!help.intersects(&options) && options.x < help.x);
    }
}
