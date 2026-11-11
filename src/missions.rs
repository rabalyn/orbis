//! The tracks of the rovers on Mars, of Ingenuity and of the Apollo moonwalkers
//! (`data/missions.bin`, made by `tools/gen_missions.py`), and where each was at a time.
//!
//! Each track is drawn on the surface, the part already driven bright and the rest faint, with
//! a small picture of the rover, the helicopter or the astronaut where it was at the clock's
//! time. Letting the clock run moves them along.

use crate::search::Place;
use glam::DVec2;
use std::sync::OnceLock;

const DATA: &[u8] = include_bytes!("../data/missions.bin");
/// The length of a Martian sol, in seconds.
const SOL: f64 = 88_775.244;
const MAX_RESULTS: usize = 8;
/// Track points closer together on screen than this many pixels are drawn as one.
const MIN_STEP: f64 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Rover,
    Helicopter,
    Astronaut,
    /// A spacecraft that stayed where it landed.
    Lander,
}

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    /// Unix time in seconds.
    pub t: f64,
    /// Planetocentric, in radians.
    pub lat: f64,
    pub lon: f64,
    /// The distance travelled from the start of the track, in metres.
    pub odo: f64,
}

pub struct Track {
    pub body: String,
    pub name: String,
    pub kind: Kind,
    pub colour: [u8; 3],
    /// The rover stays where it ended, after its last drive.
    stays: bool,
    /// When sol 0 began at the landing site, for counting sols; NaN on the Moon.
    sol0: f64,
    pub pieces: Vec<Vec<Vertex>>,
    /// Where each piece starts in a flat numbering of all vertices.
    starts: Vec<usize>,
}

/// Where a track's traveller was at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub lat: f64,
    pub lon: f64,
    pub odo: f64,
    pub piece: usize,
    /// The vertex at or before the spot; the spot lies between it and the next, at `frac`.
    pub vertex: usize,
    pub frac: f64,
}

