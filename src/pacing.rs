//! Keeps drawing to a chosen rate, and measures the rate actually reached.

use std::time::{Duration, Instant};

/// A pause longer than this means drawing had stopped, rather than a slow frame.
const IDLE_GAP: Duration = Duration::from_millis(250);
/// How often the measured rate is updated.
const METER_PERIOD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameLimit {
    Fps120,
    Fps90,
}

impl FrameLimit {
    pub fn hz(self) -> u32 {
        match self {
            Self::Fps120 => 120,
            Self::Fps90 => 90,
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Fps120 => Self::Fps90,
            Self::Fps90 => Self::Fps120,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "120" => Some(Self::Fps120),
            "90" => Some(Self::Fps90),
            _ => None,
        }
    }

    fn interval(self) -> Duration {
        Duration::from_secs_f64(1.0 / self.hz() as f64)
    }
}

/// Decides when the next frame may start.
pub struct Pacer {
    limit: FrameLimit,
    next: Instant,
}

impl Pacer {
    pub fn new(limit: FrameLimit, now: Instant) -> Self {
        Self { limit, next: now }
    }

    pub fn limit(&self) -> FrameLimit {
        self.limit
    }

    pub fn toggle(&mut self) {
        self.limit = self.limit.other();
    }

    pub fn set_limit(&mut self, limit: FrameLimit) {
        self.limit = limit;
    }

    pub fn due(&self, now: Instant) -> bool {
        now >= self.next
    }

    pub fn next_frame_at(&self) -> Instant {
        self.next
    }

    /// Notes that a frame started. Frames are scheduled from the previous slot rather than
    /// from now, so that small wake-up delays even out instead of lowering the rate. After a
    /// longer delay it starts afresh, rather than rushing to make up for lost frames.
    pub fn frame_started(&mut self, now: Instant) {
        let interval = self.limit.interval();
        let scheduled = self.next + interval;
        self.next = if scheduled > now { scheduled } else { now + interval };
    }
}

/// Measures frames per second while drawing is going on.
#[derive(Default)]
pub struct RateMeter {
    last: Option<Instant>,
    window_start: Option<Instant>,
    frames: u32,
    rate: Option<f64>,
}

impl RateMeter {
    pub fn record(&mut self, now: Instant) {
        if self.last.is_none_or(|l| now - l > IDLE_GAP) {
            self.window_start = Some(now);
            self.frames = 0;
            self.rate = None;
        }
        self.last = Some(now);
        self.frames += 1;
        if let Some(start) = self.window_start {
            let elapsed = now - start;
            if elapsed >= METER_PERIOD {
                self.rate = Some((self.frames - 1) as f64 / elapsed.as_secs_f64());
                self.window_start = Some(now);
                self.frames = 1;
            }
        }
    }

    /// The latest measurement, if drawing is under way.
    pub fn rate(&self) -> Option<f64> {
        self.rate
    }

    /// True when a shown rate is out of date because drawing has stopped.
    pub fn is_stale(&self, now: Instant) -> bool {
        self.rate.is_some() && self.last.is_some_and(|l| now - l > IDLE_GAP)
    }
}

#[derive(Debug, PartialEq)]
pub struct FrameStats {
    pub frames: usize,
    pub average_fps: f64,
    pub median_ms: f64,
    pub p99_ms: f64,
    pub worst_ms: f64,
}

/// Summarises the times between frames, in seconds.
pub fn summarize(intervals: &[f64]) -> Option<FrameStats> {
    if intervals.is_empty() {
        return None;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize] * 1000.0;
    Some(FrameStats {
        frames: sorted.len(),
        average_fps: sorted.len() as f64 / sorted.iter().sum::<f64>(),
        median_ms: at(0.5),
        p99_ms: at(0.99),
        worst_ms: at(1.0),
    })
}

/// Mailbox shows the newest finished frame at each refresh, so drawing is not tied to the
/// display's rate and no tearing appears. Fifo, which every system has, is the fallback.
pub fn present_mode(available: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if available.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    }
}

