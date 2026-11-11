//! Saved places: a small JSON file per user, and the choice list that shows it. Independent of
//! the window system. A file that cannot be read is moved aside, never overwritten.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const VERSION: u64 = 1;
/// Larger files are not ours.
const MAX_FILE_BYTES: u64 = 4 << 20;
const MAX_PLACES: usize = 2000;
const MAX_NAME_CHARS: usize = 80;
const NAME_COLUMNS: usize = 56;
const ROWS: usize = 12;

/// A view worth coming back to. Angles are in radians, distances in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedPlace {
    pub name: String,
    /// The id of the body, as in `body.rs`.
    pub body: String,
    pub lat: f64,
    pub lon: f64,
    pub distance: f64,
    pub heading: f64,
    pub tilt: f64,
}

impl SavedPlace {
    fn to_json(&self) -> Value {
        json!({
            "name": self.name, "body": self.body,
            "lat_deg": self.lat.to_degrees(), "lon_deg": self.lon.to_degrees(),
            "distance_m": self.distance,
            "heading_deg": self.heading.to_degrees(), "tilt_deg": self.tilt.to_degrees(),
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        let num = |k: &str| v.get(k)?.as_f64().filter(|x| x.is_finite());
        let place = SavedPlace {
            name: clean_name(v.get("name")?.as_str()?),
            body: v.get("body")?.as_str()?.to_string(),
            lat: num("lat_deg")?.to_radians(),
            lon: num("lon_deg")?.to_radians(),
            distance: num("distance_m")?,
            heading: num("heading_deg")?.to_radians(),
            tilt: num("tilt_deg")?.to_radians(),
        };
        let sane = place.lat.abs() <= std::f64::consts::FRAC_PI_2 + 1e-9
            && place.lon.abs() <= std::f64::consts::PI + 1e-9
            && place.distance > 0.0
            && !place.name.is_empty();
        sane.then_some(place)
    }
}

fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A name made from the position, for a place saved without one.
pub fn default_name(body_name: &str, earth: bool, lat: f64, lon: f64) -> String {
    let (lat, lon) = (lat.to_degrees(), lon.to_degrees());
    let position = format!(
        "{:.4}°{} {:.4}°{}",
        lat.abs(),
        if lat >= 0.0 { 'N' } else { 'S' },
        lon.abs(),
        if lon >= 0.0 { 'E' } else { 'W' }
    );
    if earth {
        position
    } else {
        format!("{body_name} {position}")
    }
}

/// The folder and file for the current user.
pub fn default_path() -> Option<PathBuf> {
    Some(
        crate::paths::config_dir()?
            .join("orbis")
            .join("places.json"),
    )
}

pub struct Store {
    path: Option<PathBuf>,
    places: Vec<SavedPlace>,
    /// Set when the file could not be read and could not be moved aside: saving would lose it.
    locked: bool,
    /// Something the user should be told about the file.
    pub notice: Option<String>,
}

impl Store {
    /// A store that is never written, for a machine without a configuration folder or for tests.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            places: Vec::new(),
            locked: false,
            notice: None,
        }
    }

    /// Reads the file. A missing file is an empty list; one that cannot be understood is moved
    /// to a `.bak` next to it and reported.
    pub fn load(path: PathBuf) -> Self {
        let mut store = Self {
            path: Some(path.clone()),
            places: Vec::new(),
            locked: false,
            notice: None,
        };
        let bytes = match read_limited(&path) {
            Ok(Some(b)) => b,
            Ok(None) => return store,
            Err(e) => return store.corrupt(&path, &e),
        };
        match parse(&bytes) {
            Ok(places) => store.places = places,
            Err(e) => return store.corrupt(&path, &e),
        }
        store
    }

    fn corrupt(mut self, path: &Path, why: &str) -> Self {
        let mut backup = path.to_path_buf();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut chosen = None;
        for n in 0..10 {
            backup.set_file_name(if n == 0 {
                format!("{name}.bak")
            } else {
                format!("{name}.bak.{n}")
            });
            if !backup.exists() {
                chosen = Some(backup.clone());
                break;
            }
        }
        let backup = chosen.unwrap_or_else(|| {
            backup.set_file_name(format!("{name}.bak"));
            backup
        });
        match fs::rename(path, &backup) {
            Ok(()) => {
                self.notice = Some(format!(
                    "Saved places could not be read ({why}). The file was kept as {} and the list starts empty.",
                    backup.display()
                ));
            }
            Err(e) => {
                self.locked = true;
                self.notice = Some(format!(
                    "Saved places could not be read ({why}) nor moved aside ({e}). They will not be written."
                ));
            }
        }
        self
    }

    pub fn places(&self) -> &[SavedPlace] {
        &self.places
    }