/// Landings whose path on the surface is not in the data, as (body, name, kind, latitude,
/// east longitude in degrees, landing time as year, month, day, hour, minute UTC). Rovers among
/// them are shown where they landed.
const SITES: &[(&str, &str, Kind, f64, f64, [i64; 5])] = &[
    (
        "moon",
        "Luna 9",
        Kind::Lander,
        7.08,
        -64.37,
        [1966, 2, 3, 18, 45],
    ),
    (
        "moon",
        "Surveyor 1",
        Kind::Lander,
        -2.474,
        -43.339,
        [1966, 6, 2, 6, 17],
    ),
    (
        "moon",
        "Surveyor 3",
        Kind::Lander,
        -3.0162,
        -23.4182,
        [1967, 4, 20, 0, 4],
    ),
    (
        "moon",
        "Surveyor 5",
        Kind::Lander,
        1.455,
        23.195,
        [1967, 9, 11, 0, 46],
    ),
    (
        "moon",
        "Surveyor 6",
        Kind::Lander,
        0.49,
        -1.40,
        [1967, 11, 10, 1, 1],
    ),
    (
        "moon",
        "Surveyor 7",
        Kind::Lander,
        -40.98,
        -11.51,
        [1968, 1, 10, 1, 5],
    ),
    (
        "moon",
        "Apollo 11",
        Kind::Lander,
        0.67408,
        23.47297,
        [1969, 7, 20, 20, 17],
    ),
    (
        "moon",
        "Apollo 12",
        Kind::Lander,
        -3.01239,
        -23.42157,
        [1969, 11, 19, 6, 54],
    ),
    (
        "moon",
        "Luna 16",
        Kind::Lander,
        -0.513,
        56.364,
        [1970, 9, 20, 5, 18],
    ),
    (
        "moon",
        "Lunokhod 1",
        Kind::Rover,
        38.2378,
        -35.0017,
        [1970, 11, 17, 3, 47],
    ),
    (
        "moon",
        "Apollo 14",
        Kind::Lander,
        -3.64530,
        -17.47136,
        [1971, 2, 5, 9, 18],
    ),
    (
        "moon",
        "Apollo 15",
        Kind::Lander,
        26.13222,
        3.63386,
        [1971, 7, 30, 22, 16],
    ),
    (
        "moon",
        "Apollo 16",
        Kind::Lander,
        -8.97301,
        15.50019,
        [1972, 4, 21, 2, 23],
    ),
    (
        "moon",
        "Apollo 17",
        Kind::Lander,
        20.19080,
        30.77168,
        [1972, 12, 11, 19, 54],
    ),
    (
        "moon",
        "Lunokhod 2",
        Kind::Rover,
        25.85,
        30.45,
        [1973, 1, 15, 22, 35],
    ),
    (
        "moon",
        "Luna 24",
        Kind::Lander,
        12.7145,
        62.2129,
        [1976, 8, 18, 6, 36],
    ),
    (
        "moon",
        "Chang'e 3 and Yutu",
        Kind::Rover,
        44.1214,
        -19.5116,
        [2013, 12, 14, 13, 11],
    ),
    (
        "moon",
        "Chang'e 4 and Yutu-2",
        Kind::Rover,
        -45.4446,
        177.5991,
        [2019, 1, 3, 2, 26],
    ),
    (
        "moon",
        "Chang'e 5",
        Kind::Lander,
        43.0576,
        -51.9161,
        [2020, 12, 1, 15, 11],
    ),
    (
        "moon",
        "Chandrayaan-3",
        Kind::Rover,
        -69.373,
        32.319,
        [2023, 8, 23, 12, 33],
    ),
    (
        "moon",
        "SLIM",
        Kind::Lander,
        -13.316,
        25.251,
        [2024, 1, 19, 15, 20],
    ),
    (
        "moon",
        "Odysseus",
        Kind::Lander,
        -80.13,
        1.44,
        [2024, 2, 22, 23, 23],
    ),
    (
        "moon",
        "Chang'e 6",
        Kind::Lander,
        -41.6385,
        -153.9852,
        [2024, 6, 1, 22, 23],
    ),
    (
        "moon",
        "Blue Ghost 1",
        Kind::Lander,
        18.56,
        61.81,
        [2025, 3, 2, 8, 34],
    ),
    (
        "mars",
        "Viking 1",
        Kind::Lander,
        22.27,
        -47.95,
        [1976, 7, 20, 11, 53],
    ),
    (
        "mars",
        "Viking 2",
        Kind::Lander,
        47.67,
        134.28,
        [1976, 9, 3, 22, 58],
    ),
    (
        "mars",
        "Pathfinder and Sojourner",
        Kind::Rover,
        19.13,
        -33.22,
        [1997, 7, 4, 16, 56],
    ),
    (
        "mars",
        "Phoenix",
        Kind::Lander,
        68.22,
        -125.75,
        [2008, 5, 25, 23, 38],
    ),
    (
        "mars",
        "InSight",
        Kind::Lander,
        4.502,
        135.623,
        [2018, 11, 26, 19, 52],
    ),
    (
        "mars",
        "Zhurong",
        Kind::Rover,
        25.066,
        109.925,
        [2021, 5, 14, 23, 18],
    ),
];

pub fn all() -> &'static [Track] {
    static T: OnceLock<Vec<Track>> = OnceLock::new();
    T.get_or_init(|| {
        let mut tracks = parse(DATA).unwrap_or_default();
        tracks.extend(
            SITES
                .iter()
                .map(|&(body, name, kind, lat, lon, [y, mo, d, h, mi])| {
                    let t = (crate::sky_time::days_from_civil(y, mo, d) * 86_400
                        + h * 3600
                        + mi * 60) as f64;
                    let v = Vertex {
                        t,
                        lat: crate::geo::from_planetocentric(lat.to_radians(), body),
                        lon: lon.to_radians(),
                        odo: 0.0,
                    };
                    let colour = if kind == Kind::Rover {
                        [255, 190, 120]
                    } else {
                        [230, 230, 230]
                    };
                    Track {
                        body: body.to_string(),
                        name: name.to_string(),
                        kind,
                        colour,
                        stays: true,
                        sol0: f64::NAN,
                        pieces: vec![vec![v]],
                        starts: vec![0],
                    }
                }),
        );
        tracks
    })
}

