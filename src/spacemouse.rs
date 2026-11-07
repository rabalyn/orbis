//! Six-axis input from a 3Dconnexion SpaceMouse, through `spacenavd` or the raw HID device.
//!
//! A background thread reads the device and keeps only the latest shaped axes and the button
//! presses. The window is woken only when those change, so an idle device costs nothing.
//! Nothing here touches the window system, and nothing here has been tried on real hardware.
//!
//! Sources of the formats: the libspnav / spacenavd protocol (`spnav.h`, `libspnav.c`: the
//! AF_UNIX socket at /var/run/spnav.sock delivers records of 8 native-endian `int`s), and the
//! USB HID reports of 3Dconnexion devices as described by the Linux `hid-sensor`-less
//! raw layout used by open drivers such as spacenavd and `hidapi` examples (report 1 is
//! translation, 2 rotation, 3 buttons, little-endian i16).

use std::fs::File;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SOCKET: &str = "/var/run/spnav.sock";
/// How long to wait before looking for the device again.
pub const RETRY: Duration = Duration::from_secs(5);
/// The usual full-scale reading of the device's axes.
const FULL_SCALE: f64 = 350.0;
/// Part of full deflection that is ignored, so a resting cap does not drift the view.
const DEAD_ZONE: f64 = 0.06;
const CURVE_POWER: f64 = 1.7;

/// Translation x (right), y (up), z (towards the user), then rotation about the same axes.
pub type Axes = [f64; 6];

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    WholeBody,
    Straighten,
}

