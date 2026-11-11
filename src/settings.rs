//! The saved view options: one `settings.json`, read at start and written when the user changes
//! something. Nothing here knows about windows, so it is tested alone.
//!
//! Where it lives: `--config-dir DIR`, else `./config/` when the working directory has one (the
//! development configuration, committed with the source), else `orbis/` in the user's config
//! folder. Saved places and map services are personal data and stay in the user's folder.
//!
//! To add a setting, add one line to the `settings!` list below with its default and its key in
//! the file, change `Scene` to use it (`Scene::view`), and add a row to the options dialog. Unknown
//! keys in the file are kept as they are, and a missing or damaged value means the default.

use crate::atlas::Projection;
use crate::energy::{Detail, Idle, Palette, Style};
use crate::pacing::FrameLimit;
use crate::world_energy::MapSignal;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// One kind of value in the file.
pub trait Setting: Sized {
    fn from_json(v: &Value) -> Option<Self>;
    fn to_json(&self) -> Value;
}

impl Setting for bool {
    fn from_json(v: &Value) -> Option<Self> {
        v.as_bool()
    }
    fn to_json(&self) -> Value {
        Value::Bool(*self)
    }
}

/// A short name such as a language code or an identifier; empty means "the usual".
impl Setting for String {
    fn from_json(v: &Value) -> Option<Self> {
        let s = v.as_str()?;
        (s.len() <= 200 && !s.contains('\n')).then(|| s.to_string())
    }
    fn to_json(&self) -> Value {
        Value::String(self.clone())
    }
}

/// A number; one that is not finite is unusable.
impl Setting for f64 {
    fn from_json(v: &Value) -> Option<Self> {
        v.as_f64().filter(|x| x.is_finite())
    }
    fn to_json(&self) -> Value {
        // Rounded, so that the file stays readable after a slider was dragged.
        serde_json::Number::from_f64((self * 1000.0).round() / 1000.0).map_or(Value::Null, Value::Number)
    }
}

impl Setting for Palette {
    fn from_json(v: &Value) -> Option<Self> {
        Palette::parse(v.as_str()?)
    }
    fn to_json(&self) -> Value {
        Value::from(self.key())
    }
}

impl Setting for Detail {
    fn from_json(v: &Value) -> Option<Self> {
        Detail::parse(v.as_str()?)
    }
    fn to_json(&self) -> Value {
        Value::from(self.key())
    }
}

impl Setting for MapSignal {
    fn from_json(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "renewable_share" => Some(Self::RenewableShare),
            "carbon_intensity" => Some(Self::CarbonIntensity),
            _ => None,
        }
    }
    fn to_json(&self) -> Value {
        Value::from(match self {
            Self::RenewableShare => "renewable_share",
            Self::CarbonIntensity => "carbon_intensity",
        })
    }
}

impl Setting for Idle {
    fn from_json(v: &Value) -> Option<Self> {
        Idle::parse(v.as_str()?)
    }
    fn to_json(&self) -> Value {
        Value::from(self.key())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    Globe,
    Map,
}

impl Setting for ViewKind {
    fn from_json(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "globe" => Some(Self::Globe),
            "map" => Some(Self::Map),
            _ => None,
        }
    }
    fn to_json(&self) -> Value {
        Value::from(match self {
            Self::Globe => "globe",
            Self::Map => "map",
        })
    }
}

/// How the 3D buildings are coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildingColour {
    /// Roofs from the aerial imagery, walls from the roof's average.
    Aerial,
    /// By zone: living, business, industry, or the colour in the map data.
    Zones,
}

impl Setting for BuildingColour {
    fn from_json(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "aerial" => Some(Self::Aerial),
            "zones" => Some(Self::Zones),
            _ => None,
        }
    }
    fn to_json(&self) -> Value {
        Value::from(match self {
            Self::Aerial => "aerial",
            Self::Zones => "zones",
        })
    }
}

impl Setting for Projection {
    fn from_json(v: &Value) -> Option<Self> {
        Projection::parse(v.as_str()?)
    }
    fn to_json(&self) -> Value {
        Value::from(self.name())
    }
}

impl Setting for FrameLimit {
    fn from_json(v: &Value) -> Option<Self> {
        FrameLimit::parse(&v.as_u64()?.to_string())
    }
    fn to_json(&self) -> Value {
        Value::from(self.hz())
    }
}

