//! What lies under a point on the screen: a placed label, or else the ground. Independent of
//! the window system. Double-click flies to it; showing information about it can use the same call.

use crate::vector::{Label, LabelStyle};
use glam::DVec2;

/// A label as it stands on the screen this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLabel {
    /// Centre, in pixels from the top left.
    pub center: DVec2,
    /// Half the width and height of the text, in pixels.
    pub half: DVec2,
    /// How far the text is turned.
    pub angle: f64,
    pub name: String,
    pub style: LabelStyle,
    pub priority: i32,
    pub lat: f64,
    pub lon: f64,
}

impl PlacedLabel {
    pub fn new(label: &Label, center: DVec2, half: DVec2, angle: f64) -> Self {
        Self { center, half, angle, name: label.text.clone(), style: label.style, priority: label.priority, lat: label.lat, lon: label.lon }
    }

    /// Half the size of the text itself, from the size of its raster, which has `pad` pixels
    /// of room for the outline on every side. The hit area follows the text, not the room.
    pub fn text_half(raster: DVec2, pad: f64) -> DVec2 {
        (raster / 2.0 - DVec2::splat(pad)).max(DVec2::ZERO)
    }

    fn contains(&self, p: DVec2) -> bool {
        // A little slack, so small text is not hard to hit.
        const SLACK: f64 = 3.0;
        let d = p - self.center;
        let (s, c) = self.angle.sin_cos();
        // Rotated back into the text's own frame; either way of turning counts when the angle is small.
        let local = DVec2::new(d.x * c + d.y * s, -d.x * s + d.y * c);
        local.x.abs() <= self.half.x + SLACK && local.y.abs() <= self.half.y + SLACK
    }
}

/// The thing a screen position points at.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Label { name: String, style: LabelStyle, priority: i32, lat: f64, lon: f64 },
    Ground { lat: f64, lon: f64 },
}

impl Target {
    pub fn position(&self) -> (f64, f64) {
        match *self {
            Target::Label { lat, lon, .. } | Target::Ground { lat, lon } => (lat, lon),
        }
    }
}

/// A placed label under `pos` wins, the topmost (most important) one if they overlap;
/// otherwise the ground point `ground` found for the same position. `None` over the sky.
pub fn pick(pos: DVec2, labels: &[PlacedLabel], ground: Option<(f64, f64)>) -> Option<Target> {
    let hit = labels.iter().filter(|l| l.contains(pos)).min_by_key(|l| l.priority);
    match hit {
        Some(l) => Some(Target::Label { name: l.name.clone(), style: l.style, priority: l.priority, lat: l.lat, lon: l.lon }),
        None => ground.map(|(lat, lon)| Target::Ground { lat, lon }),
    }
}

/// How close the view comes to a named place, in metres on Earth, by kind. Within a kind the
/// importance (the label's priority, lower is bigger) nudges it: a capital shows more around it.
fn label_distance(style: LabelStyle, priority: i32) -> f64 {
    match style {
        LabelStyle::Country => 4.0e6,
        LabelStyle::State => 1.2e6,
        LabelStyle::Sea => 2.0e6,
        LabelStyle::Lake => 6.0e4,
        // Priorities run from 300 (the biggest cities) upwards.
        LabelStyle::City => 2.5e4 * (1.0 + (330 - priority).clamp(-30, 30) as f64 / 60.0),
        LabelStyle::Town => 1.5e4,
        LabelStyle::Village => 6.0e3,
        LabelStyle::Local => 3.0e3,
        LabelStyle::Peak => 5.0e3,
        LabelStyle::Road => 2.0e4,
    }
}