    pub fn add(&mut self, mut place: SavedPlace) -> Result<(), String> {
        if self.places.len() >= MAX_PLACES {
            return Err(format!("At most {MAX_PLACES} places can be saved."));
        }
        place.name = clean_name(&place.name);
        self.places.push(place);
        self.save()
    }

    pub fn remove(&mut self, index: usize) -> Result<(), String> {
        if index < self.places.len() {
            self.places.remove(index);
        }
        self.save()
    }

    /// Writes the whole list to a temporary file and renames it over the real one.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if self.locked {
            return Err("Saved places are not written because the file could not be read.".into());
        }
        let doc = json!({ "version": VERSION, "places": self.places.iter().map(SavedPlace::to_json).collect::<Vec<_>>() });
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        write_atomically(path, text.as_bytes())
            .map_err(|e| format!("Saved places could not be written: {e}"))
    }
}

pub(crate) fn read_limited(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if meta.len() > MAX_FILE_BYTES {
        return Err("file too large".into());
    }
    fs::read(path).map(Some).map_err(|e| e.to_string())
}

fn parse(bytes: &[u8]) -> Result<Vec<SavedPlace>, String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|_| "not valid JSON".to_string())?;
    match doc.get("version").and_then(Value::as_u64) {
        Some(VERSION) => {}
        Some(_) => return Err("written by a newer version".into()),
        None => return Err("no version".into()),
    }
    let list = doc
        .get("places")
        .and_then(Value::as_array)
        .ok_or("no list of places")?;
    if list.len() > MAX_PLACES {
        return Err("too many places".into());
    }
    list.iter()
        .map(|v| SavedPlace::from_json(v).ok_or_else(|| "an entry is damaged".to_string()))
        .collect()
}

pub(crate) fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.to_path_buf();
    tmp.set_extension("json.tmp");
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Delete,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pick {
    None,
    Fly(usize),
    Delete(usize),
}

/// A panel listing things, one of which is chosen.
#[derive(Default)]
pub struct Picker {
    open: bool,
    selected: usize,
}