macro_rules! settings {
    ($( $(#[$doc:meta])* $name:ident : $ty:ty = $default:expr, $key:literal; )*) => {
        /// Every saved option, as the program uses it.
        #[derive(Clone, Debug, PartialEq)]
        pub struct Settings {
            $( $(#[$doc])* pub $name: $ty, )*
        }

        impl Default for Settings {
            fn default() -> Self {
                Self { $( $name: $default, )* }
            }
        }

        impl Settings {
            /// Reads what the file holds. A value that is missing or wrong is the default, and
            /// a wrong one is reported in `warnings`.
            fn read(map: &Map<String, Value>, warnings: &mut Vec<String>) -> Self {
                let mut s = Self::default();
                $(
                    match map.get($key) {
                        None => {}
                        Some(v) => match <$ty as Setting>::from_json(v) {
                            Some(x) => s.$name = x,
                            None => warnings.push(format!("{}: unusable value {v}, using the default", $key)),
                        },
                    }
                )*
                s
            }

            fn write(&self, map: &mut Map<String, Value>) {
                $( map.insert($key.to_string(), self.$name.to_json()); )*
            }

            /// Copies into `target` each value that differs between `self` and `before`.
            /// Returns whether there was one.
            pub fn copy_changes(&self, before: &Settings, target: &mut Settings) -> bool {
                let mut any = false;
                $(
                    if self.$name != before.$name {
                        target.$name = self.$name.clone();
                        any = true;
                    }
                )*
                any
            }
        }
    };
}

settings! {
    /// Roads for vehicles, and ferries.
    streets: bool = true, "streets";
    /// Paths and tracks.
    trails: bool = false, "trails";
    /// Railways and public transport lines.
    trains: bool = false, "trains";
    boundaries: bool = true, "boundaries";
    /// Names of countries, states, cities, towns, villages and quarters.
    cities: bool = true, "cities";
    /// Names of peaks, water and streets.
    other_names: bool = false, "other_names";
    buildings: bool = true, "buildings_3d";
    buildings_colour: BuildingColour = BuildingColour::Aerial, "buildings_colour";
    /// Terrain and buildings cast shadows when the Sun lights the scene.
    cast_shadows: bool = true, "cast_shadows";
    sunlight: bool = true, "sunlight";
    stars: bool = true, "stars_and_planets";
    event_markers: bool = true, "event_markers";
    /// The tracks of rovers, Ingenuity and the Apollo moonwalkers, with where each was at the clock's time.
    missions: bool = true, "missions";
    /// Bars of renewable generating capacity in Germany, and the areas they stand for.
    energy: bool = true, "energy";
    /// The indicator used to colour countries on the flat map.
    energy_map_signal: MapSignal = MapSignal::RenewableShare, "energy_map_signal";
    /// The Electricity window (detail, scale, totals) while the bars are on; its close button turns this off.
    energy_panel: bool = true, "energy_panel";
    /// The bars show the estimated output at the clock's time, from SMARD, as far as there is data.
    energy_output: bool = true, "energy_output";
    /// Output includes generation for the owners' own use: SMARD's quarter hours brought to Ember's monthly sums.
    energy_own_use: bool = true, "energy_own_use";
    /// A chart of the clock's day of generation and consumption, while the bars show output.
    energy_chart: bool = true, "energy_chart";
    /// Beside each area's bar, its estimated demand: Germany's consumption shared out by population.
    energy_demand: bool = true, "energy_demand";
    energy_solar: bool = true, "energy_solar";
    energy_wind_onshore: bool = true, "energy_wind_onshore";
    energy_wind_offshore: bool = true, "energy_wind_offshore";
    energy_hydro: bool = true, "energy_hydro";
    energy_biomass: bool = true, "energy_biomass";
    energy_lignite: bool = true, "energy_lignite";
    energy_hard_coal: bool = true, "energy_hard_coal";
    energy_gas: bool = true, "energy_gas";
    energy_oil: bool = true, "energy_oil";
    energy_other_fossil: bool = true, "energy_other_fossil";
    energy_nuclear: bool = true, "energy_nuclear";
    /// The colours of the sources, on the bars and in the day chart.
    energy_palette: Palette = Palette::Standard, "energy_palette";
    /// How the capacity not running at the clock's time shows above the output.
    energy_idle: Idle = Idle::White, "energy_idle";
    /// Its opacity, 0 to 1.
    energy_idle_opacity: f64 = 0.2, "energy_idle_opacity";
    /// How much detail the bars show: "auto" follows the altitude.
    energy_detail: Detail = Detail::Fixed(crate::energy::Level::States), "energy_detail";
    /// Factors on the usual height and width of the bars.
    energy_bar_height: f64 = 1.0, "energy_bar_height";
    energy_bar_width: f64 = 1.0, "energy_bar_width";
    /// A click on the map looks up Wikipedia and Wikidata for the place.
    info_on_click: bool = true, "info_on_click";
    view: ViewKind = ViewKind::Globe, "view";
    projection: Projection = Projection::EqualEarth, "projection";
    frame_rate: FrameLimit = FrameLimit::Fps120, "frame_rate_limit";
    /// A language code for names; empty follows the system.
    language: String = String::new(), "language";
    /// What the globe shows, by identifier.
    body: String = "earth".to_string(), "body";
    /// The library service to show instead of the usual imagery, by identifier; empty for none.
    base_service: String = String::new(), "base_service";
}

impl Settings {
    /// How the energy bars and the day chart look.
    pub fn energy_style(&self) -> Style {
        Style {
            palette: self.energy_palette,
            idle: self.energy_idle,
            idle_opacity: self.energy_idle_opacity,
            height: self.energy_bar_height,
            width: self.energy_bar_width,
        }
    }

    pub fn set_energy_style(&mut self, style: Style) {
        self.energy_palette = style.palette;
        self.energy_idle = style.idle;
        self.energy_idle_opacity = style.idle_opacity;
        self.energy_bar_height = style.height;
        self.energy_bar_width = style.width;
    }

    /// The text of `settings.json` for these values.
    #[cfg(test)]
    pub fn to_text(&self) -> String {
        let mut map = Map::new();
        self.write(&mut map);
        text_of(&map)
    }

    /// Parses a file's text. Wrong values are the defaults; a text that is not a JSON object at
    /// all gives the defaults and one warning.
    #[cfg(test)]
    pub fn parse(text: &str, warnings: &mut Vec<String>) -> Self {
        Self::parse_with_rest(text, warnings).0
    }

    fn parse_with_rest(text: &str, warnings: &mut Vec<String>) -> (Self, Map<String, Value>) {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => (Self::read(&map, warnings), map),
            Ok(_) => {
                warnings.push("not a JSON object, using the defaults".into());
                (Self::default(), Map::new())
            }
            Err(e) => {
                warnings.push(format!("not valid JSON ({e}), using the defaults"));
                (Self::default(), Map::new())
            }
        }
    }
}

fn text_of(map: &Map<String, Value>) -> String {
    let mut text = serde_json::to_string_pretty(map).unwrap_or_default();
    text.push('\n');
    text
}

/// The folder for `settings.json`: the flag, else a `config` folder in the working directory,
/// else the user's.
pub fn config_dir(flag: Option<&Path>, work_dir: &Path, user: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(d) = flag {
        return Some(d.to_path_buf());
    }
    let dev = work_dir.join("config");
    if dev.is_dir() {
        return Some(dev);
    }
    user
}

pub fn user_config_dir() -> Option<PathBuf> {
    Some(crate::paths::config_dir()?.join("orbis"))
}

/// The saved values, and the file they are kept in.
pub struct Store {
    path: Option<PathBuf>,
    pub values: Settings,
    /// What the file held when it was last read or written: nothing is written unless `values`
    /// differ from this.
    on_disk: Settings,
    /// Keys of the file that this program does not know, kept when it writes.
    rest: Map<String, Value>,
    pub warnings: Vec<String>,
}

impl Store {
    /// Defaults, with nowhere to save.
    pub fn in_memory() -> Self {
        Self { path: None, values: Settings::default(), on_disk: Settings::default(), rest: Map::new(), warnings: Vec::new() }
    }

    /// Reads `settings.json` in `dir`. A missing file is the defaults, silently; a damaged one is
    /// the defaults with a warning, and is moved aside to `settings.json.bad` so that it is not lost.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("settings.json");
        let mut store = Self { path: Some(path.clone()), ..Self::in_memory() };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return store,
            Err(e) => {
                store.warnings.push(format!("{}: {e}", path.display()));
                return store;
            }
        };
        let mut warnings = Vec::new();
        let (values, rest) = Settings::parse_with_rest(&text, &mut warnings);
        let whole_file_bad = warnings.iter().any(|w| w.starts_with("not "));
        if whole_file_bad {
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
        }
        store.warnings = warnings.into_iter().map(|w| format!("{}: {w}", path.display())).collect();
        store.on_disk = values.clone();
        store.values = values;
        store.rest = rest;
        // A value that was unusable is written back as the default with the next change.
        store
    }

    /// Writes the file if the values changed since it was read or written. Atomic: a temporary
    /// file is renamed over the real one.
    pub fn save_if_changed(&mut self) -> Result<bool, String> {
        let Some(path) = &self.path else { return Ok(false) };
        if self.values == self.on_disk {
            return Ok(false);
        }
        let mut map = self.rest.clone();
        self.values.write(&mut map);
        crate::places::write_atomically(path, text_of(&map).as_bytes()).map_err(|e| format!("could not save {}: {e}", path.display()))?;
        self.on_disk = self.values.clone();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("orbis-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_committed_development_file_holds_the_defaults() {
        // A window run from this checkout saves its choices over the working copy, so ask git for
        // the committed file. Outside a git checkout the file on disk is the shipped one.
        let committed = std::process::Command::new("git")
            .args(["show", "HEAD:config/settings.json"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| include_str!("../config/settings.json").to_string());
        let mut w = Vec::new();
        let s = Settings::parse(&committed, &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s, Settings::default());
        assert_eq!(committed, Settings::default().to_text());
    }

    #[test]
    fn values_round_trip() {
        let s = Settings {
            trails: false,
            view: ViewKind::Map,
            buildings_colour: BuildingColour::Zones,
            projection: Projection::PlateCarree,
            frame_rate: FrameLimit::Fps90,
            language: "de".into(),
            body: "mars".into(),
            ..Settings::default()
        };
        assert!(s.to_text().contains("\"buildings_colour\": \"zones\""));
        let mut w = Vec::new();
        assert_eq!(Settings::parse(&s.to_text(), &mut w), s);
        assert!(w.is_empty());
    }

    #[test]
    fn wrong_values_fall_back_one_by_one_with_a_warning() {
        let mut w = Vec::new();
        let s = Settings::parse(
            r#"{"streets": "yes", "trails": false, "projection": "klein", "frame_rate_limit": 60, "view": "map", "language": 7, "extra": [1]}"#,
            &mut w,
        );
        assert!(s.streets, "wrong type: default");
        assert!(!s.trails, "the right ones still count");
        assert_eq!(s.view, ViewKind::Map);
        assert_eq!(s.projection, Projection::EqualEarth);
        assert_eq!(s.frame_rate, FrameLimit::Fps120);
        assert_eq!(s.language, "");
        assert_eq!(w.len(), 4, "{w:?}");
    }

    #[test]
    fn text_that_is_not_an_object_gives_the_defaults() {
        for text in ["", "[1]", "{", "null", "\u{0}\u{1}"] {
            let mut w = Vec::new();
            assert_eq!(Settings::parse(text, &mut w), Settings::default(), "{text:?}");
            assert_eq!(w.len(), 1);
        }
    }

    #[test]
    fn the_folder_is_the_flag_then_a_development_folder_then_the_users() {
        let work = temp("dir");
        let user = Some(PathBuf::from("/home/x/.config/orbis"));
        assert_eq!(config_dir(None, &work, user.clone()), user);
        std::fs::create_dir(work.join("config")).unwrap();
        assert_eq!(config_dir(None, &work, user.clone()), Some(work.join("config")));
        assert_eq!(config_dir(Some(Path::new("/a")), &work, user), Some(PathBuf::from("/a")));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn nothing_is_written_until_a_value_changes_and_only_once() {
        let dir = temp("save");
        let mut store = Store::open(&dir);
        assert!(store.warnings.is_empty());
        assert_eq!(store.save_if_changed(), Ok(false));
        assert!(!dir.join("settings.json").exists());
        store.values.stars = false;
        assert_eq!(store.save_if_changed(), Ok(true));
        assert_eq!(store.save_if_changed(), Ok(false), "no change since");
        assert!(!dir.join("settings.json.tmp").exists());
        let again = Store::open(&dir);
        assert!(!again.values.stars && again.values.streets);
        // A change back is a change too.
        store.values.stars = true;
        assert_eq!(store.save_if_changed(), Ok(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let dir = temp("rest");
        std::fs::write(dir.join("settings.json"), r#"{"from_the_future": {"a": 1}, "streets": false}"#).unwrap();
        let mut store = Store::open(&dir);
        assert!(!store.values.streets && store.warnings.is_empty());
        store.values.trains = true;
        store.save_if_changed().unwrap();
        let text = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(text.contains("from_the_future"), "{text}");
        assert!(text.contains("\"trains\": true") && text.contains("\"streets\": false"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_the_program_starts_with_defaults() {
        let dir = temp("bad");
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        let store = Store::open(&dir);
        assert_eq!(store.values, Settings::default());
        assert_eq!(store.warnings.len(), 1);
        assert!(dir.join("settings.json.bad").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_changes_takes_only_what_differs() {
        let before = Settings::default();
        let live = Settings { streets: false, frame_rate: FrameLimit::Fps90, ..Settings::default() };
        let mut saved = Settings { trails: false, ..Settings::default() };
        assert!(live.copy_changes(&before, &mut saved));
        assert!(!saved.streets && !saved.trails && saved.frame_rate == FrameLimit::Fps90, "a flag-only difference elsewhere is left alone");
        assert!(!before.copy_changes(&before.clone(), &mut saved));
    }
}