/// The distance the view should end at. Plain ground comes a level closer (twice as near),
/// or with `farther` a level away. A place gets the distance that suits its kind, but a
/// click never takes the view further out than it is: a view already closer than the fit stays.
pub fn end_distance(target: &Target, current: f64, farther: bool) -> f64 {
    match target {
        Target::Ground { .. } => current * if farther { 2.0 } else { 0.5 },
        Target::Label { style, priority, .. } => {
            let fit = crate::body::scaled(label_distance(*style, *priority));
            if farther {
                current * 2.0
            } else {
                fit.min(current)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(name: &str, priority: i32, x: f64, y: f64) -> PlacedLabel {
        PlacedLabel {
            center: DVec2::new(x, y),
            half: DVec2::new(40.0, 8.0),
            angle: 0.0,
            name: name.into(),
            style: LabelStyle::City,
            priority,
            lat: 0.5,
            lon: 0.25,
        }
    }

    #[test]
    fn the_hit_area_follows_the_text_not_its_padding() {
        // A raster 68 x 28 with 4 px of padding holds text 60 x 20.
        let half = PlacedLabel::text_half(DVec2::new(68.0, 28.0), 4.0);
        assert_eq!(half, DVec2::new(30.0, 10.0));
        let l = PlacedLabel { half, ..label("Nigeria", 100, 631.0, 495.0) };
        assert!(l.contains(DVec2::new(602.0, 495.0)));
        assert!(l.contains(DVec2::new(631.0, 486.0)));
        assert!(!l.contains(DVec2::new(631.0, 495.0 + 14.0)));
        assert!(!l.contains(DVec2::new(631.0 - 36.0, 495.0)));
        assert_eq!(PlacedLabel::text_half(DVec2::new(6.0, 6.0), 4.0), DVec2::ZERO);
    }

    #[test]
    fn a_turned_label_is_hit_along_its_own_axis() {
        let l = PlacedLabel { angle: std::f64::consts::FRAC_PI_2, half: DVec2::new(30.0, 5.0), ..label("Rhine", 100, 100.0, 100.0) };
        assert!(l.contains(DVec2::new(100.0, 125.0)));
        assert!(!l.contains(DVec2::new(125.0, 100.0)));
    }

    #[test]
    fn a_label_wins_over_the_ground() {
        let ls = [label("Bergen", 310, 300.0, 200.0)];
        let t = pick(DVec2::new(330.0, 205.0), &ls, Some((1.0, 1.0))).unwrap();
        assert!(matches!(&t, Target::Label { name, lat, .. } if name == "Bergen" && *lat == 0.5));
        let t = pick(DVec2::new(330.0, 240.0), &ls, Some((1.0, 1.0))).unwrap();
        assert_eq!(t, Target::Ground { lat: 1.0, lon: 1.0 });
    }

    #[test]
    fn empty_space_and_sky_give_nothing() {
        assert_eq!(pick(DVec2::new(5.0, 5.0), &[label("A", 300, 300.0, 200.0)], None), None);
    }

    #[test]
    fn the_more_important_of_two_overlapping_labels_is_taken() {
        let ls = [label("Small", 700, 300.0, 200.0), label("Big", 301, 310.0, 200.0)];
        let Some(Target::Label { name, .. }) = pick(DVec2::new(305.0, 200.0), &ls, None) else { panic!() };
        assert_eq!(name, "Big");
    }

    #[test]
    fn turned_text_is_hit_along_its_own_direction() {
        let mut l = label("Rhine", 800, 300.0, 200.0);
        l.angle = std::f64::consts::FRAC_PI_2;
        // Turned a quarter: its long side now runs up and down.
        assert!(l.contains(DVec2::new(300.0, 230.0)));
        assert!(!l.contains(DVec2::new(330.0, 200.0)));
    }

    #[test]
    fn plain_ground_goes_a_level_closer_or_farther() {
        let g = Target::Ground { lat: 0.0, lon: 0.0 };
        assert_eq!(end_distance(&g, 8.0e4, false), 4.0e4);
        assert_eq!(end_distance(&g, 8.0e4, true), 1.6e5);
    }

    fn named(style: LabelStyle, priority: i32) -> Target {
        Target::Label { name: "x".into(), style, priority, lat: 0.0, lon: 0.0 }
    }

    #[test]
    fn places_end_at_a_distance_that_fits_their_kind() {
        let far = 2.0e7;
        let d = |s, p| end_distance(&named(s, p), far, false);
        assert!(d(LabelStyle::Country, 100) > d(LabelStyle::State, 250));
        assert!(d(LabelStyle::State, 250) > d(LabelStyle::City, 300));
        assert!((1.0e4..=4.0e4).contains(&d(LabelStyle::City, 330)));
        assert!(d(LabelStyle::City, 300) > d(LabelStyle::City, 360));
        assert!(d(LabelStyle::City, 330) > d(LabelStyle::Village, 650));
        assert!(d(LabelStyle::Peak, 620) < 1.0e4);
    }

    #[test]
    fn a_place_never_pushes_the_view_further_out() {
        // Already closer than a country's fit: it is only centred.
        assert_eq!(end_distance(&named(LabelStyle::Country, 100), 3.0e5, false), 3.0e5);
        // Too near for a village: it does not back away to the fit.
        assert!(end_distance(&named(LabelStyle::Village, 650), 1.0e3, false) <= 1.0e3);
        // From far out, it comes in to the fit.
        assert_eq!(end_distance(&named(LabelStyle::Town, 500), 2.0e7, false), 1.5e4);
    }

    #[test]
    fn shift_goes_a_level_out_also_on_a_place() {
        assert_eq!(end_distance(&named(LabelStyle::Town, 500), 1.0e4, true), 2.0e4);
    }
}
