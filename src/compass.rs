//! The compass among the icons at the bottom right: a needle whose red end points north on the
//! screen, which turns the view back to north when clicked. The flat map is always north up,
//! so there it is hidden.

use crate::ui::{Panel, PanelId, Rect, Ui, UiEvent, Widget, WidgetId};
use std::cell::Cell;
use std::rc::Rc;

pub const ICON: PanelId = "compass";
const NEEDLE: WidgetId = 1;
/// The needle is drawn again only when the heading changed by this much, in degrees.
const STEP_DEGREES: f64 = 1.0;

/// The heading the needle shows, in whole steps of degrees, or `None` while hidden.
pub type Shared = Rc<Cell<Option<i32>>>;

pub fn icon_panel(state: Shared) -> Panel {
    let mut p = Panel::new(ICON, false, move |p, m| {
        p.rect = m.icon_rect(3);
        let heading = state.get().unwrap_or(0) as f64 * STEP_DEGREES;
        p.fixed = vec![Widget::compass(NEEDLE, Rect::new(0.0, 0.0, p.rect.w, p.rect.h), heading.to_radians())];
        p.visible = state.get().is_some();
    });
    p.in_band = true;
    p
}

/// Shows the camera's heading (radians, clockwise from north), or hides the compass for `None`.
pub fn update(ui: &mut Ui, state: &Shared, heading: Option<f64>) {
    let steps = heading.map(|h| (h.to_degrees() / STEP_DEGREES).round().rem_euclid(360.0 / STEP_DEGREES) as i32);
    if state.get() == steps {
        return;
    }
    let was_shown = state.get().is_some();
    state.set(steps);
    ui.refresh(ICON);
    if was_shown != steps.is_some() {
        ui.set_visible(ICON, steps.is_some());
    }
}

/// True if the event was a click on the compass, which asks for the view to face north.
pub fn handle(event: UiEvent) -> bool {
    matches!(event, UiEvent::Clicked { panel: ICON, widget: NEEDLE })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec2;

    fn ui_with_compass() -> (Ui, Shared) {
        let state = Shared::default();
        let mut ui = Ui::new();
        ui.resize(DVec2::new(800.0, 600.0), 1.0);
        ui.add(icon_panel(state.clone()));
        (ui, state)
    }

    #[test]
    fn shown_on_the_globe_and_hidden_on_the_flat_map() {
        let (mut ui, state) = ui_with_compass();
        assert!(!ui.is_visible(ICON));
        update(&mut ui, &state, Some(0.3));
        assert!(ui.is_visible(ICON));
        assert_eq!(state.get(), Some(17));
        update(&mut ui, &state, None);
        assert!(!ui.is_visible(ICON));
    }

    #[test]
    fn heading_is_kept_in_whole_turns() {
        let (mut ui, state) = ui_with_compass();
        update(&mut ui, &state, Some(-std::f64::consts::FRAC_PI_2));
        assert_eq!(state.get(), Some(270));
        update(&mut ui, &state, Some(2.0 * std::f64::consts::PI + 0.001));
        assert_eq!(state.get(), Some(0));
    }

    #[test]
    fn only_a_click_on_the_needle_counts() {
        assert!(handle(UiEvent::Clicked { panel: ICON, widget: NEEDLE }));
        assert!(!handle(UiEvent::Clicked { panel: "help-icon", widget: NEEDLE }));
    }
}
