//! Maps SpaceMouse axes to camera motion. Pure, and independent of the frame rate: every
//! result is a rate times `dt`.
//!
//! Axes are in the frame of `spacemouse`: x right, y up, z towards the user; then rotations
//! about those. Signs of the rotations are an assumption that no device has confirmed.

use crate::spacemouse::Axes;

/// Per second at full deflection and sensitivity 1.
const PAN: f64 = 1.5; // fractions of the camera distance
const CLIMB: f64 = 1.2; // e-folds of height
const TURN: f64 = 1.2; // radians
const TILT: f64 = 0.9; // radians
const ATLAS_PAN: f64 = 700.0; // pixels
const ATLAS_ZOOM: f64 = 1.0; // e-folds

/// Motion of the globe's camera for one step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    /// Across the ground, in fractions of the camera distance.
    pub right: f64,
    pub forward: f64,
    /// Natural log of the factor for the distance, and so for the height above ground.
    pub climb: f64,
    /// Added to the heading, clockwise.
    pub turn: f64,
    /// Added to the total tilt, so tilting past the horizon looks up into the sky.
    pub tilt: f64,
}

pub fn globe_step(a: &Axes, sensitivity: f64, dt: f64) -> Step {
    let k = sensitivity * dt;
    Step {
        right: a[0] * PAN * k,
        forward: -a[2] * PAN * k,
        climb: a[1] * CLIMB * k,
        turn: a[4] * TURN * k,
        tilt: -a[3] * TILT * k,
    }
}

/// Motion of the flat map: pan in pixels (as a drag would give) and zoom factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasStep {
    pub dx: f64,
    pub dy: f64,
    pub zoom: f64,
}

pub fn atlas_step(a: &Axes, sensitivity: f64, dt: f64) -> AtlasStep {
    let k = sensitivity * dt;
    AtlasStep {
        dx: -a[0] * ATLAS_PAN * k,
        dy: a[2] * -ATLAS_PAN * k,
        zoom: (-a[1] * ATLAS_ZOOM * k).exp(),
    }
}

pub fn is_at_rest(a: &Axes) -> bool {
    a.iter().all(|v| *v == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_gives_no_motion() {
        assert_eq!(globe_step(&[0.0; 6], 1.0, 0.1), Step::default());
        assert!(is_at_rest(&[0.0; 6]));
        assert!(!is_at_rest(&[0.0, 0.0, 0.0, 0.0, 0.1, 0.0]));
    }

    #[test]
    fn directions() {
        let up = globe_step(&[0.0, 1.0, 0.0, 0.0, 0.0, 0.0], 1.0, 0.1);
        assert!(up.climb > 0.0 && up.right == 0.0 && up.forward == 0.0);
        let away = globe_step(&[0.0, 0.0, -1.0, 0.0, 0.0, 0.0], 1.0, 0.1);
        assert!(away.forward > 0.0);
        let right = globe_step(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 1.0, 0.1);
        assert!(right.right > 0.0);
    }

    #[test]
    fn frame_rate_does_not_matter() {
        let a = [0.4, -0.3, 0.2, 0.5, 0.6, 0.0];
        let whole = globe_step(&a, 1.0, 0.1);
        let part = globe_step(&a, 1.0, 0.025);
        assert!((whole.climb - 4.0 * part.climb).abs() < 1e-12);
        assert!((whole.turn - 4.0 * part.turn).abs() < 1e-12);
        let (w, p) = (atlas_step(&a, 1.0, 0.1), atlas_step(&a, 1.0, 0.025));
        assert!((w.zoom - p.zoom.powi(4)).abs() < 1e-12);
    }

    #[test]
    fn sensitivity_scales() {
        let a = [0.0, 0.5, 0.0, 0.0, 0.0, 0.0];
        assert!(
            (globe_step(&a, 2.0, 0.1).climb - 2.0 * globe_step(&a, 1.0, 0.1).climb).abs() < 1e-12
        );
    }

    #[test]
    fn atlas_uses_pan_and_zoom_only() {
        let s = atlas_step(&[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 1.0, 0.1);
        assert_eq!(
            (s.dx, s.dy, s.zoom),
            (0.0, 0.0, 1.0),
            "tilt, twist and roll do nothing"
        );
        assert!(
            atlas_step(&[0.0, 1.0, 0.0, 0.0, 0.0, 0.0], 1.0, 0.1).zoom < 1.0,
            "up zooms out"
        );
        assert!(
            atlas_step(&[0.0, 0.0, -1.0, 0.0, 0.0, 0.0], 1.0, 0.1).dy > 0.0,
            "pushing away moves the view north"
        );
    }
}