impl Picker {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.selected = 0;
    }

    /// Where the choice is now.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Opens the list with a row chosen.
    pub fn open_at(&mut self, row: usize) {
        self.open = true;
        self.selected = row;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Keeps the choice inside a list that has changed.
    pub fn clamp(&mut self, len: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    pub fn key(&mut self, key: Key, len: usize) -> Pick {
        if !self.open {
            return Pick::None;
        }
        match key {
            Key::Escape => {
                self.open = false;
                Pick::None
            }
            Key::Up | Key::Down if len > 0 => {
                self.selected = if key == Key::Up {
                    (self.selected + len - 1) % len
                } else {
                    (self.selected + 1) % len
                };
                Pick::None
            }
            Key::Enter if self.selected < len => Pick::Fly(self.selected),
            Key::Delete if self.selected < len => {
                let i = self.selected;
                self.selected = i.min(len.saturating_sub(2));
                Pick::Delete(i)
            }
            _ => Pick::None,
        }
    }

    /// The panel's text, for `len` rows that `row` makes on demand.
    pub fn panel_text(
        &self,
        title: &str,
        len: usize,
        row: impl Fn(usize) -> String,
        empty: &str,
        hint: &str,
    ) -> String {
        if !self.open {
            return String::new();
        }
        let mut s = String::from(title);
        if len == 0 {
            s.push('\n');
            s.push_str(empty);
        }
        // A window of rows that keeps the chosen one in view.
        let first = self
            .selected
            .saturating_sub(ROWS - 1)
            .min(len.saturating_sub(ROWS));
        for i in first..len.min(first + ROWS) {
            let mark = if i == self.selected { '>' } else { ' ' };
            s.push_str(&format!("\n{mark} {}", shorten(&row(i), NAME_COLUMNS)));
        }
        if len > ROWS {
            s.push_str(&format!("\n({len} in all)"));
        }
        s.push('\n');
        s.push_str(hint);
        s
    }
}

fn shorten(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let mut out: String = name.chars().take(max - 3).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("orbis-places-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn place(name: &str) -> SavedPlace {
        SavedPlace {
            name: name.into(),
            body: "earth".into(),
            lat: 0.8,
            lon: 0.2,
            distance: 12_345.0,
            heading: 1.0,
            tilt: 0.5,
        }
    }

    #[test]
    fn places_survive_a_round_trip() {
        let d = dir("round");
        let path = d.join("orbis").join("places.json");
        let mut store = Store::load(path.clone());
        assert!(store.places().is_empty() && store.notice.is_none());
        store.add(place("Home")).unwrap();
        store
            .add(SavedPlace {
                body: "moon".into(),
                ..place("Tycho")
            })
            .unwrap();
        let again = Store::load(path.clone());
        assert!(again.notice.is_none());
        assert_eq!(again.places().len(), 2);
        let p = &again.places()[0];
        assert_eq!(p.name, "Home");
        for (a, b) in [
            (p.lat, 0.8),
            (p.lon, 0.2),
            (p.heading, 1.0),
            (p.tilt, 0.5),
            (p.distance, 12_345.0),
        ] {
            assert!((a - b).abs() < 1e-9);
        }
        assert_eq!(again.places()[1].body, "moon");
        let mut again = again;
        again.remove(0).unwrap();
        assert_eq!(Store::load(path).places()[0].name, "Tycho");
        assert!(!d.join("orbis").join("places.json.tmp").exists());
    }

    #[test]
    fn a_damaged_file_is_kept_and_reported() {
        for (n, content) in [
            "{ not json",
            r#"{"version":1,"places":[{"name":"x"}]}"#,
            r#"{"version":7,"places":[]}"#,
        ]
        .iter()
        .enumerate()
        {
            let d = dir(&format!("bad{n}"));
            let path = d.join("places.json");
            fs::write(&path, content).unwrap();
            let mut store = Store::load(path.clone());
            assert!(store.places().is_empty());
            assert!(store.notice.as_deref().unwrap().contains("places.json.bak"));
            assert_eq!(
                fs::read_to_string(d.join("places.json.bak")).unwrap(),
                *content
            );
            store.add(place("New")).unwrap();
            assert_eq!(Store::load(path).places().len(), 1);
            assert_eq!(
                fs::read_to_string(d.join("places.json.bak")).unwrap(),
                *content
            );
        }
    }

    #[test]
    fn an_earlier_backup_is_not_overwritten() {
        let d = dir("twice");
        let path = d.join("places.json");
        fs::write(d.join("places.json.bak"), "first").unwrap();
        fs::write(&path, "second").unwrap();
        Store::load(path);
        assert_eq!(
            fs::read_to_string(d.join("places.json.bak")).unwrap(),
            "first"
        );
        assert_eq!(
            fs::read_to_string(d.join("places.json.bak.1")).unwrap(),
            "second"
        );
    }

    #[test]
    fn nonsense_values_are_refused() {
        let bad = r#"{"version":1,"places":[{"name":"x","body":"earth","lat_deg":120,"lon_deg":0,"distance_m":10,"heading_deg":0,"tilt_deg":0}]}"#;
        assert!(parse(bad.as_bytes()).is_err());
        assert!(parse(b"[]").is_err());
    }

    #[test]
    fn names_are_cleaned_and_defaults_are_made_from_the_position() {
        assert_eq!(clean_name("  a\nb\t "), "ab");
        assert_eq!(clean_name(&"x".repeat(500)).chars().count(), MAX_NAME_CHARS);
        assert_eq!(
            default_name("Earth", true, 0.8_f64, -0.2_f64),
            "45.8366°N 11.4592°W"
        );
        assert!(default_name("Mars", false, 0.0, 0.0).starts_with("Mars 0.0000°N"));
    }

    #[test]
    fn the_picker_moves_and_reports_choices() {
        let mut p = Picker::default();
        assert_eq!(p.key(Key::Enter, 3), Pick::None, "closed");
        p.toggle();
        assert_eq!(p.key(Key::Down, 3), Pick::None);
        assert_eq!(p.key(Key::Enter, 3), Pick::Fly(1));
        assert_eq!(p.key(Key::Up, 3), Pick::None);
        assert_eq!(p.key(Key::Up, 3), Pick::None);
        assert_eq!(p.key(Key::Enter, 3), Pick::Fly(2), "wraps round");
        assert_eq!(p.key(Key::Delete, 3), Pick::Delete(2));
        assert_eq!(
            p.key(Key::Enter, 2),
            Pick::Fly(1),
            "the choice stays inside the shorter list"
        );
        assert_eq!(p.key(Key::Enter, 0), Pick::None);
        p.key(Key::Escape, 2);
        assert!(!p.is_open());
    }

    #[test]
    fn the_panel_lists_rows_and_marks_the_choice() {
        let mut p = Picker::default();
        p.toggle();
        let rows: Vec<String> = (0..30).map(|i| format!("row {i}")).collect();
        for _ in 0..20 {
            p.key(Key::Down, rows.len());
        }
        let text = p.panel_text("Title", rows.len(), |i| rows[i].clone(), "none", "hint");
        assert!(text.contains("> row 20"));
        assert!(!text.contains("row 0\n"));
        assert!(text.contains("(30 in all)") && text.ends_with("hint"));
        assert!(p
            .panel_text("T", 0, |_| String::new(), "Nothing here.", "h")
            .contains("Nothing here."));
    }
}