/// The tracks on a body, with their places in [`all`].
pub fn on(body: &str) -> impl Iterator<Item = (usize, &'static Track)> + '_ {
    all()
        .iter()
        .enumerate()
        .filter(move |(_, t)| t.body == body)
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn text(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

fn parse(data: &[u8]) -> Option<Vec<Track>> {
    let mut r = Reader(data);
    if r.take(4)? != b"ORBM" || r.u32()? != 1 {
        return None;
    }
    let n = r.u32()?;
    let mut tracks = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let body = r.text()?;
        let name = r.text()?;
        let kind = match r.u8()? {
            0 => Kind::Rover,
            1 => Kind::Helicopter,
            _ => Kind::Astronaut,
        };
        let colour = [r.u8()?, r.u8()?, r.u8()?];
        let stays = r.u8()? & 1 != 0;
        let sol0 = r.f64()?;
        let radius = crate::body::all()
            .iter()
            .find(|b| b.id == body)
            .map_or(1.0, |b| b.a);
        let mut pieces = Vec::new();
        let mut starts = Vec::new();
        let (mut odo, mut count) = (0.0, 0);
        let mut last: Option<(f64, f64)> = None;
        for _ in 0..r.u32()? {
            let m = r.u32()? as usize;
            let mut piece = Vec::with_capacity(m);
            for _ in 0..m {
                let t = r.f64()?;
                // Planetocentric in the data, geodetic on the globe.
                let lat =
                    crate::geo::from_planetocentric((r.i32()? as f64 * 1e-7).to_radians(), &body);
                let lon = (r.i32()? as f64 * 1e-7).to_radians();
                if let Some((plat, plon)) = last {
                    odo += angle_between(plat, plon, lat, lon) * radius;
                }
                last = Some((lat, lon));
                piece.push(Vertex { t, lat, lon, odo });
            }
            if !piece.is_empty() {
                starts.push(count);
                count += piece.len();
                pieces.push(piece);
            }
        }
        if !pieces.is_empty() {
            tracks.push(Track {
                body,
                name,
                kind,
                colour,
                stays,
                sol0,
                pieces,
                starts,
            });
        }
    }
    Some(tracks)
}

/// The angle between two places, in radians.
fn angle_between(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let h = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * h.sqrt().min(1.0).asin()
}

impl Track {
    pub fn vertex_count(&self) -> usize {
        self.starts
            .last()
            .map_or(0, |s| s + self.pieces.last().map_or(0, Vec::len))
    }

    /// The flat number of a vertex, as [`Track::vertex_count`] counts them.
    pub fn index(&self, piece: usize, vertex: usize) -> usize {
        self.starts[piece] + vertex
    }

    pub fn start(&self) -> f64 {
        self.pieces[0][0].t
    }

    pub fn end(&self) -> f64 {
        self.pieces
            .last()
            .and_then(|p| p.last())
            .map_or(f64::NAN, |v| v.t)
    }

    /// Where the traveller was at `t`; `None` before it set out, and after it was last seen
    /// unless it stays where it ended. Astronauts are inside the lander between their walks.
    pub fn at(&self, t: f64) -> Option<Spot> {
        if !(t >= self.start()) {
            return None;
        }
        if t > self.end() {
            let pi = self.pieces.len() - 1;
            return self
                .stays
                .then(|| self.spot(pi, self.pieces[pi].len() - 1, 0.0));
        }
        for (pi, piece) in self.pieces.iter().enumerate() {
            let last = piece.len() - 1;
            if t <= piece[last].t {
                if t < piece[0].t {
                    // Between two pieces: parked where the one before ended.
                    if self.kind == Kind::Astronaut {
                        return None;
                    }
                    let prev = &self.pieces[pi - 1];
                    return Some(self.spot(pi - 1, prev.len() - 1, 0.0));
                }
                let i = piece
                    .partition_point(|v| v.t <= t)
                    .saturating_sub(1)
                    .min(last);
                if i == last {
                    return Some(self.spot(pi, i, 0.0));
                }
                let (a, b) = (&piece[i], &piece[i + 1]);
                let frac = if b.t > a.t {
                    (t - a.t) / (b.t - a.t)
                } else {
                    1.0
                };
                return Some(self.spot(pi, i, frac.clamp(0.0, 1.0)));
            }
        }
        None
    }

    fn spot(&self, piece: usize, vertex: usize, frac: f64) -> Spot {
        let p = &self.pieces[piece];
        let a = p[vertex];
        let b = p.get(vertex + 1).copied().unwrap_or(a);
        let mix = |x: f64, y: f64| x + (y - x) * frac;
        Spot {
            lat: mix(a.lat, b.lat),
            lon: mix(a.lon, b.lon),
            odo: mix(a.odo, b.odo),
            piece,
            vertex,
            frac,
        }
    }

    /// What to say about the traveller at `t`: the sol and the distance so far on Mars, the
    /// walk and its distance on the Moon.
    pub fn status(&self, t: f64, spot: &Spot) -> String {
        if self.vertex_count() == 1 {
            let (y, m, d) =
                crate::sky_time::civil_from_days((self.start() / 86_400.0).floor() as i64);
            return format!(
                "Landed {d} {} {y}",
                [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec"
                ][(m - 1) as usize]
            );
        }
        let walked = if self.kind == Kind::Astronaut {
            spot.odo - self.pieces[spot.piece][0].odo
        } else {
            spot.odo
        };
        let distance = if walked < 1000.0 {
            format!("{walked:.0} m")
        } else {
            format!("{:.2} km", walked / 1000.0)
        };
        let verb = match self.kind {
            Kind::Rover => "driven",
            Kind::Helicopter => "flown",
            Kind::Astronaut => "walked",
            Kind::Lander => "",
        };
        if self.sol0.is_finite() {
            format!(
                "Sol {} · {distance} {verb}",
                ((t - self.sol0) / SOL).floor()
            )
        } else if self.kind == Kind::Astronaut {
            format!("EVA {} · {distance} {verb}", spot.piece + 1)
        } else {
            format!("{distance} {verb}")
        }
    }

    /// The area the whole track covers, as (south, west, north, east) in radians.
    fn bbox(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for v in self.pieces.iter().flatten() {
            b = [
                b[0].min(v.lat),
                b[1].min(v.lon),
                b[2].max(v.lat),
                b[3].max(v.lon),
            ];
        }
        b
    }
}

impl Track {
    /// The title of the English Wikipedia article about the mission.
    pub fn article(&self) -> &str {
        // A moonwalker's track belongs to the mission: "Apollo 12: Conrad".
        let name = self.name.split(':').next().unwrap_or(&self.name);
        match name {
            "Perseverance" => "Perseverance (rover)",
            "Ingenuity" => "Ingenuity (helicopter)",
            "Curiosity" => "Curiosity (rover)",
            "Spirit" => "Spirit (rover)",
            "Opportunity" => "Opportunity (rover)",
            "Chang'e 3 and Yutu" => "Chang'e 3",
            "Chang'e 4 and Yutu-2" => "Chang'e 4",
            "SLIM" => "Smart Lander for Investigating Moon",
            "Odysseus" => "IM-1",
            "Blue Ghost 1" => "Blue Ghost Mission 1",
            "Pathfinder and Sojourner" => "Mars Pathfinder",
            "Phoenix" => "Phoenix (spacecraft)",
            "Zhurong" => "Zhurong (rover)",
            other => other,
        }
    }
}

/// The track whose icon or line is under a window position, icons first; positions in pixels,
/// `ppp` pixels per point as the icons are drawn.
pub fn hit(marks: &[Mark], pos: DVec2, ppp: f64) -> Option<usize> {
    let icon = marks.iter().rev().find(|m| {
        m.icon.is_some_and(|(p, _)| {
            let d = (pos - p) / ppp;
            d.x.abs() <= 10.0 && (-16.0..=3.0).contains(&d.y)
        })
    });
    if let Some(m) = icon {
        return Some(m.track);
    }
    let near = |a: DVec2, b: DVec2| {
        let ab = b - a;
        let t = if ab.length_squared() > 0.0 {
            ((pos - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        pos.distance(a + ab * t) <= 5.0 * ppp
    };
    marks
        .iter()
        .rev()
        .find(|m| {
            m.done
                .iter()
                .chain(&m.todo)
                .any(|l| l.windows(2).any(|w| near(w[0], w[1])))
        })
        .map(|m| m.track)
}

/// Tracks on a body whose names hold what was typed.
pub fn search(body: &str, query: &str) -> Vec<Place> {
    let q = crate::planet_names::fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    on(body)
        .filter(|(_, t)| crate::planet_names::fold(&t.name).contains(&q))
        .take(MAX_RESULTS)
        .map(|(_, t)| {
            // A margin around the track, so that a short walk is not filled edge to edge.
            let [s, w, n, e] = t.bbox();
            let pad = ((n - s).max(e - w) * 0.25).max(2e-5);
            Place {
                name: t.name.clone(),
                lat: (s + n) / 2.0,
                lon: (w + e) / 2.0,
                bbox: Some([s - pad, w - pad, n + pad, e + pad]),
            }
        })
        .collect()
}

/// A track as it is to be drawn this frame, in window pixels.
pub struct Mark {
    /// The track's place in [`all`].
    pub track: usize,
    pub colour: [u8; 3],
    pub kind: Kind,
    /// The part travelled by the clock's time, as lines.
    pub done: Vec<Vec<DVec2>>,
    /// The part still to come.
    pub todo: Vec<Vec<DVec2>>,
    /// Where the traveller stands, and whether it faces left.
    pub icon: Option<(DVec2, bool)>,
    pub name: String,
    /// The sol and the distance so far, when close enough to read them.
    pub detail: Option<String>,
}

/// Works out how a track looks at time `t`. `project` gives the window position of a point at
/// a height above the surface, or `None` where it cannot be seen; `height` gives the ground
/// under a vertex by its flat number. `width` is the window's, to break lines that wrap
/// around the flat map.
pub fn mark(
    track: &Track,
    t: f64,
    project: &dyn Fn(f64, f64, f64) -> Option<DVec2>,
    height: &dyn Fn(usize) -> f64,
    width: f64,
    detail: bool,
) -> Mark {
    let spot = track.at(t);
    // Vertices up to `split` (piece, vertex, fraction) have been passed.
    let split = match spot {
        Some(s) => (s.piece, s.vertex, s.frac),
        None if t >= track.start() => (usize::MAX, 0, 0.0),
        None => (0, 0, -1.0),
    };
    let mut done = Vec::new();
    let mut todo = Vec::new();
    let mut spot_px = None;
    let mut facing_left = false;
    for (pi, piece) in track.pieces.iter().enumerate() {
        let mut line: Vec<DVec2> = Vec::new();
        let mut is_done = pi < split.0 || (pi == split.0 && split.2 >= 0.0);
        let flush = |line: &mut Vec<DVec2>,
                     done_now: bool,
                     done: &mut Vec<Vec<DVec2>>,
                     todo: &mut Vec<Vec<DVec2>>| {
            if line.iter().any(|p| *p != line[0]) {
                let l = std::mem::take(line);
                if done_now {
                    done.push(l);
                } else {
                    todo.push(l);
                }
            } else {
                line.clear();
            }
        };
        for (vi, v) in piece.iter().enumerate() {
            let p = project(v.lat, v.lon, height(track.index(pi, vi)));
            match p {
                Some(p) => {
                    if let Some(&last) = line.last() {
                        if (p.x - last.x).abs() > width / 2.0 {
                            flush(&mut line, is_done, &mut done, &mut todo);
                        }
                    }
                    let is_last = vi + 1 == piece.len();
                    if line
                        .last()
                        .is_none_or(|l: &DVec2| l.distance(p) >= MIN_STEP)
                        || is_last
                    {
                        line.push(p);
                    }
                }
                None => flush(&mut line, is_done, &mut done, &mut todo),
            }
            // The traveller's own position ends the travelled part and starts the rest.
            if is_done && pi == split.0 && vi == split.1 {
                if let Some(s) = spot {
                    let h = height(track.index(pi, vi));
                    let h2 = if vi + 1 < piece.len() {
                        height(track.index(pi, vi + 1))
                    } else {
                        h
                    };
                    let at = project(s.lat, s.lon, h + (h2 - h) * s.frac);
                    if let Some(at) = at {
                        line.push(at);
                        spot_px = Some(at);
                        facing_left = heading_left(track, pi, vi, project, height).unwrap_or(false);
                    }
                    flush(&mut line, true, &mut done, &mut todo);
                    if let Some(at) = at {
                        line.push(at);
                    }
                }
                is_done = false;
            }
        }
        flush(&mut line, is_done, &mut done, &mut todo);
    }
    let detail = (detail && spot.is_some() && spot_px.is_some())
        .then(|| track.status(t, spot.as_ref().unwrap()));
    Mark {
        track: all()
            .iter()
            .position(|t| std::ptr::eq(t, track))
            .unwrap_or(usize::MAX),
        colour: track.colour,
        kind: track.kind,
        done,
        todo,
        icon: spot_px.map(|p| (p, facing_left)),
        name: track.name.clone(),
        detail,
    }
}

/// Whether the traveller at a vertex heads left on screen: along the segment it is on, or the
/// last one that went anywhere.
fn heading_left(
    track: &Track,
    pi: usize,
    vi: usize,
    project: &dyn Fn(f64, f64, f64) -> Option<DVec2>,
    height: &dyn Fn(usize) -> f64,
) -> Option<bool> {
    let piece = &track.pieces[pi];
    let at = |i: usize| project(piece[i].lat, piece[i].lon, height(track.index(pi, i)));
    let (mut a, mut b) = if vi + 1 < piece.len() {
        (vi, vi + 1)
    } else {
        (vi.checked_sub(1)?, vi)
    };
    loop {
        if let (Some(pa), Some(pb)) = (at(a), at(b)) {
            if (pb.x - pa.x).abs() > 0.5 {
                return Some(pb.x < pa.x);
            }
        }
        if a == 0 {
            return None;
        }
        (a, b) = (a - 1, a);
        if vi - a > 50 {
            return None;
        }
    }
}

/// Draws the tracks under the interface's windows. Positions are in window pixels; `ppp` is
/// the number of pixels per point.
pub fn paint(ctx: &egui::Context, marks: &[Mark], ppp: f32) {
    use egui::{Color32, Pos2, Stroke};
    if marks.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::background());
    let pt = |p: DVec2| Pos2::new(p.x as f32 / ppp, p.y as f32 / ppp);
    let shadow = Color32::from_black_alpha(150);
    for m in marks {
        let [r, g, b] = m.colour;
        for line in &m.todo {
            let pts: Vec<Pos2> = line.iter().map(|p| pt(*p)).collect();
            painter.add(egui::Shape::line(
                pts,
                Stroke::new(1.5, Color32::from_rgba_unmultiplied(r, g, b, 110)),
            ));
        }
        for line in &m.done {
            let pts: Vec<Pos2> = line.iter().map(|p| pt(*p)).collect();
            painter.add(egui::Shape::line(pts.clone(), Stroke::new(4.0, shadow)));
            painter.add(egui::Shape::line(
                pts,
                Stroke::new(2.0, Color32::from_rgb(r, g, b)),
            ));
        }
    }
    let mut taken: Vec<egui::Rect> = Vec::new();
    for m in marks {
        let Some((p, left)) = m.icon else { continue };
        let p = pt(p);
        icon(
            &painter,
            m.kind,
            p,
            left,
            Color32::from_rgb(m.colour[0], m.colour[1], m.colour[2]),
        );
        taken.push(egui::Rect::from_min_max(
            p + egui::vec2(-9.0, -15.0),
            p + egui::vec2(9.0, 2.0),
        ));
    }
    // Names beside the icons, as many as fit without covering each other.
    for m in marks {
        let Some((p, _)) = m.icon else { continue };
        let p = pt(p) + egui::vec2(13.0, -14.0);
        let font = egui::FontId::proportional(13.0);
        let small = egui::FontId::proportional(11.0);
        let name = painter.layout_no_wrap(m.name.clone(), font, Color32::WHITE);
        let detail = m
            .detail
            .as_ref()
            .map(|d| painter.layout_no_wrap(d.clone(), small, Color32::from_gray(225)));
        let w = name
            .size()
            .x
            .max(detail.as_ref().map_or(0.0, |d| d.size().x));
        let h = name.size().y + detail.as_ref().map_or(0.0, |d| d.size().y);
        let rect = egui::Rect::from_min_size(p, egui::vec2(w, h)).expand(2.0);
        // Icons stay visible; a name that would cover another name or an icon is left out.
        if taken.iter().any(|t| t.intersects(rect)) {
            continue;
        }
        painter.rect_filled(rect, 3.0, Color32::from_black_alpha(140));
        let name_h = name.size().y;
        painter.galley(p, name, Color32::WHITE);
        if let Some(d) = detail {
            painter.galley(p + egui::vec2(0.0, name_h), d, Color32::from_gray(225));
        }
        taken.push(rect);
    }
}

/// A side view of a rover, a helicopter or an astronaut, about 14 points tall, standing on
/// `at` and facing right unless `left`.
fn icon(painter: &egui::Painter, kind: Kind, at: egui::Pos2, left: bool, colour: egui::Color32) {
    use egui::{Color32, Pos2, Rect, Stroke};
    let s = if left { -1.0 } else { 1.0 };
    let p = |x: f32, y: f32| Pos2::new(at.x + x * s, at.y + y);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_two_pos(p(x0, y0), p(x1, y1));
    let dark = Color32::from_gray(25);
    let edge = Stroke::new(1.0, dark);
    let line = |a: Pos2, b: Pos2, w: f32| {
        painter.line_segment([a, b], Stroke::new(w + 2.0, dark));
        painter.line_segment([a, b], Stroke::new(w, colour));
    };
    painter.circle_filled(at, 2.2, dark);
    match kind {
        Kind::Rover => {
            line(p(3.5, -6.0), p(3.5, -11.0), 1.4);
            painter.rect(
                rect(1.5, -13.0, 7.0, -10.5),
                1.0,
                colour,
                edge,
                egui::StrokeKind::Outside,
            );
            painter.rect(
                rect(-7.0, -7.0, 6.5, -3.8),
                1.0,
                colour,
                edge,
                egui::StrokeKind::Outside,
            );
            for x in [-5.5, 0.0, 5.0] {
                painter.circle(p(x, -1.9), 1.9, dark, Stroke::new(1.0, colour));
            }
        }
        Kind::Helicopter => {
            line(p(-2.5, -3.5), p(-5.0, -0.5), 1.0);
            line(p(2.5, -3.5), p(5.0, -0.5), 1.0);
            painter.circle(p(0.0, -5.5), 3.2, colour, edge);
            line(p(0.0, -8.5), p(0.0, -12.5), 1.0);
            line(p(-8.0, -11.0), p(8.0, -11.0), 1.2);
            line(p(-7.0, -13.5), p(7.0, -13.5), 1.2);
        }
        Kind::Lander => {
            line(p(-3.0, -5.0), p(-6.5, -0.5), 1.2);
            line(p(3.0, -5.0), p(6.5, -0.5), 1.2);
            painter.rect(
                rect(-4.5, -9.5, 4.5, -4.0),
                1.0,
                colour,
                edge,
                egui::StrokeKind::Outside,
            );
            painter.rect(
                rect(-3.0, -13.0, 3.0, -9.5),
                1.0,
                colour,
                edge,
                egui::StrokeKind::Outside,
            );
        }
        Kind::Astronaut => {
            painter.rect(
                rect(-5.5, -11.0, -2.5, -5.5),
                1.0,
                colour,
                edge,
                egui::StrokeKind::Outside,
            );
            line(p(-1.0, -5.0), p(-1.5, -0.5), 1.6);
            line(p(1.5, -5.0), p(2.0, -0.5), 1.6);
            painter.rect(
                rect(-2.5, -10.5, 3.0, -4.5),
                1.5,
                colour,
                edge,
                egui::StrokeKind::Outside,
            );
            painter.circle(p(0.5, -13.0), 2.6, colour, edge);
            painter.rect_filled(rect(1.2, -14.0, 3.0, -12.2), 0.5, dark);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(name: &str) -> &'static Track {
        all().iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn the_tracks_are_read_with_rising_times() {
        assert_eq!(all().len(), 9 + SITES.len());
        assert!(on("moon").count() > 20 && on("mars").count() > 10);
        for t in all().iter().take(9) {
            assert!(t.vertex_count() > 10, "{}", t.name);
            for piece in &t.pieces {
                assert!(
                    piece
                        .windows(2)
                        .all(|w| w[0].t <= w[1].t && w[0].odo <= w[1].odo),
                    "{}",
                    t.name
                );
            }
        }
    }

    #[test]
    fn rovers_move_along_their_track_and_stay_where_they_ended() {
        let p = track("Perseverance");
        assert_eq!(p.kind, Kind::Rover);
        assert!(p.at(p.start() - 1.0).is_none());
        let first = p.at(p.start()).unwrap();
        let centric = crate::geo::to_planetocentric(first.lat, "mars").to_degrees();
        assert!((centric - 18.44).abs() < 0.05 && (first.lon.to_degrees() - 77.45).abs() < 0.05);
        let later = p.at(p.start() + 400.0 * 86_400.0).unwrap();
        assert!(later.odo > first.odo + 1000.0);
        let end = p.at(p.end() + 1e8).unwrap();
        assert!(end.odo > 20_000.0, "{}", end.odo);
        assert!(
            p.status(p.start() + 1.0, &first).starts_with("Sol 0 ")
                || p.status(p.start() + 1.0, &first).starts_with("Sol 1 ")
        );
    }

    #[test]
    fn astronauts_are_only_out_during_their_walks() {
        let a = track("Apollo 14: Shepard");
        assert_eq!((a.kind, a.pieces.len()), (Kind::Astronaut, 2));
        let first_end = a.pieces[0].last().unwrap().t;
        let second_start = a.pieces[1][0].t;
        assert!(a.at(first_end - 60.0).is_some());
        assert!(a.at((first_end + second_start) / 2.0).is_none());
        assert!(a.at(a.end() + 60.0).is_none());
        let walk = a.at(second_start + 1800.0).unwrap();
        assert!(a
            .status(second_start + 1800.0, &walk)
            .starts_with("EVA 2 · "));
    }

    #[test]
    fn the_travelled_part_ends_at_the_traveller() {
        let c = track("Curiosity");
        let t = c.start() + 1000.0 * 86_400.0;
        // A flat projection, pixels per radian.
        let project = |lat: f64, lon: f64, _h: f64| Some(DVec2::new(lon * 1e5, -lat * 1e5));
        let m = mark(c, t, &project, &|_| 0.0, 1e9, true);
        let (at, _) = m.icon.unwrap();
        assert_eq!(*m.done.last().unwrap().last().unwrap(), at);
        assert_eq!(m.todo[0][0], at);
        assert!(m.detail.unwrap().starts_with("Sol "));
        let before = mark(c, c.start() - 1.0, &project, &|_| 0.0, 1e9, true);
        assert!(before.icon.is_none() && before.done.is_empty() && !before.todo.is_empty());
    }

    #[test]
    fn landing_sites_are_shown_from_their_landing_on() {
        let a = track("Apollo 11");
        assert_eq!(a.kind, Kind::Lander);
        assert!(a.at(a.start() - 60.0).is_none());
        let now = 1.79e9;
        let spot = a.at(now).unwrap();
        assert!((spot.lat.to_degrees() - 0.674).abs() < 1e-3);
        assert_eq!(a.status(now, &spot), "Landed 20 Jul 1969");
        let project = |lat: f64, lon: f64, _h: f64| Some(DVec2::new(lon * 1e5, -lat * 1e5));
        let m = mark(a, now, &project, &|_| 0.0, 1e9, false);
        assert!(m.icon.is_some() && m.done.is_empty() && m.todo.is_empty());
    }

    #[test]
    fn clicks_on_icons_and_lines_find_the_track() {
        let c = track("Curiosity");
        let t = c.start() + 1000.0 * 86_400.0;
        let project = |lat: f64, lon: f64, _h: f64| Some(DVec2::new(lon * 1e5, -lat * 1e5));
        let m = mark(c, t, &project, &|_| 0.0, 1e9, false);
        let (at, _) = m.icon.unwrap();
        let marks = [m];
        assert_eq!(
            hit(&marks, at + DVec2::new(0.0, -8.0), 1.0),
            Some(marks[0].track)
        );
        assert_eq!(all()[marks[0].track].article(), "Curiosity (rover)");
        let mid = marks[0].todo[0][1];
        assert_eq!(
            hit(&marks, mid + DVec2::new(3.0, 0.0), 1.0),
            Some(marks[0].track)
        );
        assert_eq!(hit(&marks, at + DVec2::new(500.0, 500.0), 1.0), None);
        assert_eq!(track("Apollo 14: Shepard").article(), "Apollo 14");
    }

    #[test]
    fn search_finds_tracks_by_name() {
        let found = search("moon", "apollo 12");
        assert_eq!(found.len(), 3, "the landing and both moonwalkers");
        assert!(search("mars", "ingen")[0].name == "Ingenuity");
        assert!(search("mars", "apollo").is_empty());
    }
}