pub fn action_for_button(n: u32) -> Option<Action> {
    match n {
        0 => Some(Action::WholeBody),
        1 => Some(Action::Straighten),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpnavEvent {
    Motion { axes: [i32; 6], period: i32 },
    Button { n: u32, press: bool },
}

/// Size of one record on the socket.
pub const SPNAV_RECORD: usize = 32;

/// Reads one record: type 0 motion (x y z rx ry rz period), 1 press, 2 release (button number).
pub fn parse_spnav(rec: &[u8]) -> Option<SpnavEvent> {
    if rec.len() < SPNAV_RECORD {
        return None;
    }
    let v: Vec<i32> = rec[..SPNAV_RECORD]
        .chunks_exact(4)
        .map(|c| i32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    match v[0] {
        0 => Some(SpnavEvent::Motion { axes: [v[1], v[2], v[3], v[4], v[5], v[6]], period: v[7] }),
        1 | 2 => Some(SpnavEvent::Button { n: u32::try_from(v[1]).ok()?, press: v[0] == 1 }),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HidReport {
    Translation([i16; 3]),
    Rotation([i16; 3]),
    Combined([i16; 6]),
    Buttons(u32),
}

fn i16s<const N: usize>(b: &[u8]) -> [i16; N] {
    std::array::from_fn(|i| i16::from_le_bytes([b[2 * i], b[2 * i + 1]]))
}

/// Parses a raw HID input report, whose first byte is the report number.
pub fn parse_hid(buf: &[u8]) -> Option<HidReport> {
    let (&id, body) = buf.split_first()?;
    match id {
        1 if body.len() >= 12 => Some(HidReport::Combined(i16s(body))),
        1 if body.len() >= 6 => Some(HidReport::Translation(i16s(body))),
        2 if body.len() >= 6 => Some(HidReport::Rotation(i16s(body))),
        3 if !body.is_empty() => {
            let mut mask = 0u32;
            for (i, b) in body.iter().take(4).enumerate() {
                mask |= (*b as u32) << (8 * i);
            }
            Some(HidReport::Buttons(mask))
        }
        _ => None,
    }
}

/// Dead zone and sensitivity curve for one axis, from -1 to 1 in and out. Small deflections
/// give fine movement and a full push gives full speed.
pub fn shape(v: f64) -> f64 {
    let a = v.abs().min(1.0);
    if a <= DEAD_ZONE || !v.is_finite() {
        return 0.0;
    }
    let t = (a - DEAD_ZONE) / (1.0 - DEAD_ZONE);
    t.powf(CURVE_POWER).copysign(v)
}

pub fn shape_axes(raw: [f64; 6]) -> Axes {
    raw.map(|r| shape(r / FULL_SCALE))
}

/// Brings HID axes into the frame of spacenavd (y up, z towards the user): the device reports
/// y towards the user and z downwards. Assumed from the open drivers; not tried on a device.
pub fn hid_to_frame(t: [f64; 3], r: [f64; 3]) -> [f64; 6] {
    [t[0], -t[2], t[1], r[0], -r[2], r[1]]
}

/// Whether a `HID_ID` line from sysfs names a 3Dconnexion device (0003:VVVVVVVV:PPPPPPPP).
pub fn is_3dconnexion(hid_id: &str) -> bool {
    let mut it = hid_id.trim().split(':');
    let (Some(bus), Some(v), Some(p)) = (it.next(), it.next(), it.next()) else { return false };
    let (Ok(bus), Ok(v), Ok(p)) =
        (u32::from_str_radix(bus, 16), u32::from_str_radix(v, 16), u32::from_str_radix(p, 16))
    else {
        return false;
    };
    bus == 3 && (v == 0x256f || (v == 0x046d && (0xc603..=0xc63f).contains(&p)))
}

/// The /dev/hidrawN files of 3Dconnexion devices, found through sysfs.
pub fn find_hidraw(sys: &Path, dev: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(sys) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let uevent = std::fs::read_to_string(e.path().join("device/uevent")).ok()?;
            let id = uevent.lines().find_map(|l| l.strip_prefix("HID_ID="))?;
            is_3dconnexion(id).then(|| dev.join(e.file_name()))
        })
        .collect();
    found.sort();
    found
}

#[derive(Default)]
struct Inner {
    axes: Axes,
    actions: Vec<Action>,
    notice: Option<String>,
}

/// What the window takes from the reader on a wake.
#[derive(Default, Debug, PartialEq)]
pub struct Update {
    pub axes: Axes,
    pub actions: Vec<Action>,
    pub notice: Option<String>,
}

#[derive(Default)]
pub struct Shared {
    inner: Mutex<Inner>,
    pending: AtomicBool,
}

impl Shared {
    pub fn drain(&self) -> Update {
        self.pending.store(false, Ordering::SeqCst);
        let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Update { axes: i.axes, actions: std::mem::take(&mut i.actions), notice: i.notice.take() }
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Turns decoded device input into shared state, waking the window only on a change.
pub struct Sink {
    shared: Arc<Shared>,
    wake: Wake,
    raw: [f64; 6],
    /// HID reports come in halves, kept in the device's own order.
    hid_t: [f64; 3],
    hid_r: [f64; 3],
    buttons: u32,
}

impl Sink {
    pub fn new(shared: Arc<Shared>, wake: Wake) -> Self {
        Self { shared, wake, raw: [0.0; 6], hid_t: [0.0; 3], hid_r: [0.0; 3], buttons: 0 }
    }

    fn publish(&self, f: impl FnOnce(&mut Inner) -> bool) {
        let changed = f(&mut self.shared.inner.lock().unwrap_or_else(|e| e.into_inner()));
        if changed && !self.shared.pending.swap(true, Ordering::SeqCst) {
            (self.wake)();
        }
    }

    fn set_axes(&mut self) {
        let axes = shape_axes(self.raw);
        self.publish(|i| {
            let changed = i.axes != axes;
            i.axes = axes;
            changed
        });
    }

    fn press(&mut self, n: u32) {
        if let Some(a) = action_for_button(n) {
            self.publish(|i| {
                i.actions.push(a);
                true
            });
        }
    }

    pub fn spnav(&mut self, e: SpnavEvent) {
        match e {
            SpnavEvent::Motion { axes, .. } => {
                self.raw = axes.map(|a| a as f64);
                self.set_axes();
            }
            SpnavEvent::Button { n, press } => {
                if press {
                    self.press(n);
                }
            }
        }
    }

    pub fn hid(&mut self, r: HidReport) {
        let f = |a: [i16; 3]| a.map(|x| x as f64);
        match r {
            HidReport::Translation(a) => self.hid_t = f(a),
            HidReport::Rotation(a) => self.hid_r = f(a),
            HidReport::Combined(a) => {
                self.hid_t = f([a[0], a[1], a[2]]);
                self.hid_r = f([a[3], a[4], a[5]]);
            }
            HidReport::Buttons(mask) => {
                for n in 0..32 {
                    if mask & (1 << n) != 0 && self.buttons & (1 << n) == 0 {
                        self.press(n);
                    }
                }
                self.buttons = mask;
                return;
            }
        }
        self.raw = hid_to_frame(self.hid_t, self.hid_r);
        self.set_axes();
    }

    fn connected(&mut self, what: &str) {
        self.raw = [0.0; 6];
        self.hid_t = [0.0; 3];
        self.hid_r = [0.0; 3];
        self.buttons = 0;
        let notice = format!("SpaceMouse connected ({what}).");
        self.publish(|i| {
            i.notice = Some(notice);
            true
        });
    }

    /// The device went away: the view must not keep moving on its last reading.
    fn disconnected(&mut self) {
        self.raw = [0.0; 6];
        self.hid_t = [0.0; 3];
        self.hid_r = [0.0; 3];
        self.set_axes();
    }
}

/// Reads records until the stream ends or fails. A short final record is dropped.
pub fn read_spnav<R: Read>(r: &mut R, sink: &mut Sink) -> io::Result<()> {
    let mut rec = [0u8; SPNAV_RECORD];
    loop {
        match r.read_exact(&mut rec) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        }
        if let Some(e) = parse_spnav(&rec) {
            sink.spnav(e);
        }
    }
}

/// Reads HID reports until the device ends or fails.
pub fn read_hid<R: Read>(r: &mut R, sink: &mut Sink) -> io::Result<()> {
    let mut buf = [0u8; 64];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        if let Some(rep) = parse_hid(&buf[..n]) {
            sink.hid(rep);
        }
    }
}

pub struct Config {
    pub socket: PathBuf,
    pub sys_hidraw: PathBuf,
    pub dev: PathBuf,
    pub retry: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket: SOCKET.into(),
            sys_hidraw: "/sys/class/hidraw".into(),
            dev: "/dev".into(),
            retry: RETRY,
        }
    }
}

/// Keeps the reader thread alive; dropping it asks the thread to stop at its next pause.
pub struct Handle {
    pub shared: Arc<Shared>,
    _stop: Sender<()>,
}

pub fn spawn(cfg: Config, wake: Wake) -> Handle {
    let shared = Arc::new(Shared::default());
    let (stop, rx) = mpsc::channel::<()>();
    let mut sink = Sink::new(shared.clone(), wake);
    let spawned = std::thread::Builder::new().name("spacemouse".into()).spawn(move || loop {
        connect_once(&cfg, &mut sink);
        // Waiting here is the only idle activity. Dropping the handle ends the wait.
        if !matches!(rx.recv_timeout(cfg.retry), Err(RecvTimeoutError::Timeout)) {
            return;
        }
    });
    // Without a thread there is simply no SpaceMouse.
    drop(spawned);
    Handle { shared, _stop: stop }
}

/// One try at each source. Every failure is quiet: no device is the usual case.
fn connect_once(cfg: &Config, sink: &mut Sink) {
    if let Ok(mut s) = UnixStream::connect(&cfg.socket) {
        sink.connected("spacenavd");
        let _ = read_spnav(&mut s, sink);
        sink.disconnected();
        return;
    }
    for path in find_hidraw(&cfg.sys_hidraw, &cfg.dev) {
        // Permission to read the device is often missing; the next one is tried quietly.
        if let Ok(mut f) = File::open(&path) {
            sink.connected("USB");
            let _ = read_hid(&mut f, sink);
            sink.disconnected();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::AtomicUsize;

    fn rec(v: [i32; 8]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_ne_bytes()).collect()
    }

    fn sink() -> (Sink, Arc<Shared>, Arc<AtomicUsize>) {
        let shared = Arc::new(Shared::default());
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let wake: Wake = Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        (Sink::new(shared.clone(), wake), shared, wakes)
    }

    #[test]
    fn spnav_records_parse() {
        let m = parse_spnav(&rec([0, 10, -20, 30, 1, 2, 3, 16])).unwrap();
        assert_eq!(m, SpnavEvent::Motion { axes: [10, -20, 30, 1, 2, 3], period: 16 });
        assert_eq!(parse_spnav(&rec([1, 1, 0, 0, 0, 0, 0, 0])), Some(SpnavEvent::Button { n: 1, press: true }));
        assert_eq!(parse_spnav(&rec([2, 0, 0, 0, 0, 0, 0, 0])), Some(SpnavEvent::Button { n: 0, press: false }));
    }

    #[test]
    fn spnav_junk_is_refused() {
        assert_eq!(parse_spnav(&[]), None);
        assert_eq!(parse_spnav(&rec([0, 1, 2, 3, 4, 5, 6, 7])[..31]), None);
        assert_eq!(parse_spnav(&rec([9, 0, 0, 0, 0, 0, 0, 0])), None);
        assert_eq!(parse_spnav(&rec([1, -1, 0, 0, 0, 0, 0, 0])), None);
    }

    #[test]
    fn hid_reports_parse() {
        let t = [1u8, 0x0a, 0x00, 0xf6, 0xff, 0x00, 0x01];
        assert_eq!(parse_hid(&t), Some(HidReport::Translation([10, -10, 256])));
        let r = [2u8, 1, 0, 2, 0, 3, 0];
        assert_eq!(parse_hid(&r), Some(HidReport::Rotation([1, 2, 3])));
        let mut c = vec![1u8];
        c.extend((1..=6i16).flat_map(|x| x.to_le_bytes()));
        assert_eq!(parse_hid(&c), Some(HidReport::Combined([1, 2, 3, 4, 5, 6])));
        assert_eq!(parse_hid(&[3, 0b11]), Some(HidReport::Buttons(3)));
        assert_eq!(parse_hid(&[3, 1, 1]), Some(HidReport::Buttons(0x101)));
    }

    #[test]
    fn hid_junk_is_refused() {
        assert_eq!(parse_hid(&[]), None);
        assert_eq!(parse_hid(&[1, 0, 0, 0]), None);
        assert_eq!(parse_hid(&[2]), None);
        assert_eq!(parse_hid(&[3]), None);
        assert_eq!(parse_hid(&[7, 1, 2, 3, 4, 5, 6]), None);
    }

    #[test]
    fn shaping_has_a_dead_zone_and_a_curve() {
        assert_eq!(shape(0.0), 0.0);
        assert_eq!(shape(DEAD_ZONE * 0.9), 0.0);
        assert_eq!(shape(-DEAD_ZONE * 0.9), 0.0);
        assert!((shape(1.0) - 1.0).abs() < 1e-12);
        assert!((shape(-3.0) + 1.0).abs() < 1e-12, "beyond full scale is clamped");
        assert_eq!(shape(f64::NAN), 0.0);
        assert!(shape(0.5) < 0.5, "small pushes are finer than linear");
        let mut last = 0.0;
        for i in 0..=100 {
            let s = shape(i as f64 / 100.0);
            assert!(s >= last);
            last = s;
        }
        assert_eq!(shape(0.3), -shape(-0.3));
    }

    #[test]
    fn hardware_ids() {
        assert!(is_3dconnexion("0003:0000046D:0000C626"));
        assert!(is_3dconnexion("0003:0000256F:0000C635\n"));
        assert!(is_3dconnexion("0003:0000256F:00000001"));
        assert!(!is_3dconnexion("0003:0000046D:0000C52B"), "a Logitech mouse receiver");
        assert!(!is_3dconnexion("0005:0000046D:0000C626"), "not USB");
        assert!(!is_3dconnexion("nonsense"));
        assert!(!is_3dconnexion(""));
    }

    #[test]
    fn sink_wakes_only_for_changes() {
        let (mut s, shared, wakes) = sink();
        let still = SpnavEvent::Motion { axes: [0; 6], period: 1 };
        for _ in 0..50 {
            s.spnav(still);
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 0, "an idle stream wakes nothing");
        s.spnav(SpnavEvent::Motion { axes: [200, 0, 0, 0, 0, 0], period: 1 });
        s.spnav(SpnavEvent::Motion { axes: [210, 0, 0, 0, 0, 0], period: 1 });
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "one wake until the window takes the state");
        let u = shared.drain();
        assert!(u.axes[0] > 0.0);
        s.spnav(SpnavEvent::Motion { axes: [210, 0, 0, 0, 0, 0], period: 1 });
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "same reading again changes nothing");
        s.spnav(still);
        assert_eq!(wakes.load(Ordering::SeqCst), 2, "coming to rest is a change");
        assert_eq!(shared.drain().axes, [0.0; 6]);
    }

    #[test]
    fn buttons_give_actions_on_press_only() {
        let (mut s, shared, _) = sink();
        s.spnav(SpnavEvent::Button { n: 0, press: true });
        s.spnav(SpnavEvent::Button { n: 0, press: false });
        s.spnav(SpnavEvent::Button { n: 1, press: true });
        s.spnav(SpnavEvent::Button { n: 7, press: true });
        assert_eq!(shared.drain().actions, vec![Action::WholeBody, Action::Straighten]);
        s.hid(HidReport::Buttons(0b10));
        s.hid(HidReport::Buttons(0b10));
        s.hid(HidReport::Buttons(0));
        s.hid(HidReport::Buttons(0b11));
        assert_eq!(shared.drain().actions, vec![Action::Straighten, Action::WholeBody, Action::Straighten]);
    }

    #[test]
    fn hid_halves_combine() {
        let (mut s, shared, _) = sink();
        s.hid(HidReport::Translation([350, 0, 0]));
        s.hid(HidReport::Rotation([0, 0, 350]));
        let a = shared.drain().axes;
        assert!((a[0] - 1.0).abs() < 1e-9, "x kept after the rotation report");
        assert!((a[4] + 1.0).abs() < 1e-9, "twist taken from the rotation report");
    }

    #[test]
    fn short_final_record_is_dropped() {
        let (mut s, shared, _) = sink();
        let mut bytes = rec([0, 350, 0, 0, 0, 0, 0, 1]);
        bytes.extend([1, 2, 3]);
        read_spnav(&mut &bytes[..], &mut s).unwrap();
        assert!(shared.drain().axes[0] > 0.9);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("orbis-sm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn nothing_found_is_quiet() {
        let d = temp_dir("none");
        let cfg = Config { socket: d.join("no.sock"), sys_hidraw: d.join("sys"), dev: d.clone(), retry: Duration::from_millis(5) };
        let (mut s, shared, wakes) = sink();
        connect_once(&cfg, &mut s);
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
        assert_eq!(shared.drain(), Update::default());
        // A matching device that cannot be opened is skipped without a sound.
        let node = d.join("sys/hidraw0/device");
        std::fs::create_dir_all(&node).unwrap();
        std::fs::write(node.join("uevent"), "DRIVER=hid\nHID_ID=0003:0000256F:0000C635\n").unwrap();
        assert_eq!(find_hidraw(&d.join("sys"), &d), vec![d.join("hidraw0")]);
        connect_once(&cfg, &mut s);
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        for _ in 0..400 {
            if f() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn socket_bytes_reach_the_camera_and_a_closed_socket_is_retried() {
        use crate::camera::Camera;
        use crate::nav::Nav;
        use glam::DVec2;

        let d = temp_dir("sock");
        let path = d.join("spnav.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let cfg = Config { socket: path, sys_hidraw: d.join("sys"), dev: d.clone(), retry: Duration::from_millis(20) };
        let handle = spawn(cfg, Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }));

        let mut cam = Camera { lat: 0.8, lon: 0.2, target_h: 0.0, distance: 100_000.0, heading: 0.0, tilt: 0.0, sky: 0.0 };
        cam.clamp();
        let mut nav = Nav::new(cam, DVec2::new(1200.0, 800.0));

        // First connection: push the cap up, then close the socket with the cap still pushed.
        let (mut c, _) = listener.accept().unwrap();
        c.write_all(&rec([0, 0, 350, 0, 0, 0, 0, 8])).unwrap();
        let mut notice = String::new();
        let mut axes = [0.0; 6];
        wait_for("the motion", || {
            let u = handle.shared.drain();
            notice += &u.notice.unwrap_or_default();
            axes = u.axes;
            axes[1] > 0.9
        });
        assert!(notice.contains("connected"));
        nav.set_device(axes, 1.0);
        let d0 = nav.cam.distance;
        nav.tick(1.0 / 60.0, &|_, _| None);
        assert!(nav.cam.distance > d0, "pushing up climbs");
        drop(c);

        // The closed socket stops the motion and the reader goes on to ask again.
        wait_for("rest after close", || {
            let u = handle.shared.drain();
            u.axes == [0.0; 6]
        });
        let (mut c2, _) = listener.accept().unwrap();
        c2.write_all(&rec([1, 0, 0, 0, 0, 0, 0, 0])).unwrap();
        wait_for("a button on the second connection", || {
            handle.shared.drain().actions == vec![Action::WholeBody]
        });
        drop(c2);
        drop(handle);
        let _ = std::fs::remove_dir_all(&d);
    }
}