/// The line for the on-screen display.
pub fn status_line(limit: FrameLimit, rate: Option<f64>, display_hz: Option<f64>) -> String {
    let now = rate.map_or("idle".to_string(), |r| format!("{r:.0} fps"));
    match display_hz {
        Some(hz) => format!("limit {} fps  now {now}  display {hz:.0} Hz", limit.hz()),
        None => format!("limit {} fps  now {now}", limit.hz()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: f64) -> Duration {
        Duration::from_secs_f64(n / 1000.0)
    }

    /// Runs a pacer against a clock that wakes up with the given delay after each slot, and
    /// returns the resulting frame rate.
    fn simulate(limit: FrameLimit, wake_delay: Duration, seconds: f64) -> f64 {
        let start = Instant::now();
        let mut pacer = Pacer::new(limit, start);
        let mut frames = 0u32;
        let mut now = start;
        let end = start + Duration::from_secs_f64(seconds);
        while now < end {
            now = now.max(pacer.next_frame_at()) + wake_delay;
            assert!(pacer.due(now));
            pacer.frame_started(now);
            frames += 1;
        }
        // The frames span the time between the first and the last of them.
        (frames - 1) as f64 / seconds
    }

    #[test]
    fn both_limits_hold_their_rate() {
        for (limit, hz) in [(FrameLimit::Fps120, 120.0), (FrameLimit::Fps90, 90.0)] {
            let rate = simulate(limit, Duration::ZERO, 2.0);
            assert!((rate - hz).abs() < 1.0, "{rate}");
        }
    }

    #[test]
    fn small_wake_delays_do_not_lower_the_rate() {
        let rate = simulate(FrameLimit::Fps120, ms(1.0), 2.0);
        assert!((rate - 120.0).abs() < 1.0, "{rate}");
    }

    #[test]
    fn nothing_starts_before_its_slot() {
        let start = Instant::now();
        let mut pacer = Pacer::new(FrameLimit::Fps90, start);
        assert!(pacer.due(start));
        pacer.frame_started(start);
        assert!(!pacer.due(start + ms(5.0)));
        assert!(pacer.due(start + ms(11.2)));
    }

    #[test]
    fn a_long_stall_does_not_cause_a_burst() {
        let start = Instant::now();
        let mut pacer = Pacer::new(FrameLimit::Fps120, start);
        pacer.frame_started(start);
        let later = start + Duration::from_secs(2);
        pacer.frame_started(later);
        assert!(!pacer.due(later + ms(1.0)), "must wait a full interval again");
    }

    #[test]
    fn toggling_switches_between_the_two_limits() {
        let mut pacer = Pacer::new(FrameLimit::Fps120, Instant::now());
        pacer.toggle();
        assert_eq!(pacer.limit(), FrameLimit::Fps90);
        pacer.toggle();
        assert_eq!(pacer.limit(), FrameLimit::Fps120);
    }

    #[test]
    fn limits_parse() {
        assert_eq!(FrameLimit::parse("120"), Some(FrameLimit::Fps120));
        assert_eq!(FrameLimit::parse("90"), Some(FrameLimit::Fps90));
        assert_eq!(FrameLimit::parse("60"), None);
    }

    #[test]
    fn meter_measures_and_goes_idle() {
        let start = Instant::now();
        let mut meter = RateMeter::default();
        assert_eq!(meter.rate(), None);
        let mut now = start;
        for _ in 0..120 {
            meter.record(now);
            now += ms(10.0);
        }
        let rate = meter.rate().unwrap();
        assert!((rate - 100.0).abs() < 1.0, "{rate}");
        assert!(!meter.is_stale(now));
        assert!(meter.is_stale(now + ms(300.0)));
        // Drawing resumes: the pause does not count as a slow frame.
        meter.record(now + Duration::from_secs(5));
        assert_eq!(meter.rate(), None);
        assert!(!meter.is_stale(now + Duration::from_secs(6)));
    }

    #[test]
    fn stats_summarise_frame_times() {
        assert_eq!(summarize(&[]), None);
        let mut times = vec![0.01; 99];
        times.push(0.05);
        let s = summarize(&times).unwrap();
        assert_eq!(s.frames, 100);
        assert!((s.median_ms - 10.0).abs() < 1e-9);
        assert!((s.worst_ms - 50.0).abs() < 1e-9);
        assert!((s.average_fps - 100.0 / 1.04).abs() < 1e-6);
    }

    #[test]
    fn mailbox_is_preferred_with_fifo_as_fallback() {
        use wgpu::PresentMode::{Fifo, Immediate, Mailbox};
        assert_eq!(present_mode(&[Fifo, Immediate, Mailbox]), Mailbox);
        assert_eq!(present_mode(&[Fifo, Immediate]), Fifo);
    }

    #[test]
    fn status_shows_limit_rate_and_display() {
        let line = status_line(FrameLimit::Fps90, Some(89.6), Some(143.9));
        assert_eq!(line, "limit 90 fps  now 90 fps  display 144 Hz");
        assert_eq!(status_line(FrameLimit::Fps120, None, None), "limit 120 fps  now idle");
    }
}
