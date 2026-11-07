//! The path of a smooth flight between two views, after van Wijk and Nuij, "Smooth and
//! efficient zooming and panning", IEEE InfoVis 2003. Independent of the window system.
//!
//! The view is described by its position `u` along the path and its width `w`. The paper
//! gives the path of least "perceived" effort: it rises while the hop is long, so that the
//! distance covered on screen stays moderate, and sinks again towards the end. Speed along
//! the path is eased in and out so the flight starts and ends at rest.

/// The paper's trade-off between zooming and panning; its recommended value, the square root of 2.
const RHO: f64 = std::f64::consts::SQRT_2;
/// The visible width of a globe view as a multiple of the camera's distance, roughly.
pub const WIDTH_PER_DISTANCE: f64 = 1.2;
/// Seconds of flight per unit of path length, plus a base, within the limits.
const SECONDS_PER_UNIT: f64 = 1.1;
const BASE_SECONDS: f64 = 1.0;
pub const MIN_SECONDS: f64 = 1.5;
pub const MAX_SECONDS: f64 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct Curve {
    /// Ground distance to cover, in metres.
    ground: f64,
    w0: f64,
    w1: f64,
    r0: f64,
    /// Total length of the path in the paper's units.
    length: f64,
    /// No panning to speak of: the path is a plain zoom.
    zoom_only: bool,
}

/// Where the flight is at some moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    /// How far along the ground path, from 0 to 1.
    pub along: f64,
    /// The width of the view.
    pub width: f64,
    /// Progress of the whole flight, from 0 to 1, for blending other values.
    pub eased: f64,
}

impl Curve {
    /// A flight covering `ground` of surface from a view of width `w0` to one of width `w1`,
    /// all in the same unit (metres, or units of a map).
    pub fn new(ground: f64, w0: f64, w1: f64) -> Self {
        let (w0, w1) = (w0.max(f64::MIN_POSITIVE), w1.max(f64::MIN_POSITIVE));
        let u = ground.max(0.0);
        // Too short a hop to pan; the formulas divide by it.
        if u < 1e-4 * w0.min(w1) {
            return Self { ground: 0.0, w0, w1, r0: 0.0, length: (w1 / w0).ln().abs() / RHO, zoom_only: true };
        }
        let r2 = RHO * RHO;
        let b = |w: f64, sign: f64| (w1 * w1 - w0 * w0 + sign * r2 * r2 * u * u) / (2.0 * w * r2 * u);
        let r = |b: f64| (-b + (b * b + 1.0).sqrt()).ln();
        let (r0, r1) = (r(b(w0, 1.0)), r(b(w1, -1.0)));
        Self { ground: u, w0, w1, r0, length: (r1 - r0) / RHO, zoom_only: false }
    }

    /// How long the flight takes, longer for a longer path.
    pub fn duration(&self) -> f64 {
        (BASE_SECONDS + SECONDS_PER_UNIT * self.length).clamp(MIN_SECONDS, MAX_SECONDS)
    }

    /// The state a fraction `f` of the way through the flight's time.
    pub fn at(&self, f: f64) -> State {
        let f = f.clamp(0.0, 1.0);
        let eased = f * f * (3.0 - 2.0 * f);
        let s = eased * self.length;
        let (along, w) = if self.zoom_only {
            (0.0, self.w0 * (if self.w1 >= self.w0 { 1.0 } else { -1.0 } * RHO * s).exp())
        } else {
            let r2 = RHO * RHO;
            let a = RHO * s + self.r0;
            let u = self.w0 / r2 * (self.r0.cosh() * a.tanh() - self.r0.sinh());
            (u / self.ground, self.w0 * self.r0.cosh() / a.cosh())
        };
        // The ends are exact whatever rounding did on the way.
        let (along, w) = match f {
            f if f <= 0.0 => (0.0, self.w0),
            f if f >= 1.0 => (if self.zoom_only { 0.0 } else { 1.0 }, self.w1),
            _ => (along.clamp(0.0, 1.0), w),
        };
        State { along, width: w, eased }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(c: &Curve, n: usize) -> Vec<State> {
        (0..=n).map(|i| c.at(i as f64 / n as f64)).collect()
    }

    #[test]
    fn endpoints_are_exact() {
        let c = Curve::new(3.0e6, 2.0e5, 5.0e4);
        let (a, b) = (c.at(0.0), c.at(1.0));
        assert_eq!((a.along, a.width), (0.0, 2.0e5));
        assert_eq!(b.along, 1.0);
        assert!((b.width - 5.0e4).abs() < 1e-6);
    }

    #[test]
    fn progress_is_monotonic_and_distance_continuous() {
        for (g, d0, d1) in [(1.0e7, 1.0e6, 3.0e4), (2.0e3, 1.0e4, 5.0e3), (5.0e5, 2.0e5, 2.0e5)] {
            let c = Curve::new(g, d0, d1);
            let s = samples(&c, 2000);
            for w in s.windows(2) {
                assert!(w[1].along >= w[0].along - 1e-12, "{g}: going backwards");
                // No jumps: the log distance changes by little between close samples.
                assert!((w[1].width / w[0].width).ln().abs() < 0.05, "{g}: distance jumps");
                assert!((w[1].along - w[0].along).abs() < 0.02, "{g}: position jumps");
            }
        }
    }

    #[test]
    fn starts_and_ends_at_rest() {
        let c = Curve::new(8.0e6, 1.0e6, 3.0e4);
        let e = 1e-4;
        let (a0, a1) = (c.at(0.0), c.at(e));
        let (b0, b1) = (c.at(1.0 - e), c.at(1.0));
        let mid = (c.at(0.5 + e).along - c.at(0.5).along) / e;
        assert!((a1.along - a0.along) / e < 0.01 * mid, "starts slowly");
        assert!((b1.along - b0.along) / e < 0.01 * mid, "ends slowly");
        assert!(((a1.width / a0.width).ln() / e).abs() < 0.05);
    }

    #[test]
    fn long_hops_rise_and_short_ones_do_not() {
        let long = Curve::new(1.0e7, 1.0e5, 1.0e5);
        let peak = samples(&long, 200).iter().map(|s| s.width).fold(0.0, f64::max);
        assert!(peak > 3.0 * 1.0e5, "peak {peak}");
        let short = Curve::new(2.0e4, 1.0e5, 1.0e5);
        let peak = samples(&short, 200).iter().map(|s| s.width).fold(0.0, f64::max);
        assert!(peak < 1.5 * 1.0e5, "peak {peak}");
    }

    #[test]
    fn pure_zoom_goes_straight_in_or_out() {
        for (d0, d1) in [(1.0e5, 5.0e4), (5.0e4, 1.0e5)] {
            let c = Curve::new(0.0, d0, d1);
            let s = samples(&c, 100);
            assert!(s.iter().all(|s| s.along == 0.0));
            assert!(s.windows(2).all(|w| (w[1].width - w[0].width).signum() == (d1 - d0).signum() || w[1].width == w[0].width));
            assert!((s[100].width - d1).abs() < 1e-6);
        }
    }

    #[test]
    fn duration_is_clamped_and_grows_with_the_path() {
        let tiny = Curve::new(10.0, 1.0e3, 1.0e3);
        assert_eq!(tiny.duration(), MIN_SECONDS);
        let huge = Curve::new(2.0e7, 30.0, 30.0);
        assert_eq!(huge.duration(), MAX_SECONDS);
        let (a, b) = (Curve::new(1.0e5, 1.0e5, 1.0e5), Curve::new(3.0e6, 1.0e5, 1.0e5));
        assert!(b.duration() > a.duration());
    }
}
