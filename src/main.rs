#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused))]
mod atlas;
mod atlas_nav;
mod blend;
mod body;
mod buildings;
#[cfg(not(target_arch = "wasm32"))]
mod cache;
#[cfg(target_arch = "wasm32")]
#[path = "cache_mem.rs"]
mod cache;
mod clicks;
mod compass;
mod camera;
mod capabilities;
mod fetcher;
mod geocode;
mod gui;
mod energy_panel;
mod readout;
mod help;
mod info;
mod geo;
mod labels;
mod layers;
mod lan;
mod loader;
mod moon;
mod nav;
mod navtrace;
mod options;
mod overlay;
mod pacing;
mod planetary;
mod planets;
mod recorder;
mod render;
mod import;
mod import_draw;
mod places;
mod power_chart;
mod power_feed;
mod eclipse;
mod energy;
mod energy_style;
mod event_feeds;
mod events;
mod flight;
mod pick;
mod ephemeris;
mod search;
mod service_ui;
mod settings;
mod shadows;
mod select;
mod sky;
mod spacemouse;
mod spacemouse_nav;
mod sky_time;
mod servers;
mod service_lib;
mod services;
mod sources;
mod time_dialog;
mod time_slider;
mod vector;
mod wiki;
mod wtime;
mod worker;
mod paths;
#[cfg(target_arch = "wasm32")]
mod web;
mod wikilookup;

use anyhow::{anyhow, bail, Context, Result};
use atlas::{AtlasView, Projection};
use atlas_nav::AtlasNav;
use cache::DiskCache;
use camera::Camera;
use fetcher::{Fetcher, Peers};
use geo::{ecef_to_geodetic, TileKey};
use glam::{DVec2, DVec3};
use layers::{LineKinds, NameKinds};
use labels::{Screen, TextKey, TextWorker, ATLAS_SIZE};
use loader::{Loader, Outcome, VECTOR_MAX_Z};
use nav::{Arrow, Button, Mods, Nav, WHOLE_EARTH};
use pacing::{FrameLimit, Pacer, RateMeter};
use render::{hemisphere_label, LabelInstance, Renderer};
use select::View;
use settings::{Settings, ViewKind};
use sky_time::{Clock, ClockKey};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use crate::wtime::{Duration, Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

/// The data credits. These must remain visible wherever the data is shown.
const CREDITS: &str = "EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH \
(Contains modified Copernicus Sentinel data 2024)\n\
Terrain: © Mapterhorn https://mapterhorn.com/attribution\n\
Map: OpenFreeMap © OpenMapTiles Data from OpenStreetMap";
/// Shown while the stars are displayed.
const STARS_CREDIT: &str = "Stars: HYG database by David Nash, CC BY-SA 4.0";
/// The search panel is no wider than this many characters, and keeps clear of the readout.
/// Imported points that are given a name label at most.
const IMPORT_LABELS: usize = 3000;
const GIB: u64 = 1024 * 1024 * 1024;
const CACHE_LIMIT_GIB: u64 = 10;
/// A long-running program checks its cache's size this often.
const TRIM_EVERY: Duration = Duration::from_secs(3600);
const UPLOADS_PER_FRAME: usize = 12;
/// Label sets for this many vector tiles stay in memory.
/// Room around a label's text when deciding what a click hits, in pixels at scale 1.
const LABEL_PAD: f64 = 4.0;
const LABEL_SETS: usize = 2000;
/// The most labels considered per frame, most important first.
const LABEL_CANDIDATES: usize = 1500;
/// Fading a label in takes the inverse of this, in seconds.
const LABEL_FADE_RATE: f64 = 6.0;
/// A label already on screen gets this much priority, so that labels do not trade places.
const LABEL_STAY_BONUS: i32 = 60;
const USAGE: &str = "\
orbis: a globe viewer

Usage: orbis [options]

  --lat DEG  --lon DEG    view point (default 30, 10)
  --body NAME             the body to view: earth, moon, mars, mercury, ceres or vesta
                          (default earth). The others are smooth, with no roads, labels or buildings
  --distance METRES       distance from the view point (default the whole body)
  --heading DEG           heading, 0 is north up
  --tilt DEG              tilt, 0 is straight down
  --exaggeration N        terrain height factor (default 1)
  --lang CODE             language for names, such as de or en (default from the locale)
  --atlas PROJECTION      show the flat map instead of the globe: equal-earth, mollweide,
                          hammer-aitoff, mercator or plate-carree (key M switches, P cycles)
  --central-meridian DEG  longitude at the middle of the map (default 0, keys [ and ])
  --globe                 show the globe, whatever the saved options say
  --no-roads              start with streets, trails, trains and boundaries hidden (key R)
  --no-streets  --no-trails  --no-trains  --no-boundaries
                          start with just that kind of line hidden
  --no-labels             start with all names hidden (key L)
  --no-cities  --no-names start with just place names, or just other names (peaks, water, streets) hidden
  --no-markers            start with event markers hidden
  --energy                start with the bars of renewable capacity in Germany (W)
  --no-info               start with the information box on click off (clicks look nothing up)
  --config-dir DIR        keep the saved options (settings.json) in DIR. Without it they are in ./config
                          if that exists, else in orbis/ in your config folder. The command line wins
                          over saved options for one run and is not saved; only what you change in the
                          window is. A --screenshot ignores saved options and saves none, unless DIR is given
  --no-buildings          start with 3D buildings hidden (key B)
  --time ISO8601          set the clock, such as 2026-06-21T12:00:00Z (default now, UTC)
  --time-speed N          clock seconds per real second (default 1)
  --no-shadows            start without shadows cast by terrain and buildings (view options, V)
  --no-sun                start with flat, always-lit imagery (key S)
  --no-stars              start with stars, planets and the Moon hidden (key O)
  --no-spacemouse         do not look for a SpaceMouse (spacenavd or USB)
  --spacemouse-sensitivity N  scale SpaceMouse speed (default 1)
  --fps 120|90            frame rate limit (default 120, key F switches)
  --benchmark SECONDS     turn the view in the window, print frame timings and exit
  --record DIR            keep every drawn frame of the window as an image in the new folder DIR, with a log
                          of the view, the time and the input in DIR/frames.jsonl
  --record-limit GIB      keep only the newest images up to this size (default 4)
  --cache-dir PATH        tile cache location
  --cache-limit GB        largest size of the tile cache (default 10)
  --cache-status          print the cache size and server timeouts, then exit
  --offline               send no requests, and use only what is cached
  --search TEXT           fly to the first place found for TEXT, or to the coordinates in it
                          (key Ctrl-F opens the search box in the window; Earth only for names)
  --search-panel          with --screenshot, also show the open search panel with its results
  --geocoder-url URL      the place search service (default OpenStreetMap's Nominatim)
  --places-file PATH      where saved places are kept (default: orbis/places.json in the config folder)
  --save-place NAME       save the view as a place before anything else (key Ctrl-S names the view in
                          the window; K lists, flies to and deletes saved places)
  --places-panel          with --screenshot, also show the list of saved places
  --eclipse DATE          jump to the eclipse nearest DATE (such as 2024-04-08): the clock is set to
                          its greatest moment and paused, and the view flies to its greatest point
                          (a --time or --search given as well decides the moment or the view instead)
                          (key E lists the eclipses around the clock's date)
  --eclipses-panel        with --screenshot, also show the list of eclipses
  --events-panel          with --screenshot, also show the events panel (key E), which asks the four feeds
  --event-feed NAME       ask one feed for events around the clock's date (quakes, eruptions, tsunamis or
                          fireballs) and mark them on the globe; nothing is requested without this or key E
  --import FILE           show the places, tracks and outlines in a GeoJSON, GPX, KML, KMZ or IGC file
                          (repeatable; files can also be dropped on the window; key I lists them;
                          Earth only)
  --imports-panel         with --screenshot, also show the list of imported items
  --add-service ADDRESS   add a map service to your library and exit: an XYZ address with {z}/{x}/{y}
                          (or {-y}), or the address of a WMTS or WMS service (services are shown
                          on Earth only). Tiles are fetched only when the service is chosen
  --service-name TEXT     with --add-service: the name to show
  --service-credit TEXT   with --add-service: the credit to show while the service is in view
                          (default: what the service asks for, else its host name)
  --service-layer NAME    with --add-service: the layer of a WMTS or WMS service (default: the first usable)
  --service-tms           with --add-service: the XYZ address counts rows from the south
  --service-max-zoom N    with --add-service: ask for no finer tiles than this (default 19 for XYZ)
  --services-file PATH    where the library is kept (default: orbis/services.json in the config folder)
  --list-services         list your library and the built-in directory, then exit
  --service NAME          use this map service instead of the usual Earth imagery
                          (key G lists services, picks the base layer and overlay)
  --service-overlay NAME  lay this map service over the imagery
  --service-opacity N     opacity of the overlay, 0 to 1 (default 0.6; keys + and - in the list)
  --services-panel        with --screenshot, also show the list of map services
  --show-help             with --screenshot, also show the help window (key H)
  --show-options          with --screenshot, also show the view options dialog (key V)
  --show-energy-style     with --screenshot, also show the energy display window (colours and size of the bars)
  --show-time             with --screenshot, also show the time dialog (key C)
  --help-scroll N         with --screenshot, scroll the help window down by N wheel notches (implies --show-help)
  --simulate-click X,Y    with --screenshot, click there (pixels from the top left) before the image is taken,
                          through the same code as a real click; repeatable, in order
  --simulate-drag X1,Y1,X2,Y2  with --screenshot, press at the first point, move to the second in steps and let go,
                          through the same code as the mouse (to drag a slider of the time dialog); repeatable,
                          done after the clicks
  --simulate-double-click X,Y  likewise a double click: flies there smoothly, and the image is taken at the
                          end of the flight (a label under the point is flown to, else the ground)
  --flight-stop-at T      with --simulate-double-click, take the image T seconds into the flight instead
  --simulate-nav SCRIPT   with --screenshot, drive the view from the start, through the window's wheel
                          routing, and print what is drawn after each step (a diagnostic). Steps, comma
                          separated: pX:Y pointer, z+N wheel, a+N Alt-wheel (tilt), fN frames without
                          waiting for tiles, s wait until loaded, l log, d list tiles, g:NAME picture
  --nav-uploads N         with --simulate-nav, take only N finished tiles from the loader per frame
                          (default 12), so that tiles arrive slowly even from a warm cache
  --nav-draw-max-z Z      with --simulate-nav, draw no tile finer than Z, though they still load and give the
                          ground height: the moments when finer tiles have come for the camera but not the view
  --mark-underlay         with --screenshot, paint the base sphere under the tiles magenta, so holes show
  --nav-shots DIR         where g:NAME steps of --simulate-nav save NAME.png
  --serve-cache           run as a LAN cache server, with no window
  --share-cache           also offer this cache on the LAN, and use others found there
  --cache-server HOST:PORT  use this LAN cache server (repeatable)
  --cache-port PORT       port to serve on (default 47813)
  --screenshot FILE.png   render once all tiles arrive, write the image and exit
  --size WxH              window size (default 1280x800), or screenshot size (default 1600x1000)
  --timeout SECONDS       longest wait for tiles in a screenshot (default 90)
  --help                  show this text

See docs/FEATURES.md for the mouse and keyboard controls.";

struct Args {
    cam: Camera,
    /// The body to view; chosen once for the run.
    body: &'static body::Body,
    distance_given: bool,
    /// The saved options, with what the command line chose put over them.
    settings: Settings,
    /// Where `settings.json` is read and written; `None` keeps the options for this run only.
    config_dir: Option<PathBuf>,
    /// `--config-dir`, as given.
    config_flag: Option<PathBuf>,
    central_meridian: f64,
    exaggeration: f32,
    cache_dir: PathBuf,
    screenshot: Option<PathBuf>,
    size: (u32, u32),
    size_given: bool,
    timeout: f64,
    lang: String,
    /// The clock's start, as seconds since 1970 UTC; now when not given.
    time: Option<f64>,
    time_speed: f64,
    spacemouse: bool,
    spacemouse_sensitivity: f64,
    benchmark: Option<f64>,
    /// `--record`: the folder for this run's frames.
    record: Option<PathBuf>,
    record_limit_gib: f64,
    cache_limit_gib: u64,
    cache_status: bool,
    offline: bool,
    serve_cache: bool,
    share_cache: bool,
    cache_servers: Vec<String>,
    cache_port: u16,
    /// Fly to the first result for this text, or the coordinates in it, before the picture is saved.
    search: Option<String>,
    /// Show the open search panel with its results in the saved picture.
    search_panel: bool,
    places_file: Option<PathBuf>,
    save_place: Option<String>,
    places_panel: bool,
    /// Jump to the eclipse nearest this time.
    eclipse: Option<f64>,
    eclipses_panel: bool,
    events_panel: bool,
    event_feed: Option<event_feeds::Feed>,
    imports: Vec<PathBuf>,
    imports_panel: bool,
    geocoder_url: String,
    services: service_ui::Cli,
    show_help: bool,
    show_options: bool,
    show_energy_style: bool,
    show_time: bool,
    simulate_drags: Vec<(DVec2, DVec2)>,
    help_scroll: f64,
    simulate_clicks: Vec<DVec2>,
    simulate_doubles: Vec<DVec2>,
    simulate_nav: Option<Vec<navtrace::Step>>,
    nav_shots: Option<PathBuf>,
    mark_underlay: bool,
    nav_uploads: usize,
    nav_draw_max_z: u8,
    flight_stop_at: Option<f64>,
}

#[cfg(target_arch = "wasm32")]
fn locale_language() -> String {
    // The browser's language, such as "de-AT", read like a locale.
    web_sys::window()
        .and_then(|w| w.navigator().language())
        .map(|l| vector::language_from_locale(&l.replace('-', "_")))
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn locale_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| vector::language_from_locale(&value))
        .find(|lang| !lang.is_empty())
        .unwrap_or_default()
}

/// Where the options are kept for this run, found before the rest of the command line is read:
/// a picture ignores them unless the folder is given, so that images are the same on every machine.
fn settings_dir(argv: &[String]) -> (Option<PathBuf>, Option<PathBuf>) {
    let flag = argv.iter().position(|a| a == "--config-dir").and_then(|i| argv.get(i + 1)).map(PathBuf::from);
    let picture = argv.iter().any(|a| a == "--screenshot");
    if picture && flag.is_none() {
        return (None, None);
    }
    let work = std::env::current_dir().unwrap_or_default();
    (settings::config_dir(flag.as_deref(), &work, settings::user_config_dir()), flag)
}

fn parse_args(mut it: impl Iterator<Item = String>, saved: &Settings) -> Result<Option<Args>> {
    let mut a = Args {
        settings: saved.clone(),
        config_dir: None,
        config_flag: None,
        body: body::by_id(&saved.body).unwrap_or(&body::EARTH),
        distance_given: false,
        cam: Camera {
            lat: 30f64.to_radians(),
            lon: 10f64.to_radians(),
            target_h: 0.0,
            distance: WHOLE_EARTH,
            heading: 0.0,
            tilt: 0.0,
            sky: 0.0,
        },
        central_meridian: 0.0,
        exaggeration: 1.0,
        cache_dir: DiskCache::default_root(),
        screenshot: None,
        size: (1600, 1000),
        size_given: false,
        timeout: 90.0,
        lang: Some(saved.language.clone()).filter(|l| !l.is_empty()).unwrap_or_else(locale_language),
        time: None,
        time_speed: 1.0,
        spacemouse: true,
        spacemouse_sensitivity: 1.0,
        benchmark: None,
        record: None,
        record_limit_gib: 4.0,
        cache_limit_gib: CACHE_LIMIT_GIB,
        cache_status: false,
        offline: false,
        serve_cache: false,
        share_cache: false,
        cache_servers: Vec::new(),
        cache_port: lan::TCP_PORT,
        search: None,
        search_panel: false,
        places_file: None,
        save_place: None,
        places_panel: false,
        eclipse: None,
        eclipses_panel: false,
        events_panel: false,
        event_feed: None,
        imports: Vec::new(),
        imports_panel: false,
        services: Default::default(),
        show_help: false,
        show_options: false,
        show_energy_style: false,
        show_time: false,
        simulate_drags: Vec::new(),
        help_scroll: 0.0,
        simulate_clicks: Vec::new(),
        simulate_doubles: Vec::new(),
        simulate_nav: None,
        nav_shots: None,
        mark_underlay: false,
        nav_uploads: UPLOADS_PER_FRAME,
        nav_draw_max_z: u8::MAX,
        flight_stop_at: None,
        geocoder_url: geocode::DEFAULT_SERVICE.to_string(),
    };
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--help" | "-h" => return Ok(None),
            "--no-roads" => {
                let s = &mut a.settings;
                (s.streets, s.trails, s.trains, s.boundaries) = (false, false, false, false);
                continue;
            }
            "--no-labels" => {
                (a.settings.cities, a.settings.other_names) = (false, false);
                continue;
            }
            "--no-streets" => {
                a.settings.streets = false;
                continue;
            }
            "--no-trails" => {
                a.settings.trails = false;
                continue;
            }
            "--no-trains" => {
                a.settings.trains = false;
                continue;
            }
            "--mark-underlay" => {
                a.mark_underlay = true;
                continue;
            }
            "--no-boundaries" => {
                a.settings.boundaries = false;
                continue;
            }
            "--no-cities" => {
                a.settings.cities = false;
                continue;
            }
            "--no-names" => {
                a.settings.other_names = false;
                continue;
            }
            "--no-markers" => {
                a.settings.event_markers = false;
                continue;
            }
            "--energy" => {
                a.settings.energy = true;
                continue;
            }
            "--no-info" => {
                a.settings.info_on_click = false;
                continue;
            }
            "--globe" => {
                a.settings.view = ViewKind::Globe;
                continue;
            }
            "--no-buildings" => {
                a.settings.buildings = false;
                continue;
            }
            "--no-stars" => {
                a.settings.stars = false;
                continue;
            }
            "--no-spacemouse" => {
                a.spacemouse = false;
                continue;
            }
            "--no-shadows" => {
                a.settings.cast_shadows = false;
                continue;
            }
            "--no-sun" => {
                a.settings.sunlight = false;
                continue;
            }
            "--cache-status" => {
                a.cache_status = true;
                continue;
            }
            "--offline" => {
                a.offline = true;
                continue;
            }
            "--search-panel" => {
                a.search_panel = true;
                continue;
            }
            "--places-panel" => {
                a.places_panel = true;
                continue;
            }
            "--events-panel" => {
                a.events_panel = true;
                continue;
            }
            "--eclipses-panel" => {
                a.eclipses_panel = true;
                continue;
            }
            "--imports-panel" => {
                a.imports_panel = true;
                continue;
            }
            "--service-tms" => {
                a.services.tms = true;
                continue;
            }
            "--show-help" => {
                a.show_help = true;
                continue;
            }
            "--show-options" => {
                a.show_options = true;
                continue;
            }
            "--show-energy-style" => {
                a.show_energy_style = true;
                continue;
            }
            "--show-time" => {
                a.show_time = true;
                continue;
            }
            "--services-panel" => {
                a.services.panel = true;
                continue;
            }
            "--list-services" => {
                a.services.list = true;
                continue;
            }
            "--serve-cache" => {
                a.serve_cache = true;
                continue;
            }
            "--share-cache" => {
                a.share_cache = true;
                continue;
            }
            _ => {}
        }
        let value = it.next().ok_or_else(|| anyhow!("{flag} needs a value"))?;
        let num = || -> Result<f64> {
            value.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| anyhow!("{flag}: not a number: {value}"))
        };
        match flag.as_str() {
            "--lat" => a.cam.lat = num()?.clamp(-89.9, 89.9).to_radians(),
            "--lon" => a.cam.lon = num()?.to_radians(),
            "--distance" => {
                a.cam.distance = num()?;
                a.distance_given = true;
            }
            "--body" => {
                a.body = body::by_id(&value)
                    .ok_or_else(|| anyhow!("--body expects one of: {}", body::ids().join(", ")))?;
                a.settings.body = a.body.id.to_string();
            }
            "--heading" => a.cam.heading = num()?.to_radians(),
            "--tilt" => a.cam.set_total_tilt(num()?.to_radians()),
            "--time" => {
                a.time = Some(sky_time::parse_iso8601(&value).ok_or_else(|| {
                    anyhow!("--time expects a date and time such as 2026-06-21T12:00:00Z")
                })?)
            }
            "--event-feed" => {
                a.event_feed = Some(event_feeds::Feed::from_name(&value).ok_or_else(|| anyhow!("--event-feed expects quakes, eruptions, tsunamis or fireballs"))?)
            }
            "--eclipse" => {
                a.eclipse = Some(sky_time::parse_iso8601(&value).ok_or_else(|| anyhow!("--eclipse expects a date such as 2024-04-08"))?)
            }
            "--time-speed" => a.time_speed = num()?,
            "--spacemouse-sensitivity" => {
                let n = num()?;
                if !(n.is_finite() && n > 0.0) {
                    bail!("--spacemouse-sensitivity expects a number above 0");
                }
                a.spacemouse_sensitivity = n.min(20.0);
            }
            "--exaggeration" => a.exaggeration = num()? as f32,
            "--atlas" => {
                a.settings.projection = Projection::parse(&value).ok_or_else(|| {
                    anyhow!("--atlas expects equal-earth, mollweide, hammer-aitoff, mercator or plate-carree")
                })?;
                a.settings.view = ViewKind::Map;
            }
            "--central-meridian" => a.central_meridian = atlas::wrap_lon(num()?.to_radians()),
            "--timeout" => a.timeout = num()?,
            "--cache-limit" => {
                a.cache_limit_gib = value.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                    anyhow!("--cache-limit expects a whole number of gigabytes")
                })?;
            }
            "--cache-server" => {
                let valid = value.rsplit_once(':').is_some_and(|(host, port)| {
                    !host.is_empty()
                        && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-[]:".contains(c))
                        && port.parse::<u16>().is_ok_and(|p| p != 0)
                });
                if !valid {
                    bail!("--cache-server expects HOST:PORT");
                }
                a.cache_servers.push(value.clone());
            }
            "--cache-port" => {
                a.cache_port = value.parse().ok().filter(|p| *p != 0).ok_or_else(|| {
                    anyhow!("--cache-port expects a port number")
                })?;
            }
            "--fps" => {
                a.settings.frame_rate = FrameLimit::parse(&value).ok_or_else(|| anyhow!("--fps expects 120 or 90"))?;
            }
            "--benchmark" => {
                let seconds = num()?;
                if seconds <= 0.0 {
                    bail!("--benchmark expects a positive number of seconds");
                }
                a.benchmark = Some(seconds);
            }
            "--record" => a.record = Some(value.into()),
            "--record-limit" => {
                a.record_limit_gib = num()?;
                if !(a.record_limit_gib > 0.0) {
                    bail!("--record-limit expects a positive number of GiB");
                }
            }
            "--lang" => {
                if !value.chars().all(|c| c.is_ascii_alphabetic()) {
                    bail!("--lang expects a language code such as de");
                }
                a.lang = value.to_ascii_lowercase();
            }
            "--search" => a.search = Some(value.clone()).filter(|v| !v.trim().is_empty()),
            "--import" => a.imports.push(value.into()),
            "--places-file" => a.places_file = Some(value.into()),
            "--config-dir" => a.config_flag = Some(value.into()),
            "--add-service" => a.services.add = Some(value.clone()),
            "--service-name" => a.services.name = Some(value.clone()),
            "--service-credit" => a.services.credit = Some(value.clone()),
            "--service-layer" => a.services.layer = Some(value.clone()),
            "--service-max-zoom" => {
                a.services.max_zoom = Some(value.parse().ok().filter(|z| *z <= sources::ABSOLUTE_MAX_Z).ok_or_else(|| anyhow!("--service-max-zoom expects a zoom level from 0 to {}", sources::ABSOLUTE_MAX_Z))?);
            }
            "--services-file" => a.services.file = Some(value.into()),
            "--service" => a.services.base = Some(value.clone()),
            "--service-overlay" => a.services.overlay = Some(value.clone()),
            "--service-opacity" => {
                let v = num()?;
                if !(0.0..=1.0).contains(&v) {
                    bail!("--service-opacity expects a number from 0 to 1");
                }
                a.services.opacity = Some(v as f32);
            }
            "--help-scroll" => {
                a.help_scroll = num()?;
                a.show_help = true;
            }
            "--simulate-double-click" => {
                let (x, y) = value.split_once(',').ok_or_else(|| anyhow!("--simulate-double-click expects X,Y"))?;
                let n = |v: &str| v.trim().parse::<f64>().context("--simulate-double-click expects X,Y");
                a.simulate_doubles.push(DVec2::new(n(x)?, n(y)?));
            }
            "--simulate-nav" => a.simulate_nav = Some(navtrace::parse(&value)?),
            "--nav-uploads" => a.nav_uploads = value.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(|| anyhow!("--nav-uploads expects a positive number"))?,
            "--nav-draw-max-z" => a.nav_draw_max_z = value.parse().context("--nav-draw-max-z expects a zoom level")?,
            "--nav-shots" => a.nav_shots = Some(value.into()),
            "--flight-stop-at" => a.flight_stop_at = Some(value.parse().context("--flight-stop-at expects seconds")?),
            "--simulate-click" => {
                let (x, y) = value.split_once(',').ok_or_else(|| anyhow!("--simulate-click expects X,Y"))?;
                let n = |v: &str| v.trim().parse::<f64>().context("--simulate-click expects X,Y");
                a.simulate_clicks.push(DVec2::new(n(x)?, n(y)?));
            }
            "--simulate-drag" => {
                let n: Vec<f64> = value.split(',').map(|v| v.trim().parse::<f64>()).collect::<Result<_, _>>().context("--simulate-drag expects X1,Y1,X2,Y2")?;
                let [x1, y1, x2, y2] = n[..] else { bail!("--simulate-drag expects X1,Y1,X2,Y2") };
                a.simulate_drags.push((DVec2::new(x1, y1), DVec2::new(x2, y2)));
            }
            "--save-place" => a.save_place = Some(value.clone()),
            "--geocoder-url" => {
                if !value.starts_with("http://") && !value.starts_with("https://") {
                    bail!("--geocoder-url expects an http or https address");
                }
                a.geocoder_url = value.clone();
            }
            "--cache-dir" => a.cache_dir = value.into(),
            "--screenshot" => a.screenshot = Some(value.into()),
            "--size" => {
                let (w, h) = value.split_once('x').ok_or_else(|| anyhow!("--size expects WxH"))?;
                a.size = (w.parse().context("--size width")?, h.parse().context("--size height")?);
                if a.size.0 == 0 || a.size.1 == 0 {
                    bail!("--size must not be zero");
                }
                a.size_given = true;
            }
            _ => bail!("unknown option {flag}\n\n{USAGE}"),
        }
    }
    a.cam.clamp();
    Ok(Some(a))
}

/// Everything between input and drawing that does not depend on a window.
/// What the window shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Globe,
    Atlas,
}

struct Scene {
    nav: Nav,
    /// Finished tiles taken from the loader per frame.
    uploads_per_frame: usize,
    /// Tiles finer than this are loaded but not drawn (`--nav-draw-max-z`).
    draw_max_z: u8,
    mode: Mode,
    map: AtlasNav,
    loader: Loader,
    /// When a failed tile may be tried again, and how many times it has failed.
    retries: HashMap<TileKey, (Instant, u32)>,
    missing: HashSet<TileKey>,
    draw: Vec<TileKey>,
    /// The shadow plan last used with the tiles that cast into it, and what they were chosen for.
    shadow_cast: Option<(shadows::Plan, u64, usize, Vec<TileKey>)>,
    /// Tiles and overlays the view needs that are not loaded yet.
    waiting: usize,
    /// How many of those have failed before.
    retrying: usize,
    /// Tiles whose map data contains no lines, so they need no overlay.
    overlay_empty: HashSet<TileKey>,
    overlay_retries: HashMap<TileKey, (Instant, u32)>,
    /// Labels by the vector tile they came from.
    label_sets: HashMap<TileKey, Arc<Vec<vector::Label>>>,
    text: TextWorker,
    /// How visible each label on screen is, as it fades in.
    label_alpha: HashMap<u64, f64>,
    /// The labels as placed in the last frame, for finding what a click lands on.
    placed_labels: Vec<pick::PlacedLabel>,
    labels_shown: usize,
    /// Tiles known to have no buildings.
    building_empty: HashSet<TileKey>,
    building_retries: HashMap<TileKey, (Instant, u32)>,
    /// A line about the frame rate for the on-screen display, when drawing to a window.
    frame_status: Option<String>,
    clock: Clock,
    /// Real time for the clock is counted from here.
    clock_epoch: Instant,
    /// The clock reading when the picture was last drawn.
    drawn_time: f64,
    /// Stars, planets and the Moon are shown.
    /// What the user sees chosen: the saved options with the command line over them, and as changed
    /// since. `view_last` is what they were when `store` was last told.
    view: Settings,
    view_last: Settings,
    store: settings::Store,
    /// The kinds of line the overlays were drawn with, and how many times that changed.
    lines_drawn: LineKinds,
    lines_version: u64,
    /// What the view options dialog shows; filled from the live state before each frame.
    options: options::Shared,
    energy_style: energy_style::Shared,
    time_model: time_dialog::Shared,
    eclipse_watch: time_slider::EclipseWatch,
    /// The planets and the Moon as of `bodies_time`.
    bodies: Vec<planets::Body>,
    bodies_time: f64,
    bodies_pos: DVec3,
    fetcher: Arc<Fetcher>,
    search: search::Search,
    places: places::Store,
    place_list: places::Picker,
    services: service_ui::Control,
    /// The interface: windows, icons, the readout and the credits.
    gui: gui::Gui,
    /// The size (pixels) and pixels per point the interface was last built for.
    gui_size: (u32, u32, f32),
    /// The pointer was over the interface at its last move.
    gui_hover: bool,
    /// The technical figures are shown (F12).
    debug_readout: bool,
    /// Where the help window is scrolled to, in points, while a saved image asks for it.
    help_scroll: Option<f32>,
    /// The credits for what is drawn, and how many tiles the renderer holds, as of the last frame.
    credits_text: String,
    tiles_loaded: usize,
    clicks: clicks::ClickClassifier,
    started: Instant,
    pointer: DVec2,
    /// Buttons (bit 0 left, bit 1 right) that went down on the map, not on the interface.
    map_buttons: u8,
    /// The choice of map services the drawn tiles were made under.
    services_seen: u64,
    library: import::Library,
    /// Names of imported points, placed like map labels.
    import_labels: Arc<Vec<vector::Label>>,
    import_list: places::Picker,
    /// Eclipses (and later other events) around the clock's date, and the central line shown.
    events: events::EventList,
    feeds: event_feeds::Feeds,
    /// Feeds asked and not yet answered, and what the answered ones said when they had nothing.
    feeds_pending: usize,
    feed_problems: Vec<(event_feeds::Feed, String)>,
    /// The markers on the globe, the list's revision and the events they were made from.
    markers: Option<Arc<import::Imported>>,
    markers_version: u64,
    markers_from: (u64, Vec<usize>),
    marker_credits: Vec<&'static str>,
    eclipse_path: Option<Arc<import::Imported>>,
    eclipse_path_version: u64,
    /// Something to tell the user, shown until the next list is opened or Escape is pressed.
    notice: Option<String>,
    /// The renderer is to drop its overlays at the next update.
    clear_overlays: bool,
    geocoder: geocode::Geocoder,
    /// The information box: what it shows, the thread that asks, and the number of the question it waits for.
    info: info::Shared,
    lookup: wikilookup::Lookup,
    info_id: u64,
    info_pending: bool,
    /// The box was opened or closed by a click that no event of the window announced.
    info_changed: bool,
    /// Keeps a slow tick going for tiles waiting out a retry delay.
    last_retry_tick: Instant,
    energy: EnergyLayer,
}

/// What the user did in one frame of the interface, carried out once it is built.
#[derive(Default)]
struct GuiDone {
    icon: Option<compass::Clicked>,
    options: Option<options::Action>,
    time: Vec<time_dialog::Action>,
    style: Option<energy_style::Action>,
    chart: Option<power_chart::Event>,
    info: (bool, Option<info::Action>),
    energy: Option<energy_panel::Action>,
}

type OutlineKey = (energy::Level, [bool; energy::SOURCES], Option<energy::Subject>, Option<u16>);

/// Level, month, sources, centre, tile count, and the quarter hour of the output shown.
type EnergyBuild = (energy::Level, u16, [bool; energy::SOURCES], (i64, i64), usize, Option<i64>);

/// Generation and consumption from SMARD: what has arrived and what was asked.
#[derive(Default)]
struct PowerState {
    feed: Option<power_feed::PowerFeed>,
    index: Option<Vec<i64>>,
    index_asked: Option<Instant>,
    weeks: HashMap<i64, power_feed::Week>,
    asked: HashMap<i64, Instant>,
    problem: Option<String>,
    /// The quarter hour shown, and the shares of capacity running then.
    shown: Option<(power_feed::Moment, [f32; energy::SOURCES])>,
}

/// What the energy bars show now, and what they were last built from.
#[derive(Default)]
struct EnergyLayer {
    level: Option<energy::Level>,
    selected: Option<energy::Subject>,
    /// The areas outlined on the ground, and what they were made for.
    outlines: Option<Arc<import::Imported>>,
    /// Level, sources, choice, and the month where the ground's shade follows it (close up).
    outlines_from: Option<OutlineKey>,
    version: u64,
    bars: Vec<energy::Bar>,
    scale: Option<energy::Scale>,
    /// The level, month, sources, centre and tile count the mesh was built for, and when.
    built_from: Option<EnergyBuild>,
    built_at: Option<Instant>,
    /// The view's aspect and height in pixels, for finding the bar under a click.
    view: (f64, f64),
    /// The data could not be read: said once.
    missing: bool,
    power: PowerState,
    /// What the day chart shows.
    chart: power_chart::Shared,
    /// The look the bars and outlines were built with; a change rebuilds them.
    style: Option<energy::Style>,
}

/// Sets up tile fetching, including sharing on the LAN if asked for.
fn make_fetcher(args: &Args) -> Arc<Fetcher> {
    let cache = DiskCache::new(args.cache_dir.clone());
    // In a browser the cache is in memory, and keeps itself within bounds.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let trimmer = cache.clone();
        let limit = args.cache_limit_gib * GIB;
        std::thread::spawn(move || loop {
            trimmer.trim(limit);
            std::thread::sleep(TRIM_EVERY);
        });
    }
    let peers = Peers::new(args.cache_servers.clone());
    Arc::new(Fetcher::new(cache, peers, args.offline))
}

/// Opens the LAN port, and announces this machine there.
fn start_sharing(fetcher: &Arc<Fetcher>, args: &Args, mode: lan::Mode) -> Result<()> {
    let listener = std::net::TcpListener::bind(("0.0.0.0", args.cache_port))
        .with_context(|| format!("could not open port {}", args.cache_port))?;
    lan::serve(listener, fetcher.clone(), mode);
    let id = lan::instance_id();
    lan::announce(id, args.cache_port, mode);
    // A dedicated server fetches for others and asks nobody itself.
    let looking = (mode == lan::Mode::SharedApp).then(|| fetcher.clone());
    lan::discover(id, Some((args.cache_port, mode)), looking);
    Ok(())
}

/// Runs as a LAN cache server until stopped.
fn serve_cache(args: &Args) -> Result<()> {
    let fetcher = make_fetcher(args);
    start_sharing(&fetcher, args, lan::Mode::Dedicated)?;
    println!("Serving the tile cache on port {}.", args.cache_port);
    println!("  Tiles:  http://THIS-MACHINE:{}/tiles/SOURCE/Z/X/Y", args.cache_port);
    println!("  Status: http://THIS-MACHINE:{}/status", args.cache_port);
    println!("  Sources: {}", sources::all().map(|s| s.id).collect::<Vec<_>>().join(", "));
    println!("Credit the data when displaying it; see docs/PROVENANCE.md. Press Ctrl-C to stop.");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB as f64 {
        format!("{:.1} GB", b / GIB as f64)
    } else {
        format!("{:.1} MB", b / MIB)
    }
}

/// How long until a time, in whole minutes, rounding up.
fn minutes_until(until: SystemTime, now: SystemTime) -> u64 {
    until.duration_since(now).map_or(0, |d| d.as_secs().div_ceil(60))
}

/// Prints what the cache holds and which servers are being left alone.
fn cache_status(args: &Args) {
    let cache = DiskCache::new(args.cache_dir.clone());
    let (entries, bytes) = cache.usage();
    println!("Cache: {}", args.cache_dir.display());
    println!("  {entries} tiles, {} of {} GB", format_bytes(bytes), args.cache_limit_gib);
    let servers = servers::Servers::new(Some(cache.state_path("servers.txt")));
    let now = SystemTime::now();
    let paused: Vec<_> = servers.report(now).into_iter().filter(|r| r.paused_until.is_some()).collect();
    if paused.is_empty() {
        println!("  No servers are timed out.");
    }
    for r in paused {
        let until = r.paused_until.unwrap_or(now);
        println!("  {} is timed out for {} more minutes", r.host, minutes_until(until, now));
    }
}

fn vector_source(k: TileKey) -> TileKey {
    k.ancestor(k.z.min(VECTOR_MAX_Z))
}

/// The building tiles covering the drawn tiles. Height data only exists at one zoom level,
/// so finer tiles share the mesh of the tile they lie in, and a tile one level coarser uses
/// its four children. That moves the line where buildings end further into the distance.
fn building_tiles(draw: &[TileKey]) -> Vec<TileKey> {
    let mut keys: Vec<TileKey> = Vec::new();
    for k in draw {
        if k.z >= vector::BUILDING_Z {
            keys.push(k.ancestor(vector::BUILDING_Z));
        } else if k.z + 1 == vector::BUILDING_Z {
            keys.extend(k.children());
        }
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

fn label_id(label: &vector::Label) -> u64 {
    let mut h = DefaultHasher::new();
    label.text.hash(&mut h);
    // Coarse enough that the same place from a neighbouring zoom level counts as the same.
    ((label.lat * 2000.0) as i64, (label.lon * 2000.0) as i64).hash(&mut h);
    h.finish()
}

/// Schedules another attempt: when a server said so, or else after a growing delay.
fn next_retry(retries: &mut HashMap<TileKey, (Instant, u32)>, k: TileKey, until: Option<SystemTime>) {
    let n = retries.get(&k).map_or(1, |e| e.1 + 1);
    let told = until.and_then(|u| u.duration_since(SystemTime::now()).ok());
    let delay = told.unwrap_or(Duration::from_secs(2u64.saturating_pow(n.min(8))));
    retries.insert(k, (Instant::now() + delay, n));
}

/// The credits for what is on screen: those always shown, then any aerial imagery in view.
fn credits_for<'a>(detail: impl Iterator<Item = &'a str>) -> String {
    let credits = body::current().credit;
    // Map services' credits start with a mark and are listed apart from the aerial images.
    let parts: BTreeSet<&str> = detail.flat_map(|c| c.split(service_lib::CREDIT_SEP)).filter(|c| !c.is_empty()).collect();
    let (services, names): (Vec<&str>, Vec<&str>) = parts.into_iter().partition(|c| c.starts_with(service_lib::CREDIT_MARK));
    let mut out = credits.to_string();
    if !names.is_empty() {
        out.push_str(&format!("\nAerial images: {}", names.join(", ")));
    }
    if !services.is_empty() {
        let list: Vec<&str> = services.iter().map(|c| c.trim_start_matches(service_lib::CREDIT_MARK)).collect();
        out.push_str(&format!("\nMap service: {}", list.join(", ")));
    }
    out
}

/// Says whether more is coming: a finished view and a stuck one otherwise look the same.
fn loading_status(waiting: usize, retrying: usize) -> String {
    match (waiting.saturating_sub(retrying), retrying) {
        (0, 0) => "all loaded".to_string(),
        (loading, 0) => format!("loading {loading}"),
        (0, retrying) => format!("{retrying} waiting to retry"),
        (loading, retrying) => format!("loading {loading}, {retrying} waiting to retry"),
    }
}

/// The on-screen line about traffic, and a line for each server being left alone.
fn network_status(fetcher: &Fetcher, now: SystemTime) -> Vec<String> {
    let t = fetcher.traffic();
    let total = t.disk_hits + t.lan_hits + t.downloads;
    let mut lines = Vec::new();
    if let Some(saved) = ((t.disk_hits + t.lan_hits) * 100).checked_div(total) {
        let lan = if t.lan_hits > 0 { format!("  LAN {}", t.lan_hits) } else { String::new() };
        lines.push(format!("downloaded {}  cached {saved}%{lan}", format_bytes(t.downloaded_bytes)));
    }
    for r in fetcher.server_report() {
        if let Some(until) = r.paused_until {
            lines.push(format!("timed out: {} {} min", r.host, minutes_until(until, now)));
        }
    }
    lines
}

/// The map service list, with what the command line chose already in use.
fn make_services(args: &Args) -> service_ui::Control {
    let mut c = service_ui::Control::new(args.services.library(), args.body.surface_data);
    if let Err(e) = c.choose_at_start(args.services.base.as_deref(), args.services.overlay.as_deref(), args.services.opacity) {
        eprintln!("{e}");
    }
    c
}

impl Scene {
    fn new(args: &Args, size: DVec2) -> Self {
        let fetcher = make_fetcher(args);
        if args.share_cache {
            // The app is useful without sharing, for example if the port is taken.
            if let Err(e) = start_sharing(&fetcher, args, lan::Mode::SharedApp) {
                eprintln!("cache sharing is off: {e:#}");
            }
        }
        let places = match args.places_file.clone().or_else(places::default_path) {
            Some(path) => places::Store::load(path),
            None => places::Store::in_memory(),
        };
        let mut view = AtlasView::whole(args.settings.projection, args.central_meridian, (size.x, size.y));
        view.match_globe(args.cam.lat, args.cam.lon, args.cam.distance);
        let mut scene = Self {
            nav: Nav::starting_above_ground(args.cam, size),
            uploads_per_frame: args.nav_uploads,
            draw_max_z: args.nav_draw_max_z,
            mode: if args.settings.view == ViewKind::Map { Mode::Atlas } else { Mode::Globe },
            map: AtlasNav::new(view),
            loader: Loader::new(fetcher.clone(), args.exaggeration, vector::name_fields(&args.lang)),
            search: search::Search::new(args.body.surface_data),
            notice: places.notice.clone(),
            places,
            place_list: places::Picker::default(),
            services: make_services(args),
            gui: gui::Gui::new(),
            gui_size: (size.x.max(1.0) as u32, size.y.max(1.0) as u32, 1.0),
            gui_hover: false,
            debug_readout: false,
            help_scroll: None,
            credits_text: String::new(),
            tiles_loaded: 0,
            clicks: clicks::ClickClassifier::default(),
            started: Instant::now(),
            pointer: DVec2::ZERO,
            map_buttons: 0,
            services_seen: service_lib::active().generation,
            clear_overlays: false,
            library: import::Library::default(),
            import_labels: Arc::default(),
            import_list: places::Picker::default(),
            events: events::EventList::default(),
            feeds: event_feeds::Feeds::new(fetcher.clone()),
            feeds_pending: 0,
            feed_problems: Vec::new(),
            markers: None,
            markers_version: 0,
            markers_from: (u64::MAX, Vec::new()),
            marker_credits: Vec::new(),
            eclipse_path: None,
            eclipse_path_version: 0,
            geocoder: geocode::Geocoder::new(fetcher.clone(), args.geocoder_url.clone(), args.lang.clone()),
            info: info::Shared::default(),
            lookup: wikilookup::Lookup::new(fetcher.clone(), &args.lang),
            info_id: 0,
            info_pending: false,
            info_changed: false,
            fetcher,
            last_retry_tick: Instant::now(),
            retries: HashMap::new(),
            missing: Default::default(),
            draw: Vec::new(),
            shadow_cast: None,
            waiting: 0,
            retrying: 0,
            overlay_empty: HashSet::new(),
            overlay_retries: HashMap::new(),
            label_sets: HashMap::new(),
            text: TextWorker::new(),
            label_alpha: HashMap::new(),
            placed_labels: Vec::new(),
            labels_shown: 0,
            building_empty: HashSet::new(),
            building_retries: HashMap::new(),
            frame_status: None,
            clock: {
                let mut clock = Clock::new(args.time.unwrap_or_else(sky_time::wall_clock_now), 0.0);
                clock.set_rate(args.time_speed);
                // A saved image shows one moment.
                if args.screenshot.is_some() {
                    clock.pause();
                }
                clock
            },
            clock_epoch: Instant::now(),
            drawn_time: 0.0,
            view: args.settings.clone(),
            view_last: args.settings.clone(),
            store: args.config_dir.as_deref().map_or_else(settings::Store::in_memory, settings::Store::open),
            lines_drawn: LineKinds::of(&args.settings),
            lines_version: 0,
            options: Default::default(),
            energy_style: Default::default(),
            time_model: Default::default(),
            eclipse_watch: Default::default(),
            bodies: Vec::new(),
            bodies_time: f64::NEG_INFINITY,
            bodies_pos: DVec3::ZERO,
            energy: EnergyLayer::default(),
        };
        scene.loader.set_lines(scene.lines_drawn);
        for path in &args.imports {
            scene.import_path(path, false);
        }
        let base = scene.services.base_id().unwrap_or_default().to_string();
        (scene.view.base_service, scene.view_last.base_service) = (base.clone(), base);
        scene
    }

    fn lines_on(&self) -> bool {
        let v = &self.view;
        v.streets || v.trails || v.trains || v.boundaries
    }

    fn names_on(&self) -> bool {
        NameKinds::of(&self.view).any()
    }

    /// Key R: all the lines off, or if they are all off, on.
    fn toggle_lines(&mut self) {
        let on = !self.lines_on();
        let v = &mut self.view;
        (v.streets, v.trails, v.trains, v.boundaries) = (on, on, on, on);
    }

    /// Key L: all the names off, or if they are all off, on.
    fn toggle_names(&mut self) {
        let on = !self.names_on();
        (self.view.cities, self.view.other_names) = (on, on);
    }

    /// The options as they are now: `view`, with what the map and the services hold.
    fn live(&self) -> Settings {
        let mut live = self.view.clone();
        live.view = if self.mode == Mode::Atlas { ViewKind::Map } else { ViewKind::Globe };
        live.projection = self.map.view.projection;
        live.base_service = self.services.base_id().unwrap_or_default().to_string();
        live
    }

    /// Keeps the options dialog true to the state, whatever changed it.
    fn sync_options(&mut self) {
        if !self.gui.is_visible(options::WINDOW) {
            return;
        }
        let model = options::Model {
            settings: self.live(),
            running_body: body::current().id,
            earth: body::current().surface_data,
            library: self.services.library_choices(),
        };
        if *self.options.borrow() != model {
            *self.options.borrow_mut() = model;
        }
    }

    /// Carries out what a click in the options dialog asked for.
    fn options_action(&mut self, action: options::Action) {
        use options::Action;
        match action {
            Action::Set(index, value) => {
                options::set_toggle(&mut self.view, index, value);
                self.sync_energy_style();
                if !self.view.info_on_click {
                    self.gui.set_visible(info::WINDOW, false);
                    self.info_id += 1;
                    self.info_pending = false;
                }
            }
            Action::View(kind) => {
                if (kind == ViewKind::Map) != (self.mode == Mode::Atlas) {
                    self.toggle_mode();
                }
            }
            Action::Projection(p) => self.map.view.set_projection(p),
            Action::Body(id) => self.view.body = id.to_string(),
            Action::Base(id) => self.services.choose_base(id.as_deref()),
            Action::FrameRate(limit) => self.view.frame_rate = limit,
            Action::EnergyStyle => self.toggle_dialog(energy_style::WINDOW),
        }
    }

    /// Shows or hides a dialog; one that does not fit beside the other closes it.
    fn toggle_dialog(&mut self, id: gui::PanelId) {
        self.gui.toggle(id);
        self.separate_dialogs(id);
    }

    /// Windows are moved and stacked by the user, so opening one only brings the dialogs up to date.
    fn separate_dialogs(&mut self, _opened: gui::PanelId) {
        self.sync_options();
        self.sync_time_dialog();
        self.sync_energy_style();
    }

    /// Keeps the time dialog true to the clock, whatever moved it. A drag that ended without the
    /// release reaching the slider (the dialog was closed) lets the clock go.
    fn sync_time_dialog(&mut self) {
        if self.clock.is_held() && !self.gui.is_held() {
            self.clock.release();
        }
        if !self.gui.is_visible(time_dialog::WINDOW) {
            return;
        }
        self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
        let time = self.clock.time();
        let earth = body::current().id == body::EARTH.id;
        let model = time_dialog::Model {
            time: (time / 60.0).floor() * 60.0,
            reading: self.clock.reading(),
            running: self.clock.is_running(),
            earth,
            eclipse: if earth { self.eclipse_watch.status(time) } else { None },
        };
        if *self.time_model.borrow() != model {
            *self.time_model.borrow_mut() = model;
        }
    }

    /// Carries out what the time dialog asked of the clock. True if the picture changes.
    fn time_action(&mut self, action: time_dialog::Action) -> bool {
        use time_dialog::Action;
        self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
        let before = self.clock.time();
        let at = |s: &mut Self, t: f64| s.clock.set_time(t);
        match action {
            Action::Begin(track, pos) => {
                self.clock.hold();
                if let Some(p) = pos {
                    at(self, time_slider::set_position(track, before, p));
                }
            }
            Action::Set(track, pos) => at(self, time_slider::set_position(track, before, pos)),
            Action::End => self.clock.release(),
            Action::Step(track, n) => at(self, time_slider::step(track, before, n as i64)),
            Action::Now => {
                self.clock.release();
                self.sky_key(ClockKey::Now);
            }
            Action::PauseRun => self.sky_key(ClockKey::PauseResume),
            Action::PreviousEclipse | Action::NextEclipse => {
                let found = match action {
                    Action::NextEclipse => time_slider::next_eclipse(before),
                    _ => time_slider::previous_eclipse(before),
                };
                if let Some(t) = found.filter(|_| body::current().id == body::EARTH.id) {
                    self.clock.stop_at(t);
                }
            }
        }
        self.sync_time_dialog();
        // A move of the pointer along a slider that leaves the time as it was needs no new picture.
        !matches!(action, Action::Set(..)) || self.clock.time() != before
    }

    /// Keeps the energy display window true to the settings.
    fn sync_energy_style(&mut self) {
        let model = energy_style::Model { style: self.view.energy_style(), output: self.view.energy_output };
        if *self.energy_style.borrow() != model {
            *self.energy_style.borrow_mut() = model;
        }
    }

    /// Notes what the user changed since the last call, and saves it. A value that only the
    /// command line set is not a change, so it is not saved.
    fn persist(&mut self) {
        let live = self.live();
        if live == self.view_last {
            return;
        }
        live.copy_changes(&self.view_last, &mut self.store.values);
        self.view_last = live.clone();
        self.view = live;
        if let Err(e) = self.store.save_if_changed() {
            eprintln!("{e}");
            self.notice = Some(e);
        }
    }

    /// Mouse input, shared by the window and `--simulate-click`. Each returns whether the
    /// picture needs drawing again: a plain pointer move over the map does not.
    fn mouse_moved(&mut self, pos: DVec2) -> bool {
        self.pointer = pos;
        let over = self.gui.pointer_moved(pos);
        // Leaving the interface changes what it shows as hovered, so that needs a picture too.
        let left = std::mem::replace(&mut self.gui_hover, over);
        // A drag that began on the map stays with it, even across a window.
        if over && self.map_buttons == 0 {
            return true;
        }
        if self.mode == Mode::Atlas {
            self.map.moved(pos);
        } else {
            self.nav.moved(pos);
        }
        over || left || self.map_buttons != 0
    }

    fn mouse_pressed(&mut self, b: Button, mods: Mods) -> bool {
        if self.map_buttons == 0 && self.gui.press(b) {
            return true;
        }
        self.map_buttons |= 1 << (b == Button::Right) as u8;
        if self.mode == Mode::Atlas {
            if b == Button::Left {
                self.map.press(self.pointer);
            }
        } else {
            self.nav.press(b, self.pointer, mods);
        }
        if b == Button::Left {
            let now = self.started.elapsed().as_secs_f64();
            self.clicks.press(now, self.pointer);
        }
        true
    }

    fn mouse_released(&mut self, b: Button, mods: Mods) -> bool {
        let r = self.release_input(b, mods);
        self.persist();
        r
    }

    fn release_input(&mut self, b: Button, mods: Mods) -> bool {
        let bit = 1 << (b == Button::Right) as u8;
        if self.gui.release(b) || self.map_buttons & bit == 0 {
            // The interface acts on it when it builds its next frame.
            return true;
        }
        self.map_buttons &= !bit;
        if self.mode == Mode::Atlas {
            self.map.release();
        } else {
            self.nav.release();
        }
        if b == Button::Left {
            let now = self.started.elapsed().as_secs_f64();
            if let Some(click) = self.clicks.release(now, self.pointer, mods) {
                self.map_click(click);
            }
        }
        true
    }

    fn mouse_wheel(&mut self, lines: f64, mods: Mods) -> bool {
        if self.gui.wheel(lines) {
            return true;
        }
        if self.mode == Mode::Atlas {
            self.map.scroll(lines, self.pointer);
        } else {
            self.nav.scroll(lines, self.pointer, mods);
        }
        true
    }

    /// The pointer left the window, or the window lost focus.
    fn mouse_lost(&mut self) {
        self.gui.mouse_lost();
        self.gui_hover = false;
        self.map_buttons = 0;
        self.clicks.cancel();
    }

    /// Releases single clicks whose double-click window ran out. Returns when to look again.
    fn poll_clicks(&mut self) -> Option<Instant> {
        let now = self.started.elapsed().as_secs_f64();
        if let Some(click) = self.clicks.poll(now) {
                self.map_click(click);
        }
        self.clicks.deadline().map(|t| self.started + Duration::from_secs_f64(t))
    }

    /// A click or double click on the map that no panel took.
    fn map_click(&mut self, click: clicks::Click) {
        if click.kind == clicks::ClickKind::Double {
            if self.mode == Mode::Atlas {
                self.map.double_click(click.pos, click.mods.shift);
            } else if let Some(target) = pick::pick(click.pos, &self.placed_labels, self.nav.ground_at(click.pos)) {
                let (lat, lon) = target.position();
                let distance = pick::end_distance(&target, self.nav.cam.distance, click.mods.shift);
                self.nav.fly_smoothly(Camera { lat, lon, distance, ..self.nav.cam });
            }
        }
        self.on_map_click(click);
    }

    /// The hook for what a click on the map means (picking a place, flying there). It is
    /// called for single clicks after the double-click window and for double clicks.
    fn on_map_click(&mut self, click: clicks::Click) {
        // With the box open, a click on the map only closes it; a double click still flies.
        if self.gui.is_visible(info::WINDOW) {
            self.gui.set_visible(info::WINDOW, false);
            self.info_id += 1;
            self.info_pending = false;
            self.info_changed = true;
            return;
        }
        if click.kind == clicks::ClickKind::Single && self.energy_click(click.pos) {
            return;
        }
        if click.kind == clicks::ClickKind::Single && self.view.info_on_click {
            self.start_info(click.pos);
        }
    }

    /// Opens the information box for the label or the ground under a point, and asks for its
    /// content. Nothing is asked for other bodies, whose places Wikipedia does not locate.
    fn start_info(&mut self, pos: DVec2) {
        let (target, altitude) = if self.mode == Mode::Atlas {
            let ground = self.map.view.lat_lon_at(pos.x, pos.y).map(|(lat, lon)| pick::Target::Ground { lat, lon });
            (ground, self.map.view.distance_for_scale())
        } else {
            (pick::pick(pos, &self.placed_labels, self.nav.ground_at(pos)), self.nav.cam.distance)
        };
        let Some(target) = target else { return };
        // Positions are in radians here, and in degrees for Wikipedia and for the box.
        let (lat, lon) = (target.position().0.to_degrees(), target.position().1.to_degrees());
        let (title, job) = match &target {
            pick::Target::Label { name, style, .. } => {
                let name = wiki::label_name(name, *style);
                (wiki::display_text(&name), wikilookup::Job::Label { name, style: *style, lat, lon })
            }
            pick::Target::Ground { .. } => ("Nearby articles".to_string(), wikilookup::Job::Ground { lat, lon, altitude }),
        };
        let earth = body::current().id == body::EARTH.id;
        self.info_id += 1;
        *self.info.borrow_mut() = info::Model {
            title: if earth { title } else { body::current().name.to_string() },
            at: Some((lat, lon)),
            content: if earth { info::Content::Looking } else { info::Content::Message("Information from Wikipedia is available for Earth only.".to_string()) },
            note: None,
        };
        if earth {
            self.lookup.submit(self.info_id, job);
        }
        self.info_pending = earth;
        self.show_info();
    }

    fn show_info(&mut self) {
        self.gui.set_visible(info::WINDOW, true);
        self.separate_dialogs(info::WINDOW);
        self.info_changed = true;
    }

    /// Carries out what a click in the information box asked for.
    fn info_action(&mut self, action: info::Action) {
        match action {
            info::Action::OpenBrowser => {
                let url = self.info.borrow().article_url().map(str::to_string);
                if let Some(url) = url {
                    self.info.borrow_mut().note = info::open_in_browser(&url).err();
                }
            }
            info::Action::Pick(i) => {
                let Some(near) = self.info.borrow().near(i).cloned() else { return };
                self.info_id += 1;
                *self.info.borrow_mut() = info::Model { title: near.title.clone(), at: None, content: info::Content::Looking, note: None };
                self.lookup.submit(self.info_id, wikilookup::Job::Page { lang: near.lang, page_id: near.page_id, item: near.item });
                self.info_pending = true;
            }
        }
    }

    /// Takes in the answer to the question the box waits for. True if the box changed.
    fn poll_info(&mut self) -> bool {
        let mut changed = std::mem::take(&mut self.info_changed);
        while let Some(reply) = self.lookup.poll() {
            if reply.id != self.info_id {
                continue;
            }
            eprintln!("info: {} request(s) to the network", reply.requests);
            self.info_pending = false;
            let mut m = self.info.borrow_mut();
            match reply.outcome {
                wikilookup::Outcome::Article(a) => {
                    m.title = a.title.clone();
                    m.content = info::Content::Article(a);
                }
                wikilookup::Outcome::Nearby { items, radius } => m.content = info::Content::Nearby { items, radius },
                wikilookup::Outcome::NoArticle => {
                    let text = format!("No article found for {}.", m.title);
                    m.content = info::Content::Message(text);
                }
                wikilookup::Outcome::Failed(f) => m.content = info::Content::Message(f.message().to_string()),
            }
            drop(m);
            changed = true;
        }
        changed
    }

    /// A double click at a point, as the window would deliver it.
    fn simulate_double_click(&mut self, pos: DVec2) {
        self.mouse_moved(pos);
        for _ in 0..2 {
            self.mouse_pressed(Button::Left, Mods::default());
            self.mouse_released(Button::Left, Mods::default());
        }
    }

    /// A left drag between two points, as the window would deliver it: the pointer goes down at the
    /// first, moves in steps and is let go at the second.
    fn simulate_drag(&mut self, from: DVec2, to: DVec2) {
        const STEPS: usize = 8;
        self.mouse_moved(from);
        self.gui_step();
        self.mouse_pressed(Button::Left, Mods::default());
        self.gui_step();
        for i in 1..=STEPS {
            self.mouse_moved(from + (to - from) * (i as f64 / STEPS as f64));
            self.gui_step();
        }
        self.mouse_released(Button::Left, Mods::default());
        self.gui_step();
    }

    /// A left click at a point, as the window would deliver it.
    fn simulate_click(&mut self, pos: DVec2) {
        self.mouse_moved(pos);
        self.gui_step();
        self.mouse_pressed(Button::Left, Mods::default());
        self.gui_step();
        self.mouse_released(Button::Left, Mods::default());
        self.gui_step();
    }

    /// Reads a file and adds what is in it, saying what happened. With `fly` the view moves to it.
    fn import_path(&mut self, path: &std::path::Path, fly: bool) {
        let shown = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        if body::current().id != body::EARTH.id {
            self.notice = Some(format!("{shown}: imported files are placed on Earth only, and this is {}.", body::current().name));
            return;
        }
        match import::load_file(path) {
            Ok(doc) => {
                let target = doc.target();
                self.notice = Some(format!(
                    "Imported {shown}: {} items{}",
                    doc.items.len(),
                    if doc.truncated { " (the rest was left out, the file is too big)" } else { "" }
                ));
                self.library.add(doc);
                self.imports_changed();
                self.import_list.clamp(self.library.item_count());
                if let Some(t) = target.filter(|_| fly) {
                    self.fly_to(t);
                }
            }
            Err(e) => self.notice = Some(format!("{shown} could not be imported: {e}.")),
        }
        eprintln!("{}", self.notice.as_deref().unwrap_or(""));
    }

    /// What is drawn over the map: the imported files and the eclipse path.
    fn import_docs(&self) -> Arc<Vec<Arc<import::Imported>>> {
        let mut docs = (*self.library.snapshot()).clone();
        docs.extend(self.eclipse_path.clone());
        docs.extend(self.markers.clone());
        docs.extend(self.energy.outlines.clone());
        Arc::new(docs)
    }

    fn import_generation(&self) -> u64 {
        self.library.generation + self.eclipse_path_version + self.markers_version + self.lines_version + self.energy.version
    }

    /// Starts the drawn overlays and labels over from the new contents.
    fn imports_changed(&mut self) {
        self.loader.set_imports(self.import_docs(), self.import_generation());
        self.overlay_empty.clear();
        self.overlay_retries.clear();
        self.clear_overlays = true;
        let mut labels = Vec::new();
        'files: for doc in self.library.snapshot().iter() {
            for item in &doc.items {
                let import::Shape::Point(p) = &item.shape else { continue };
                if item.name.is_empty() {
                    continue;
                }
                if labels.len() >= IMPORT_LABELS {
                    break 'files;
                }
                labels.push(vector::Label { text: item.name.clone(), style: vector::LabelStyle::Town, priority: -1000, lat: p.lat, lon: p.lon, along: None });
            }
        }
        self.import_labels = Arc::new(labels);
    }

    fn toggle_imports(&mut self) {
        self.notice = None;
        self.place_list.close();
        self.services.picker.close();
        self.events.picker.close();
        self.import_list.toggle();
    }

    /// Opens or closes the list of eclipses, listed from a year before the clock's date to two years after.
    fn toggle_events(&mut self) {
        self.notice = None;
        self.place_list.close();
        self.import_list.close();
        self.services.picker.close();
        if self.events.picker.is_open() {
            self.events.picker.close();
            return;
        }
        if body::current().id != body::EARTH.id {
            self.notice = Some(format!("Eclipses are listed for Earth only, and this is {}.", body::current().name));
            return;
        }
        self.list_eclipses(self.clock.time());
        self.request_feeds(&event_feeds::Feed::ALL);
        let row = self.events.nearest_to(self.clock.time()).unwrap_or(0);
        self.events.picker.open_at(row);
    }

    /// Asks feeds for events around the clock's date. They answer from the disk cache when it is
    /// fresh, and from the network otherwise.
    fn request_feeds(&mut self, which: &[event_feeds::Feed]) {
        self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
        for &f in which {
            self.feed_problems.retain(|(p, _)| *p != f);
            self.feeds.submit(f, self.clock.time());
            self.feeds_pending += 1;
        }
    }

    /// Takes in what the feeds have answered. True if something changed.
    fn poll_feeds(&mut self) -> bool {
        let mut changed = self.poll_power();
        while let Some(reply) = self.feeds.poll() {
            self.feeds_pending = self.feeds_pending.saturating_sub(1);
            match reply.answer {
                Ok(mut list) => {
                    // Yearly and monthly answers hold more than the dates around the clock.
                    let time = self.clock.time();
                    list.retain(|e| (e.time - time).abs() <= 90.0 * 86_400.0);
                    self.events.set_source(reply.feed.source(), list);
                }
                Err(e) => self.feed_problems.push((reply.feed, e.message(reply.feed))),
            }
            changed = true;
        }
        changed
    }

    /// Redraws the markers when the clock or the list has moved them: events within the window
    /// of the clock's date, sized by magnitude.
    fn update_markers(&mut self) {
        const WINDOW: f64 = 15.0 * 86_400.0;
        let time = self.clock.time();
        let near = if body::current().id == body::EARTH.id && self.view.event_markers { self.events.marked_near(time, WINDOW) } else { Vec::new() };
        if self.markers_from.0 == self.events.revision && self.markers_from.1 == near {
            return;
        }
        let mut credits: Vec<&'static str> = Vec::new();
        let items: Vec<import::Item> = near
            .iter()
            .map(|&j| {
                let e = self.events.marked(j);
                let mut item = import::Item::new("", import::Shape::Point(import::Pos { lat: e.lat, lon: e.lon, alt: None }));
                item.mark = Some(import::Mark { radius: e.marker, colour: e.kind.colour() });
                if let Some(f) = event_feeds::Feed::ALL.into_iter().find(|f| f.source() == e.source) {
                    if !credits.contains(&f.credit()) {
                        credits.push(f.credit());
                    }
                }
                item
            })
            .collect();
        // Large markers first, so that small ones stay on top.
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by(|&a, &b| items[b].mark.map_or(0.0, |m| m.radius).total_cmp(&items[a].mark.map_or(0.0, |m| m.radius)));
        let items = order.into_iter().map(|i| items[i].clone()).collect();
        let was_empty = self.markers.is_none();
        self.markers = (!near.is_empty()).then(|| Arc::new(import::Imported { name: "Events".to_string(), items, truncated: false }));
        self.marker_credits = credits;
        self.markers_from = (self.events.revision, near);
        // Nothing before and nothing now: the drawn overlays are still right, and starting them
        // over would only throw away tiles that are in flight.
        if was_empty && self.markers.is_none() {
            return;
        }
        self.markers_version += 1;
        self.imports_changed();
    }

    /// Asks for the feed given with `--event-feed`, if any.
    fn start_feeds(&mut self, args: &Args) {
        if let Some(f) = args.event_feed {
            if body::current().id == body::EARTH.id {
                self.request_feeds(&[f]);
            }
        }
    }

    fn list_eclipses(&mut self, around: f64) {
        const YEAR: f64 = 365.25 * 86_400.0;
        let found = eclipse::between(around - YEAR, around + 2.0 * YEAR);
        self.events.set_source("eclipse", found.iter().map(events::Event::from_eclipse).collect());
    }

    fn events_key(&mut self, key: places::Key) {
        let pick = self.events.picker.key(key, self.events.len());
        self.notice = None;
        if let places::Pick::Fly(i) = pick {
            self.show_event(i);
        }
    }

    /// Sets the clock to the event, pauses it, and flies to its place. A solar eclipse's central
    /// line is drawn; any other event takes it away.
    fn show_event(&mut self, i: usize) {
        let Some(e) = self.events.get(i).cloned() else { return };
        if e.set_clock {
            self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
            self.clock.stop_at(e.time);
        }
        self.set_eclipse_path((e.kind == events::Kind::SolarEclipse).then_some(e.time));
        self.notice = Some(e.note.clone());
        self.fly_to(search::Target { lat: e.lat, lon: e.lon, distance: e.distance });
    }

    /// Draws the central line of the solar eclipse whose greatest phase is at `time`, or none.
    fn set_eclipse_path(&mut self, time: Option<f64>) {
        let line = time.map(|t| eclipse::central_line(t, 20.0)).unwrap_or_default();
        self.eclipse_path = (line.len() > 1).then(|| {
            let points = line.iter().map(|p| import::Pos { lat: p.lat, lon: p.lon, alt: None }).collect();
            Arc::new(import::Imported {
                name: "Eclipse central line".to_string(),
                items: vec![import::Item::new("", import::Shape::Line(points))],
                truncated: false,
            })
        });
        self.eclipse_path_version += 1;
        self.imports_changed();
    }

    /// Jumps to the eclipse nearest `time`, as choosing it in the list does.
    fn jump_to_eclipse(&mut self, time: f64) {
        if body::current().id != body::EARTH.id {
            self.notice = Some(format!("Eclipses are listed for Earth only, and this is {}.", body::current().name));
            return;
        }
        let Some(found) = eclipse::nearest(time) else { return };
        self.list_eclipses(found.time());
        if let Some(i) = self.events.nearest_to(found.time()) {
            self.events.picker.open_at(i);
            self.events.picker.close();
            self.show_event(i);
        }
    }

    fn imports_key(&mut self, key: places::Key) {
        let pick = self.import_list.key(key, self.library.item_count());
        self.notice = None;
        if let places::Pick::Fly(i) = pick {
            if let Some(t) = self.library.item(i).and_then(|(_, item)| item.target()) {
                self.fly_to(t);
            }
        }
    }

    /// Brings the clock up to date and hands the Sun's place to the renderer.
    fn sync_sky(&mut self, r: &mut Renderer) {
        self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
        let time = self.clock.time();
        let earth = body::current().id == body::EARTH.id;
        r.sun = (body::current().sun)(time);
        r.sun_on = self.view.sunlight;
        r.roads_visible = self.lines_on();
        r.sky_on = self.view.stars;
        if !r.has_stars() {
            r.set_stars(&sky::star_points());
        }
        r.sky_rot = (body::current().rotation)(time);
        r.shadow = (self.view.sunlight && earth).then(|| eclipse::shadow_at(time)).flatten();
        let pos = self.nav.cam.basis().pos;
        let (_, _, height) = ecef_to_geodetic(pos);
        // On the Earth the Sun and the Moon come from the long series, the Moon as seen from the
        // camera, so that an eclipse looks right from the ground.
        let discs = earth.then(|| ephemeris::SunMoon::at(time));
        r.sky_discs = discs.map(|sm| sm.topocentric(pos));
        if let Some(sm) = discs {
            r.sun = (sm.to_earth_fixed() * sm.sun).normalize();
        }
        let sun_sin = self.view.sunlight.then(|| pos.normalize().dot(r.sun));
        // Without air, nothing hides the stars, whatever the Sun is doing.
        let air_height = if body::current().atmosphere { height } else { f64::INFINITY };
        let daylight = r.sky_discs.filter(|_| self.view.sunlight).map_or(1.0, |d| 1.0 - d.obscuration());
        r.sky_limit = sky::limiting_magnitude(air_height, sun_sin, daylight);
        // The planets move slowly, but the Moon is worked out again whenever the clock or the
        // viewer has moved, so that it looks continuous at any clock speed. At real-time speed
        // a frame is only drawn every few seconds, so this costs nothing when idle.
        if (time - self.bodies_time).abs() > 0.25 || (pos - self.bodies_pos).length() > 100.0 {
            self.bodies = (body::current().sky_bodies)(time);
            if earth {
                let tint = eclipse::moon_tint(time);
                for b in self.bodies.iter_mut().filter(|b| b.name == "Moon") {
                    if let Some(d) = r.sky_discs {
                        // Back to J2000, which the renderer turns Earth-fixed again.
                        b.dir = sky::earth_fixed(time).transpose() * d.moon;
                        b.radius = d.moon_radius;
                    }
                    if let Some(tint) = tint {
                        b.colour = std::array::from_fn(|i| b.colour[i] * tint[i]);
                    }
                }
            }
            let points: Vec<_> = self.bodies.iter().map(|b| b.point()).collect();
            r.set_sky_bodies(&points);
            self.bodies_time = time;
            self.bodies_pos = pos;
        }
        self.drawn_time = self.clock.time();
        self.update_markers();
    }

    fn sky_key(&mut self, key: ClockKey) {
        self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
        if key == ClockKey::ToggleSun {
            self.view.sunlight = !self.view.sunlight;
        }
        self.clock.apply(key, sky_time::wall_clock_now());
    }

    /// Real seconds until the Sun has moved enough to redraw; zero when due.
    fn sky_redraw_in(&mut self) -> Option<f64> {
        self.clock.advance(self.clock_epoch.elapsed().as_secs_f64());
        self.clock.redraw_in(self.drawn_time)
    }

    fn receive(&mut self, r: &mut Renderer) -> bool {
        let changed = self.collect_results(r);
        // Not every update is followed by a frame: a hidden window draws none.
        if changed {
            r.flush_uploads();
        }
        changed
    }

    fn collect_results(&mut self, r: &mut Renderer) -> bool {
        let mut changed = false;
        let generation = service_lib::active().generation;
        if generation != self.services_seen {
            // Another map service: the tiles drawn from the old choice are made again.
            self.services_seen = generation;
            r.clear_tiles();
            self.missing.clear();
            self.retries.clear();
            changed = true;
        }
        if std::mem::take(&mut self.clear_overlays) {
            r.clear_overlays();
            changed = true;
        }
        for _ in 0..self.uploads_per_frame {
            match self.loader.try_result() {
                Some(Outcome::Ready(data)) if data.service_generation != generation => {
                    // Made under an earlier choice of services: it will be asked for again.
                    changed = true;
                }
                Some(Outcome::Ready(data)) => {
                    r.insert_tile(&data);
                    self.retries.remove(&data.key);
                    changed = true;
                }
                Some(Outcome::Missing(k)) => {
                    self.missing.insert(k);
                }
                Some(Outcome::Retry(k, until)) => next_retry(&mut self.retries, k, until),
                Some(Outcome::Overlay(data)) => {
                    self.label_sets.insert(data.source, data.labels.clone());
                    // Drawn before the imports last changed: the tile will be asked for again.
                    if data.generation != self.import_generation() {
                        changed = true;
                        continue;
                    }
                    self.overlay_retries.remove(&data.key);
                    match &data.mips {
                        Some(mips) => r.insert_overlay(data.key, mips),
                        None => {
                            self.overlay_empty.insert(data.key);
                        }
                    }
                    changed = true;
                }
                Some(Outcome::OverlayRetry(k, until)) => next_retry(&mut self.overlay_retries, k, until),
                Some(Outcome::Buildings(k, mesh)) => {
                    self.building_retries.remove(&k);
                    match mesh {
                        Some(mesh) => r.insert_buildings(k, &mesh),
                        None => {
                            self.building_empty.insert(k);
                        }
                    }
                    changed = true;
                }
                Some(Outcome::BuildingsRetry(k, until)) => next_retry(&mut self.building_retries, k, until),
                // The workers keep these themselves.
                Some(Outcome::Deferred(_)) => {}
                None => break,
            }
        }
        changed
    }

    /// Advances the view and works out what to draw and fetch. Returns whether the picture changed.
    /// Draws the overlays again when the kinds of line shown have changed. They come from the
    /// decoded tiles in memory, or the disk cache: nothing is asked of a server.
    fn energy_sources(&self) -> [bool; energy::SOURCES] {
        let v = &self.view;
        [
            v.energy_solar,
            v.energy_wind_onshore,
            v.energy_wind_offshore,
            v.energy_hydro,
            v.energy_biomass,
            v.energy_lignite,
            v.energy_hard_coal,
            v.energy_gas,
            v.energy_oil,
            v.energy_other_fossil,
            v.energy_nuclear,
        ]
    }

    fn energy_on(&self) -> bool {
        self.view.energy && self.mode == Mode::Globe && body::current().id == body::EARTH.id
    }

    /// Keeps the energy bars and the outlines of their areas up to date with the view, the clock
    /// and the options.
    fn update_energy(&mut self, r: &mut Renderer, aspect_height: (f64, f64)) {
        self.energy.view = aspect_height;
        let data = if self.energy_on() { energy::Energy::bundled() } else { None };
        if self.energy_on() && data.is_none() && !self.energy.missing {
            self.energy.missing = true;
            self.notice = Some("The energy data built into this program could not be read.".to_string());
        }
        let Some(data) = data else {
            if self.gui.is_visible(power_chart::WINDOW) {
                self.gui.set_visible(power_chart::WINDOW, false);
            }
            if self.energy.outlines.is_some() || self.energy.built_from.is_some() {
                let power = std::mem::take(&mut self.energy.power);
                let chart = self.energy.chart.clone();
                self.energy = EnergyLayer { view: aspect_height, missing: self.energy.missing, version: self.energy.version + 1, power, chart, ..Default::default() };
                r.set_energy(None);
                self.imports_changed();
            }
            return;
        };
        let style = self.view.energy_style();
        if self.energy.style != Some(style) {
            self.energy.style = Some(style);
            self.energy.outlines_from = None;
            self.energy.built_from = None;
        }
        let level = self.view.energy_detail.level(self.nav.altitude, self.energy.level);
        self.energy.level = Some(level);
        let shown = self.energy_sources();
        let month = energy::month_of(self.clock.time());
        // Close up the shade, finer down the dashing of district outlines and with a choice its fill
        // follow the clock's month.
        let monthly = level >= energy::Level::Towns || self.energy.selected.is_some();
        let wanted = (level, shown, self.energy.selected, monthly.then_some(month));
        if self.energy.outlines_from != Some(wanted) {
            self.energy.outlines = Some(Arc::new(data.outlines(level, shown, self.energy.selected, month, style.palette)));
            self.energy.outlines_from = Some(wanted);
            self.energy.version += 1;
            self.imports_changed();
        }
        self.update_power(data, energy::month_of(self.clock.time()));
        self.sync_power_chart();
        let cam = &self.nav.cam;
        let scale = style.apply(energy::Scale::for_view(data, level, cam.distance, aspect_height.1));
        // The centre in steps of a tenth of the bars' reach, so that panning rebuilds only now and then.
        let reach = (self.nav.altitude * 6.0 / geo::radius()).clamp(0.02, std::f64::consts::PI);
        let step = reach / 10.0;
        let centre = ((cam.lat / step).round() as i64, (cam.lon / step).round() as i64);
        // The quarter hour shown, and whether demand stands beside the bars.
        let quarter = self.energy.power.shown.as_ref().map(|m| (m.0.time as i64) * 2 + self.view.energy_demand as i64);
        let key = (level, month, shown, centre, r.tile_count(), quarter);
        let rescaled = self.energy.scale.is_none_or(|s| (s.per_kw / scale.per_kw - 1.0).abs() > 0.03);
        let same_but_terrain = self.energy.built_from.is_some_and(|b| (b.0, b.1, b.2, b.3, b.5) == (key.0, key.1, key.2, key.3, key.5));
        let terrain_due = self.energy.built_at.is_none_or(|t| t.elapsed() > Duration::from_millis(500));
        if !rescaled && (self.energy.built_from == Some(key) || (same_but_terrain && !terrain_due)) {
            return;
        }
        let near = match level {
            energy::Level::States | energy::Level::Districts => None,
            _ => Some((cam.lat, cam.lon, reach)),
        };
        self.energy.bars = data.bars(level, month, shown, near);
        // Beside the bars of states, districts and municipalities, their estimated demand.
        if let (Some((m, _)), true) = (&self.energy.power.shown, self.view.energy_demand) {
            let consumption = m.consumption();
            for b in self.energy.bars.iter_mut().filter(|b| matches!(b.subject, energy::Subject::Area(_))) {
                b.demand = data.demand_kw(b.subject, consumption).unwrap_or(0.0);
            }
        }
        let ground = |lat: f64, lon: f64| r.ground(lat, lon).unwrap_or(0.0);
        let shares = self.energy.power.shown.as_ref().map(|m| m.1);
        let bars = energy::mesh(&self.energy.bars, scale, geo::geodetic_to_ecef(cam.lat, cam.lon, 0.0), &ground, shares.as_ref(), &style);
        r.set_energy(Some((&bars.mesh, bars.solid)));
        self.energy.scale = Some(scale);
        self.energy.built_from = Some(key);
        self.energy.built_at = Some(Instant::now());
    }

    /// Asks SMARD for the week of the clock when the bars are to show output, and finds the
    /// quarter hour to show.
    fn update_power(&mut self, data: &energy::Energy, month: u16) {
        if !self.view.energy_output {
            self.energy.power.shown = None;
            return;
        }
        let time = self.clock.time();
        let fresh = Duration::from_secs(600);
        let now = Instant::now();
        let p = &mut self.energy.power;
        let feed = p.feed.get_or_insert_with(|| power_feed::PowerFeed::new(self.fetcher.clone()));
        // The index grows by a week each Monday; it is asked again every ten minutes while in use.
        if p.index_asked.is_none_or(|t| now.duration_since(t) > fresh) {
            p.index_asked = Some(now);
            feed.ask_index();
            self.feeds_pending += 1;
        }
        // The clock's week, and for the day chart the weeks of the day's first and last quarter hours.
        let day = (time / 86_400.0).floor() * 86_400.0;
        let chart = self.view.energy_chart;
        let wanted: Vec<i64> = [Some(time), chart.then_some(day), chart.then_some(day + 86_399.0)]
            .into_iter()
            .flatten()
            .filter_map(|t| p.index.as_deref().and_then(|i| power_feed::week_for(i, t)))
            .collect();
        for &w in &wanted {
            let latest = p.index.as_deref().and_then(|i| i.last()) == Some(&w);
            let due = match p.asked.get(&w) {
                None => true,
                // The latest week fills up as the day goes on.
                Some(t) => latest && now.duration_since(*t) > fresh && p.weeks.contains_key(&w),
            };
            if due {
                p.asked.insert(w, now);
                feed.ask_week(w);
                self.feeds_pending += 1;
            }
        }
        let Some(week) = p.index.as_deref().and_then(|i| power_feed::week_for(i, time)) else {
            p.shown = None;
            return;
        };
        let latest = p.index.as_deref().and_then(|i| i.last()) == Some(&week);
        // Live, SMARD is about three quarters of an hour behind: its latest values stand in.
        let moment = p.weeks.get(&week).and_then(|w| w.moment(time).or_else(|| w.last_moment().filter(|m| latest && m.time < time)));
        p.shown = moment.map(|m| {
            let shares = m.shares(&data.national(month));
            (m, shares)
        });
    }

    /// Fills the day chart for the clock's day, and shows it while it is wanted.
    fn sync_power_chart(&mut self) {
        let wanted = self.energy_on() && self.view.energy_output && self.view.energy_chart;
        if self.gui.is_visible(power_chart::WINDOW) != wanted {
            self.gui.set_visible(power_chart::WINDOW, wanted);
        }
        if !wanted {
            return;
        }
        let time = self.clock.time();
        let day = (time / 86_400.0).floor() * 86_400.0;
        let p = &self.energy.power;
        let moment = |t: f64| p.index.as_deref().and_then(|i| power_feed::week_for(i, t)).and_then(|w| p.weeks.get(&w)).and_then(|w| w.moment(t));
        let moments: Vec<Option<power_feed::Moment>> = (0..96).map(|k| moment(day + k as f64 * 900.0)).collect();
        let have = moments.iter().filter(|m| m.is_some()).count();
        let date = &sky_time::format_utc(day, false)[..10];
        let model = power_chart::Model {
            title: format!("Germany {date}, GW"),
            chart: (have > 0).then(|| power_chart::chart(&moments, Some((time - day) / 86_400.0), self.view.energy_palette)),
            note: if have > 0 {
                "Areas above the line: surplus.".to_string()
            } else if self.feeds_pending > 0 {
                "Asking SMARD...".to_string()
            } else {
                "No data for this day (SMARD has 2015 to an hour ago).".to_string()
            },
        };
        if *self.energy.chart.borrow() != model {
            *self.energy.chart.borrow_mut() = model;
        }
    }

    /// Takes in what SMARD answered. Returns whether anything arrived.
    fn poll_power(&mut self) -> bool {
        let mut changed = false;
        let p = &mut self.energy.power;
        while let Some(reply) = p.feed.as_ref().and_then(|f| f.poll()) {
            self.feeds_pending = self.feeds_pending.saturating_sub(1);
            changed = true;
            match reply {
                power_feed::Reply::Index(Ok(index)) => p.index = Some(index),
                power_feed::Reply::Week(start, Ok(week)) => {
                    p.weeks.insert(start, week);
                    p.problem = None;
                }
                power_feed::Reply::Index(Err(e)) | power_feed::Reply::Week(_, Err(e)) => p.problem = Some(e),
            }
        }
        if changed {
            // Rebuild the bars with what came.
            self.energy.built_from = None;
        }
        changed
    }

    /// Chooses the bar under a click and tells about it. Returns whether there was one.
    fn energy_click(&mut self, pos: DVec2) -> bool {
        let (Some(data), Some(scale), true) = (energy::Energy::bundled(), self.energy.scale, self.energy_on()) else { return false };
        let (aspect, height) = self.energy.view;
        let screen = Screen { basis: self.nav.cam.basis(), size: DVec2::new(aspect * height, height), reserved: Vec::new() };
        // The ground as the bars were built on it is not kept, so sea level is close enough for picking.
        let mut hit = energy::hit(&self.energy.bars, scale, &|_, _| 0.0, &|p| screen.project(p), pos);
        // Close up, a municipality's units without a position are its shade on the ground.
        if hit.is_none() && self.energy.level == Some(energy::Level::Plants) {
            let shown = self.energy_sources();
            let month = energy::month_of(self.clock.time());
            hit = self
                .nav
                .ground_at(pos)
                .and_then(|(lat, lon)| data.town_at(lat, lon))
                .filter(|&i| data.unplaced_density(i, month, shown).is_some_and(|d| d > 0.0))
                .map(energy::Subject::Unplaced);
        }
        let Some(subject) = hit else {
            if self.energy.selected.take().is_some() {
                self.energy.outlines_from = None;
            }
            return false;
        };
        self.energy.selected = Some(subject);
        self.energy.outlines_from = None;
        let month = energy::month_of(self.clock.time());
        self.info_id += 1;
        self.info_pending = false;
        let at = match subject {
            energy::Subject::Unplaced(i) => Some(data.areas[i as usize].label),
            _ => self.energy.bars.iter().find(|b| b.subject == subject).map(|b| (b.lat, b.lon)),
        }
        .map(|(lat, lon)| (lat.to_degrees(), lon.to_degrees()));
        *self.info.borrow_mut() = info::Model {
            title: data.subject_title(subject),
            at,
            content: info::Content::Facts({
                let when = self.energy.power.shown.as_ref().map(|(m, sh)| (sh, sky_time::format_utc(m.time, false)[11..16].to_string() + " UTC", m.consumption()));
                let at = when.as_ref().map(|(sh, w, c)| energy::At { shares: sh, when: w.as_str(), consumption_mw: *c });
                data.describe(subject, month, self.energy_sources(), self.view.energy_palette, at)
            }),
            note: None,
        };
        self.show_info();
        true
    }

    /// What the energy card tells: what a bar stands for, the level of detail, the scale and the figures.
    fn energy_card(&self) -> Option<energy_panel::Model> {
        let (level, scale) = (self.energy.level?, self.energy.scale?);
        if !self.energy_on() {
            return None;
        }
        let data = energy::Energy::bundled()?;
        let mut notes = Vec::new();
        if level == energy::Level::Plants {
            notes.push(format!("Shade: units without a position, full at {}/km²", energy::format_power(data.dense())));
        } else if self.view.energy_demand && self.energy.power.shown.is_some() {
            notes.push("White column: estimated use, Germany's consumption shared out by population".to_string());
        }
        Some(energy_panel::Model {
            detail: self.view.energy_detail,
            level,
            counts: energy::Level::ALL.map(|l| data.count(l)),
            month: energy::month_name(energy::month_of(self.clock.time())),
            scale: format_distance(1.0e6 * scale.per_kw),
            output: self.view.energy_output.then(|| self.power_status()),
            notes,
            flat: self.nav.cam.total_tilt() < 0.3,
        })
    }

    /// Germany's generation against consumption at the quarter hour shown, or why there is none.
    fn power_status(&self) -> String {
        let p = &self.energy.power;
        let Some((m, _)) = &p.shown else {
            return match (&p.problem, &p.index) {
                (Some(e), _) => format!("Output unavailable ({e}): bars show capacity"),
                (None, None) => "Output: asking SMARD...".to_string(),
                (None, Some(_)) if p.feed.is_some() && self.feeds_pending > 0 => "Output: asking SMARD...".to_string(),
                _ => "No output data for this time: bars show capacity".to_string(),
            };
        };
        let gw = |mw: f32| format!("{:.1} GW", mw / 1000.0);
        let (generation, consumption) = (m.generation(), m.consumption());
        let balance = generation - consumption;
        // The date is the clock's, shown above; only the quarter hour can differ.
        format!(
            "{} UTC: {} generated, {:.0}% renewable; use {}, {} {}",
            &sky_time::format_utc(m.time, false)[11..16],
            gw(generation),
            100.0 * m.renewable() / generation.max(1.0),
            gw(consumption),
            if balance >= 0.0 { "surplus" } else { "short" },
            gw(balance.abs()),
        )
    }

    fn sync_lines(&mut self) {
        let kinds = LineKinds::of(&self.view);
        if kinds == self.lines_drawn {
            return;
        }
        self.lines_drawn = kinds;
        self.lines_version += 1;
        self.loader.set_lines(kinds);
        self.imports_changed();
    }

    fn update(&mut self, r: &mut Renderer, dt: f64, aspect_height: (f64, f64)) -> bool {
        self.sync_lines();
        self.sync_sky(r);
        let received = self.receive(r);
        let atlas = self.mode == Mode::Atlas;
        let selection = if atlas {
            let (axes, sens) = self.nav.device();
            self.map.device(&axes, sens, dt);
            let size = DVec2::new((aspect_height.0 * aspect_height.1).round(), aspect_height.1);
            if self.map.view.size != (size.x, size.y) {
                self.map.resize(size);
            }
            self.map.tick(dt);
            atlas::select(&self.map.view, &|k| r.has_tile(k), sources::max_tile_z)
        } else {
            {
                // The camera also stays above the surface as drawn, which a coarse parent tile can
                // hold higher than the finest ground until its finer tiles have all arrived.
                let ground = |lat: f64, lon: f64| r.ground(lat, lon);
                let draw = &self.draw;
                let floor = |lat: f64, lon: f64| match (r.ground(lat, lon), r.drawn_ground(draw, lat, lon)) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                self.nav.tick_over(dt, &ground, &floor);
            }
            let mut view;
            let mut selection;
            // The surface that was drawn a frame ago is only a guess for this one: the view may
            // have moved onto a coarser, higher tile. So the camera is checked again against the
            // tiles chosen for it now, and lifted and chosen for again if it is under them.
            let mut pass = 0;
            loop {
                view = View {
                    basis: self.nav.cam.basis(),
                    aspect: aspect_height.0,
                    height_px: aspect_height.1,
                    max_z: sources::max_tile_z,
                };
                selection = view.select(&|k| k.z <= self.draw_max_z && r.has_tile(k));
                pass += 1;
                let draw = &selection.draw;
                let floor = |lat: f64, lon: f64| match (r.ground(lat, lon), r.drawn_ground(draw, lat, lon)) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
                if !self.nav.keep_above(&floor) || pass == 3 {
                    break;
                }
            }
            // During a flight only the tiles of the view it lands on are asked for, early; the
            // views passed on the way are drawn from what is there, and cost no downloads.
            if let Some(goal) = self.nav.flight_goal() {
                let at_goal = View { basis: goal.basis(), ..view };
                selection.wanted = at_goal.select(&|k| r.has_tile(k)).wanted;
            }
            selection
        };
        let flying = self.nav.flight_goal().is_some() && !atlas;
        let now = Instant::now();
        let wanted: Vec<TileKey> = selection
            .wanted
            .into_iter()
            .filter(|k| !self.missing.contains(k))
            .collect();
        // Tiles waiting out a retry delay still count as outstanding, but are not requested yet.
        self.waiting = wanted.len();
        self.retrying = wanted.iter().filter(|k| self.retries.contains_key(k)).count();
        let wanted: Vec<TileKey> = wanted
            .into_iter()
            .filter(|k| self.retries.get(k).is_none_or(|e| e.0 <= now))
            .collect();
        let changed = received || selection.draw != self.draw;
        self.draw = selection.draw;

        let mut overlays = Vec::new();
        // Labels are placed on the globe only.
        let labels_wanted = self.names_on() && !atlas && !flying;
        if body::current().surface_data && ((self.lines_on() && !flying) || labels_wanted) {
            for &k in &self.draw {
                let lines_needed = !r.has_overlay(k) && !self.overlay_empty.contains(&k);
                if !lines_needed && (!labels_wanted || self.label_sets.contains_key(&vector_source(k))) {
                    continue;
                }
                self.waiting += 1;
                match self.overlay_retries.get(&k) {
                    Some(e) if e.0 > now => self.retrying += 1,
                    Some(_) => {
                        self.retrying += 1;
                        overlays.push(k);
                    }
                    None => overlays.push(k),
                }
            }
        }
        let mut building_jobs = Vec::new();
        let visible = if body::current().surface_data && self.view.buildings && !atlas && !flying { building_tiles(&self.draw) } else { Vec::new() };
        for &k in &visible {
            if r.has_buildings(k) || self.building_empty.contains(&k) {
                continue;
            }
            self.waiting += 1;
            match self.building_retries.get(&k) {
                Some(e) if e.0 > now => self.retrying += 1,
                Some(_) => {
                    self.retrying += 1;
                    building_jobs.push(k);
                }
                None => building_jobs.push(k),
            }
        }
        r.set_visible_buildings(visible);
        self.update_energy(r, aspect_height);
        self.update_shadows(r, aspect_height, atlas);
        self.loader.set_wanted(&wanted, &overlays, &building_jobs);
        // Once everything has arrived, the terrain under the start view is known for good.
        if self.waiting == 0 && self.loader.is_idle() {
            self.nav.finish_start();
        }
        changed
    }

    /// Plans the shadow maps for this view and finds the loaded tiles that cast into them, drawn or not.
    fn update_shadows(&mut self, r: &mut Renderer, (aspect, height): (f64, f64), atlas: bool) {
        r.shadows_on = self.view.cast_shadows;
        let cam = self.nav.cam;
        let input = shadows::Input {
            basis: cam.basis(),
            aspect,
            ground_h: cam.target_h,
            near: cam.near_plane(self.nav.altitude),
        };
        let plan = (self.view.cast_shadows && self.view.sunlight && !atlas)
            .then(|| shadows::plan(&input, r.sun))
            .flatten();
        let Some(plan) = plan else {
            r.set_cast(None);
            self.shadow_cast = None;
            return;
        };
        let mut hasher = DefaultHasher::new();
        self.draw.hash(&mut hasher);
        let (fingerprint, loaded) = (hasher.finish(), r.tile_count());
        let extra = match self.shadow_cast.take() {
            Some((p, f, n, extra)) if p == plan && f == fingerprint && n == loaded => extra,
            _ => {
                let view = View { basis: input.basis, aspect, height_px: height, max_z: sources::max_tile_z };
                let pos = input.basis.pos;
                view.select_in(
                    &|c, radius| plan.touches(c - pos, radius),
                    &|k| k.z <= self.draw_max_z && r.has_tile(k),
                    &self.draw,
                )
            }
        };
        r.set_cast(Some((plan.clone(), extra.clone())));
        self.shadow_cast = Some((plan, fingerprint, loaded, extra));
    }

    /// Chooses the labels for this frame and hands them to the renderer.
    fn place_labels(&mut self, r: &mut Renderer, (width, height): (u32, u32), scale: u32, dt: f64) {
        while let Some(result) = self.text.try_result() {
            if let Ok(mask) = result {
                r.receive_mask(mask);
            }
        }
        self.labels_shown = 0;
        if !self.names_on() || self.mode == Mode::Atlas {
            r.set_labels(&[]);
            self.label_alpha.clear();
            self.placed_labels.clear();
            return;
        }
        let sources: HashSet<TileKey> = self.draw.iter().map(|k| vector_source(*k)).collect();
        if self.label_sets.len() > LABEL_SETS {
            self.label_sets.retain(|k, _| sources.contains(k));
        }
        // In a fixed order: labels with the same name nearby share an identity, so only their
        // position in this list keeps them from swapping between frames.
        let mut ordered: Vec<TileKey> = sources.iter().copied().collect();
        ordered.sort_unstable();
        let mut sets: Vec<_> = ordered.iter().filter_map(|k| self.label_sets.get(k).cloned()).collect();
        if !self.import_labels.is_empty() {
            sets.push(self.import_labels.clone());
        }

        let names = NameKinds::of(&self.view);
        let pad = |p: [f64; 4]| [p[0] - 4.0, p[1] - 4.0, p[2] + 4.0, p[3] + 4.0];
        let screen = Screen {
            basis: self.nav.cam.basis(),
            size: DVec2::new(width as f64, height as f64),
            reserved: self.gui.covered().iter().map(|p| pad(*p)).collect(),
        };
        let mut shown: Vec<(&vector::Label, i32, u64)> = sets
            .iter()
            .flat_map(|set| set.iter())
            .filter(|label| names.allows(label.style) && labels::may_show(&screen, label))
            .map(|label| {
                let id = label_id(label);
                let bonus = if self.label_alpha.contains_key(&id) { LABEL_STAY_BONUS } else { 0 };
                (label, label.priority - bonus, id)
            })
            .collect();
        // The identity breaks ties, so equal labels keep their order from frame to frame.
        shown.sort_by_key(|s| (s.1, s.2));
        shown.truncate(LABEL_CANDIDATES);

        let mut candidates = Vec::with_capacity(shown.len());
        let mut details = Vec::with_capacity(shown.len());
        for (label, _, id) in &shown {
            let Some(text) = labels::usable_text(&label.text) else { continue };
            let key = TextKey { text, style: label.style, scale };
            let Some(rect) = r.label_rect(&key, &mut self.text) else { continue };
            candidates.push((*label, DVec2::new(rect.w as f64, rect.h as f64)));
            details.push((rect, *id));
        }

        let ground = |lat: f64, lon: f64| r.ground(lat, lon);
        let placed = labels::place(&screen, &candidates, &ground);

        self.placed_labels = placed
            .iter()
            .map(|p| {
                let rect = details[p.index].0;
                let half = pick::PlacedLabel::text_half(DVec2::new(rect.w as f64, rect.h as f64), LABEL_PAD * scale as f64);
                pick::PlacedLabel::new(candidates[p.index].0, p.center, half, p.angle)
            })
            .collect();
        let mut alpha = HashMap::with_capacity(placed.len());
        let atlas = ATLAS_SIZE as f32;
        let instances: Vec<LabelInstance> = placed
            .iter()
            .map(|p| {
                let (rect, id) = details[p.index];
                let look = labels::appearance(candidates[p.index].0.style);
                let opacity = self.label_alpha.get(&id).copied().unwrap_or(0.0);
                let opacity = (opacity + dt * LABEL_FADE_RATE).min(1.0);
                alpha.insert(id, opacity);
                LabelInstance {
                    center: [p.center.x as f32, p.center.y as f32],
                    half: [rect.w as f32 / 2.0, rect.h as f32 / 2.0],
                    uv: [
                        rect.x as f32 / atlas,
                        rect.y as f32 / atlas,
                        (rect.x + rect.w) as f32 / atlas,
                        (rect.y + rect.h) as f32 / atlas,
                    ],
                    fill: [look.fill[0], look.fill[1], look.fill[2], opacity as f32],
                    halo: [look.halo[0], look.halo[1], look.halo[2], p.angle as f32],
                }
            })
            .collect();
        let mut instances = instances;
        instances.extend(self.sky_labels(r, &screen, scale));
        self.labels_shown = instances.len();
        self.label_alpha = alpha;
        r.set_labels(&instances);
    }

    /// For saved images: waits for all label text, then chooses the labels afresh and fully
    /// faded in. Labels on screen otherwise keep their place, so which ones won depends on the
    /// order their text arrived; this makes images repeatable, and simulated clicks use the same
    /// choice as the image they end up in.
    fn settle_labels(&mut self, r: &mut Renderer, size: (u32, u32), step: f64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.place_labels(r, size, 1, step);
            r.flush_uploads();
            if !self.labels_changing() || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.label_alpha.clear();
        self.place_labels(r, size, 1, 1.0);
    }

    /// Names for the planets and the Moon where they are seen, above the ground and bright
    /// enough to be visible.
    fn sky_labels(&mut self, r: &mut Renderer, screen: &Screen, scale: u32) -> Vec<LabelInstance> {
        if !self.view.stars || !r.has_stars() {
            return Vec::new();
        }
        let pos = screen.basis.pos;
        let rotation = r.sky_rot;
        let atlas = ATLAS_SIZE as f32;
        let mut out = Vec::new();
        for body in &self.bodies {
            let fade = ((r.sky_limit - body.mag) / 1.5).clamp(0.0, 1.0);
            if fade < 0.3 {
                continue;
            }
            let dir = (rotation * body.dir).normalize();
            if geo::ray_ellipsoid(pos, dir, 0.0).is_some() {
                continue;
            }
            let Some(p) = screen.project(pos + dir * 1.0e10) else { continue };
            if p.x < 0.0 || p.y < 0.0 || p.x > screen.size.x || p.y > screen.size.y {
                continue;
            }
            let key = TextKey { text: body.name.to_string(), style: vector::LabelStyle::Sea, scale };
            let Some(rect) = r.label_rect(&key, &mut self.text) else { continue };
            let look = labels::appearance(key.style);
            let radius = if body.radius > 0.0 { 7.0 } else { 6.0 };
            out.push(LabelInstance {
                center: [p.x as f32 + rect.w as f32 / 2.0 + radius as f32, p.y as f32 - rect.h as f32 / 2.0],
                half: [rect.w as f32 / 2.0, rect.h as f32 / 2.0],
                uv: [
                    rect.x as f32 / atlas,
                    rect.y as f32 / atlas,
                    (rect.x + rect.w) as f32 / atlas,
                    (rect.y + rect.h) as f32 / atlas,
                ],
                fill: [look.fill[0], look.fill[1], look.fill[2], fade as f32],
                halo: [look.halo[0], look.halo[1], look.halo[2], 0.0],
            });
        }
        out
    }

    /// True while labels are still arriving or fading in.
    fn labels_changing(&self) -> bool {
        !self.text.is_idle() || self.label_alpha.values().any(|a| *a < 1.0)
    }

    /// The quiet line at the top right: where the view is, how high, and the clock; and what is
    /// still loading, if anything.
    fn status_line(&self) -> (String, Option<String>) {
        let cam = &self.nav.cam;
        let view = &self.map.view;
        let (position, scale) = match self.mode {
            Mode::Globe => (hemisphere_label(cam.lat, cam.lon), format!("altitude {}", format_distance(self.nav.altitude))),
            Mode::Atlas => {
                let (lat, lon) = view.centre_lat_lon();
                (hemisphere_label(lat, lon), format!("{}, {} per pixel", view.projection.name(), format_distance(view.metres_per_pixel())))
            }
        };
        let position = match body::current() {
            b if b.id == body::EARTH.id => position,
            b => format!("{}  {position}", b.name),
        };
        let sky = if self.view.sunlight { self.clock.label() } else { format!("{}, flat light", self.clock.label()) };
        let busy = (self.waiting > 0).then(|| loading_status(self.waiting, self.retrying));
        (format!("{position}  \u{00b7}  {scale}  \u{00b7}  {sky}"), busy)
    }

    /// The technical figures shown on request (F12): tiles, frame rate and downloads.
    fn technical_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("tiles {}/{}, {}", self.draw.len(), self.tiles_loaded, loading_status(self.waiting, self.retrying))];
        if self.mode == Mode::Atlas {
            let meridian = self.map.view.central_meridian.to_degrees();
            lines.push(format!("central meridian {:.0}° {}", meridian.abs(), if meridian >= 0.0 { 'E' } else { 'W' }));
        }
        lines.extend(self.frame_status.clone());
        lines.extend(network_status(&self.fetcher, SystemTime::now()));
        lines
    }

    /// Gathers what the interface needs from the renderer, then builds it for this frame.
    /// True if something done in it changes the picture.
    fn set_panels(&mut self, r: &Renderer, width: u32, height: u32, ppp: f32) -> bool {
        let mut credits = credits_for(self.draw.iter().map(|k| r.tile_credit(*k)));
        if self.view.stars && self.mode == Mode::Globe {
            credits.push('\n');
            credits.push_str(STARS_CREDIT);
        }
        for c in &self.marker_credits {
            credits.push('\n');
            credits.push_str(c);
        }
        if self.energy_on() {
            credits.push('\n');
            credits.push_str(energy::CREDIT);
            if self.energy.power.shown.is_some() {
                credits.push('\n');
                credits.push_str(power_feed::CREDIT);
            }
        }
        if self.search.credit_due() {
            credits.push('\n');
            credits.push_str(search::CREDIT);
        }
        self.credits_text = credits;
        self.tiles_loaded = r.tile_count();
        self.gui_size = (width, height, ppp);
        self.gui_step()
    }

    /// Runs one frame of the interface and carries out what the user did in it. True if
    /// anything was done.
    fn gui_step(&mut self) -> bool {
        self.sync_options();
        self.sync_time_dialog();
        self.sync_energy_style();
        let (w, h, ppp) = self.gui_size;
        let raw = self.gui.begin((w, h), ppp);
        let ctx = self.gui.ctx.clone();
        let mut done = GuiDone::default();
        let out = ctx.run_pass(raw, |ctx| self.build_gui(ctx, &mut done));
        self.gui.finish(out);
        let links = std::mem::take(&mut self.gui.links);
        for url in &links {
            if let Err(e) = info::open_web_page(url) {
                self.notice = Some(e);
            }
        }
        let changed = self.gui_done(done) || !links.is_empty();
        if changed {
            self.persist();
        }
        changed
    }

    /// Lays out this frame's interface from the scene's state.
    fn build_gui(&mut self, ctx: &egui::Context, done: &mut GuiDone) {
        let heading = (self.mode == Mode::Globe).then_some(self.nav.cam.heading);
        let open = [help::WINDOW, options::WINDOW, time_dialog::WINDOW].map(|w| self.gui.is_visible(w));
        done.icon = compass::show(ctx, &mut self.gui, heading, open);
        readout::credits(ctx, &mut self.gui, &self.credits_text, compass::row_width(heading.is_some()));
        let (line, busy) = self.status_line();
        readout::status(ctx, &mut self.gui, &line, busy.as_deref());
        if self.debug_readout {
            let lines = self.technical_lines();
            readout::technical(ctx, &mut self.gui, &lines);
        }
        let search = self.search.panel_text();
        let below = readout::text_box(ctx, &mut self.gui, "search-box", &search, 10.0);
        let side = self.side_panel_text();
        readout::text_box(ctx, &mut self.gui, "side-box", &side, if below > 0.0 { below + 18.0 } else { 10.0 });
        if let Some(card) = self.energy_card() {
            done.energy = energy_panel::show(ctx, &mut self.gui, &card);
        }
        help::show(ctx, &mut self.gui, self.help_scroll);
        done.options = options::show(ctx, &mut self.gui, &self.options.borrow());
        done.time = time_dialog::show(ctx, &mut self.gui, &self.time_model.borrow());
        done.style = energy_style::show(ctx, &mut self.gui, &self.energy_style.borrow());
        done.chart = power_chart::show(ctx, &mut self.gui, &self.energy.chart.borrow());
        done.info = info::show(ctx, &mut self.gui, &self.info.borrow());
    }

    /// Carries out what the user did in a frame of the interface. True if anything was done.
    fn gui_done(&mut self, d: GuiDone) -> bool {
        let mut changed = d.icon.is_some();
        match d.icon {
            Some(compass::Clicked::Help) => self.toggle_dialog(help::WINDOW),
            Some(compass::Clicked::Options) => self.toggle_dialog(options::WINDOW),
            Some(compass::Clicked::Time) => self.toggle_dialog(time_dialog::WINDOW),
            Some(compass::Clicked::North) => self.nav.face_north(),
            None => {}
        }
        if let Some(a) = d.options {
            self.options_action(a);
            changed = true;
        }
        for a in d.time {
            changed |= self.time_action(a);
        }
        if let Some(energy_style::Action::Set(style)) = d.style {
            self.view.set_energy_style(style);
            self.sync_energy_style();
            changed = true;
        }
        match d.chart {
            Some(power_chart::Event::Closed) => self.view.energy_chart = false,
            Some(power_chart::Event::Style) => self.toggle_dialog(energy_style::WINDOW),
            None => {}
        }
        changed |= d.chart.is_some();
        let (closed, action) = d.info;
        if closed {
            self.info_id += 1;
            self.info_pending = false;
            changed = true;
        }
        if let Some(a) = action {
            self.info_action(a);
            changed = true;
        }
        match d.energy {
            Some(energy_panel::Action::Detail(detail)) => self.view.energy_detail = detail,
            Some(energy_panel::Action::Style) => self.toggle_dialog(energy_style::WINDOW),
            None => {}
        }
        changed | d.energy.is_some()
    }

    /// The list of saved places, or a notice, below where the search box would be.
    fn side_panel_text(&self) -> String {
        if self.search.is_open() {
            return String::new();
        }
        if self.services.is_open() {
            return self.services.panel_text();
        }
        if self.place_list.is_open() {
            let list = self.places.places();
            let mut text = self.place_list.panel_text(
                "Saved places",
                list.len(),
                |i| self.place_row(&list[i]),
                "None yet. Ctrl-S saves the view.",
                "Enter: fly there   Delete: remove   Up/Down: choose   K or Esc: close",
            );
            if let Some(n) = &self.notice {
                text.push('\n');
                text.push_str(n);
            }
            return text;
        }
        if self.events.picker.is_open() {
            let mut text = self.events.panel_text("Events");
            for (_, problem) in &self.feed_problems {
                text.push('\n');
                text.push_str(problem);
            }
            if self.feeds_pending > 0 {
                text.push_str("\nAsking for events...");
            }
            if let Some(n) = &self.notice {
                text.push('\n');
                text.push_str(n);
            }
            return text;
        }
        if self.import_list.is_open() {
            let library = &self.library;
            let mut text = self.import_list.panel_text(
                "Imported items",
                library.item_count(),
                |i| library.item(i).map_or_else(String::new, |(d, item)| format!("{}: {}", d.name, item.title())),
                "Nothing imported. Drop a file on the window, or start with --import FILE.",
                "Enter: fly there   Up/Down: choose   I or Esc: close",
            );
            if let Some(n) = &self.notice {
                text.push('\n');
                text.push_str(n);
            }
            return text;
        }
        self.notice.clone().unwrap_or_default()
    }

    fn place_row(&self, p: &places::SavedPlace) -> String {
        match body::by_id(&p.body) {
            Some(b) if b.id == body::current().id => p.name.clone(),
            Some(b) => format!("{} ({})", p.name, b.name),
            None => format!("{} ({})", p.name, p.body),
        }
    }

    /// Opens or closes the list of map services.
    fn toggle_services(&mut self) {
        self.notice = None;
        self.events.picker.close();
        self.import_list.close();
        self.place_list.close();
        self.services.toggle();
    }

    /// Opens or closes the list of saved places.
    fn toggle_places(&mut self) {
        self.notice = None;
        self.events.picker.close();
        self.import_list.close();
        self.services.picker.close();
        self.place_list.toggle();
    }

    /// Keeps the current view as a saved place. An empty name becomes the position.
    fn save_view(&mut self, name: &str) {
        let (lat, lon, distance, heading, tilt) = match self.mode {
            Mode::Globe => {
                let c = &self.nav.cam;
                (c.lat, c.lon, c.distance, c.heading, c.tilt)
            }
            Mode::Atlas => {
                let (lat, lon) = self.map.view.centre_lat_lon();
                (lat, lon, self.map.view.distance_for_scale(), 0.0, 0.0)
            }
        };
        let b = body::current();
        let name = if name.is_empty() { places::default_name(b.name, b.id == body::EARTH.id, lat, lon) } else { name.to_string() };
        let saved = places::SavedPlace { name: name.clone(), body: b.id.to_string(), lat, lon, distance, heading, tilt };
        self.notice = Some(match self.places.add(saved) {
            Ok(()) => format!("Saved place: {name}"),
            Err(e) => e,
        });
    }

    fn places_key(&mut self, key: places::Key) {
        let pick = self.place_list.key(key, self.places.places().len());
        self.notice = None;
        match pick {
            places::Pick::None => {}
            places::Pick::Fly(i) => {
                let Some(p) = self.places.places().get(i).cloned() else { return };
                if p.body != body::current().id {
                    let name = body::by_id(&p.body).map_or(p.body.as_str(), |b| b.name);
                    self.notice = Some(format!("{} is on {name}. Start with --body {} to go there.", p.name, p.body));
                    return;
                }
                self.fly_camera(&p);
            }
            places::Pick::Delete(i) => {
                if let Err(e) = self.places.remove(i) {
                    self.notice = Some(e);
                }
                self.place_list.clamp(self.places.places().len());
            }
        }
    }

    /// Flies to a saved view: with its heading and tilt on the globe, to its place on the map.
    fn fly_camera(&mut self, p: &places::SavedPlace) {
        match self.mode {
            Mode::Globe => {
                let c = self.nav.cam;
                self.nav.fly_to(Camera { lat: p.lat, lon: p.lon, distance: p.distance, heading: p.heading, tilt: p.tilt, sky: 0.0, ..c });
            }
            Mode::Atlas => self.map.view.match_globe(p.lat, p.lon, p.distance),
        }
    }

    /// Carries out what the search box decided.
    fn search_action(&mut self, action: search::Action) {
        match action {
            search::Action::None => {}
            search::Action::Request(query) => self.geocoder.submit(&query),
            search::Action::Fly(t) => self.fly_to(t),
            search::Action::Save(name) => self.save_view(&name),
            search::Action::AddService(typed) => {
                self.services.start_add(&self.fetcher, &typed);
                self.services.picker.open_at(0);
            }
        }
    }

    fn fly_to(&mut self, t: search::Target) {
        match self.mode {
            Mode::Globe => {
                let c = self.nav.cam;
                self.nav.fly_to(Camera { lat: t.lat, lon: t.lon, distance: t.distance, heading: 0.0, tilt: 0.0, sky: 0.0, ..c });
            }
            Mode::Atlas => self.map.view.match_globe(t.lat, t.lon, t.distance),
        }
    }

    /// Types `query` into the search box and presses Enter, as for a saved image: waits for the
    /// answer, then flies to the first result. With `keep_panel` the box stays open.
    fn headless_search(&mut self, query: &str, keep_panel: bool, timeout: f64) {
        self.search.open();
        let mut action = self.search.key(search::Key::Text(query.to_string()));
        if action == search::Action::None {
            action = self.search.key(search::Key::Enter);
        }
        self.search_action(action);
        let deadline = Instant::now() + Duration::from_secs_f64(timeout);
        while self.search.is_searching() && Instant::now() < deadline {
            self.poll_search();
            std::thread::sleep(Duration::from_millis(20));
        }
        // Coordinates have flown already. A named search has its list; the first result is chosen.
        if self.search.is_open() {
            eprintln!("search: {}", self.search.panel_text().lines().nth(1).unwrap_or("no answer"));
            if keep_panel {
                if let Some(t) = self.search.selected_target() {
                    self.fly_to(t);
                }
            } else {
                let action = self.search.key(search::Key::Enter);
                self.search_action(action);
            }
        }
        // The flight is played out here so that the picture shows its end.
        for _ in 0..1000 {
            if !self.nav.is_animating() {
                break;
            }
            self.nav.tick(0.05, &|_, _| None);
        }
        if keep_panel {
            self.search.open();
        }
    }

    /// Hands answers from the search service to the box. True if something changed.
    fn poll_search(&mut self) -> bool {
        let mut changed = false;
        while let Some(reply) = self.geocoder.poll() {
            self.search.arrived(&reply.query, reply.answer);
            changed = true;
        }
        changed
    }

    /// Switches between the globe and the flat map, keeping the place and the scale.
    fn toggle_mode(&mut self) {
        let view = &mut self.map.view;
        match self.mode {
            Mode::Globe => {
                view.match_globe(self.nav.cam.lat, self.nav.cam.lon, self.nav.cam.distance);
                self.mode = Mode::Atlas;
            }
            Mode::Atlas => {
                let (lat, lon) = view.centre_lat_lon();
                let cam = &mut self.nav.cam;
                (cam.lat, cam.lon, cam.heading, cam.tilt) = (lat, lon, 0.0, 0.0);
                cam.distance = view.distance_for_scale();
                cam.clamp();
                self.nav.stop();
                self.mode = Mode::Globe;
            }
        }
        self.map.release();
        self.map.stop();
    }

    /// A SpaceMouse button: the same as Ctrl-0 and Ctrl-Alt-N.
    fn spacemouse_action(&mut self, a: spacemouse::Action) {
        let atlas = self.mode == Mode::Atlas;
        match a {
            spacemouse::Action::WholeBody if atlas => self.map.key(atlas_nav::Key::Whole),
            spacemouse::Action::WholeBody => self.nav.whole_body(),
            spacemouse::Action::Straighten if !atlas => self.nav.straighten(),
            spacemouse::Action::Straighten => {}
        }
    }

    /// Hands the renderer the flat map's meshes, or tells it to draw the globe.
    fn prepare_draw(&self, r: &mut Renderer) {
        r.set_atlas((self.mode == Mode::Atlas).then_some((&self.map.view, self.draw.as_slice())));
    }

    /// True about once a second while tiles wait out a retry delay, which is often enough to
    /// notice one coming due without redrawing for the whole of a server's timeout.
    fn retry_tick(&mut self, now: Instant) -> bool {
        if self.retrying == 0 || now.duration_since(self.last_retry_tick) < Duration::from_secs(1) {
            return false;
        }
        self.last_retry_tick = now;
        true
    }
}

fn format_distance(m: f64) -> String {
    if m >= 10_000.0 {
        format!("{:.0} km", m / 1000.0)
    } else if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{m:.0} m")
    }
}

async fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    // A browser's limits are its own, often below the native defaults.
    #[cfg(target_arch = "wasm32")]
    let required_limits = adapter.limits();
    #[cfg(not(target_arch = "wasm32"))]
    let required_limits = wgpu::Limits::default();
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("orbis"),
            required_limits,
            ..Default::default()
        })
        .await
        .context("no suitable graphics device")
}

/// The graphics adapter and device for a window's surface.
async fn open_gpu(instance: &wgpu::Instance, surface: &wgpu::Surface<'static>) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(surface),
            ..Default::default()
        })
        .await
        .context("no graphics adapter for this window")?;
    let (device, queue) = request_device(&adapter).await?;
    Ok((adapter, device, queue))
}

/// Whether a saved image must keep waiting. A flight in progress counts: the tiles of the view it
/// ends in are not asked for until it gets there, so a quiet loader mid-flight proves nothing.
fn screenshot_busy(loader_idle: bool, waiting: usize, text_idle: bool, feeds_pending: usize, animating: bool, click_pending: bool) -> bool {
    !loader_idle || waiting > 0 || !text_idle || feeds_pending > 0 || animating || click_pending
}

#[cfg(not(target_arch = "wasm32"))]
fn screenshot(args: &Args, path: &PathBuf) -> Result<()> {
    let (width, height) = args.size;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .context("no graphics adapter")?;
    let (device, queue) = pollster::block_on(request_device(&adapter))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let samples = render::sample_count(&adapter, format);
    let mut r = Renderer::new(device, queue, format, samples);
    if args.mark_underlay {
        r.mark_underlay();
    }
    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("screenshot"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let aspect = width as f64 / height as f64;
    let mut scene = Scene::new(args, DVec2::new(width as f64, height as f64));
    if let Some(t) = args.eclipse {
        scene.jump_to_eclipse(t);
        // A time or a search given as well decides the picture's moment and view.
        if let Some(time) = args.time {
            scene.clock.stop_at(time);
        }
    }
    scene.start_feeds(args);
    if args.events_panel {
        scene.toggle_events();
    }
    if let Some(query) = &args.search {
        scene.headless_search(query, args.search_panel, args.timeout);
    }
    if let Some(name) = &args.save_place {
        scene.save_view(name);
        eprintln!("{}", scene.notice.as_deref().unwrap_or(""));
    }
    if args.imports_panel {
        scene.import_list.toggle();
        scene.notice = None;
    }
    if args.places_panel {
        scene.place_list.toggle();
        scene.notice = None;
    }
    if args.eclipses_panel {
        scene.toggle_events();
    }
    if args.services.panel {
        scene.services.toggle();
    }
    if args.show_options {
        scene.gui.set_visible(options::WINDOW, true);
    }
    if args.show_energy_style {
        scene.sync_energy_style();
        scene.gui.set_visible(energy_style::WINDOW, true);
    }
    if args.show_time {
        scene.gui.set_visible(time_dialog::WINDOW, true);
    }
    if args.show_help {
        scene.gui.set_visible(help::WINDOW, true);
        // A notch of the wheel scrolls about three lines.
        scene.help_scroll = Some((args.help_scroll * 50.0) as f32);
    }
    if let Some(script) = &args.simulate_nav {
        navtrace::run(args, script, &mut scene, &mut r, &target, (width, height))?;
    }
    // Clicks are made once everything has loaded, so they land on what the picture shows: the
    // credits, which dialogs sit above, depend on the tiles that arrived.
    let mut clicks = args.simulate_clicks.iter();
    let mut doubles = args.simulate_doubles.iter();
    let mut drags = args.simulate_drags.iter();
    // Seconds of flight played so far, while a flight is to be stopped in its middle.
    let mut flight_clock: Option<f64> = None;

    let started = Instant::now();
    let mut quiet_since = Instant::now();
    let step = 1.0 / 30.0;
    loop {
        scene.update(&mut r, step, (aspect, height as f64));
        scene.set_panels(&r, width, height, 1.0);
        scene.place_labels(&mut r, (width, height), 1, step);
        r.flush_uploads();
        scene.poll_feeds();
        if let (Some(clock), Some(stop)) = (flight_clock.as_mut(), args.flight_stop_at) {
            *clock += step;
            if *clock >= stop {
                scene.nav.freeze_flight();
                flight_clock = None;
            }
        }
        // A single click is not acted on until the double-click window has passed, and the box it opens waits for its text.
        scene.poll_clicks();
        scene.poll_info();
        let click_pending = scene.clicks.deadline().is_some() || scene.info_pending;
        if screenshot_busy(scene.loader.is_idle(), scene.waiting, scene.text.is_idle(), scene.feeds_pending, scene.nav.is_animating() || scene.map.is_animating(), click_pending) {
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() > Duration::from_millis(300) {
            let pending: Vec<DVec2> = clicks.by_ref().copied().collect();
            let double: Vec<DVec2> = doubles.by_ref().copied().collect();
            let dragged: Vec<(DVec2, DVec2)> = drags.by_ref().copied().collect();
            if pending.is_empty() && double.is_empty() && dragged.is_empty() {
                break;
            }
            // Clicks land on the labels the picture will show, not on an earlier choice.
            scene.settle_labels(&mut r, (width, height), step);
            for pos in pending {
                scene.simulate_click(pos);
            }
            for (from, to) in dragged {
                scene.simulate_drag(from, to);
            }
            for pos in double {
                scene.simulate_double_click(pos);
                flight_clock = args.flight_stop_at.map(|_| 0.0);
            }
            scene.persist();
            quiet_since = Instant::now();
        }
        if started.elapsed().as_secs_f64() > args.timeout {
            eprintln!("timed out with {} tiles outstanding", scene.waiting);
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    // A few steps let the view point settle on the terrain.
    for _ in 0..90 {
        scene.update(&mut r, step, (aspect, height as f64));
    }
    // Windows are measured in their first frame and drawn from the next.
    for _ in 0..3 {
        scene.set_panels(&r, width, height, 1.0);
    }
    scene.settle_labels(&mut r, (width, height), step);

    let frame_started = Instant::now();
    scene.update(&mut r, 0.0, (aspect, height as f64));
    scene.place_labels(&mut r, (width, height), 1, 0.0);
    let frame_cpu = frame_started.elapsed();

    let bytes_per_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let draw = scene.draw.clone();
    scene.prepare_draw(&mut r);
    scene.gui.prepare(&r.device, &r.queue, format);
    let gpu_started = Instant::now();
    let gui = &scene.gui;
    r.draw(&view, (width, height), &scene.nav.cam, scene.nav.altitude, &draw, |encoder| {
        gui.render(encoder, &view);
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    });
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device.poll(wgpu::PollType::wait_indefinitely()).context("waiting for the GPU")?;
    let frame_gpu = gpu_started.elapsed();
    let data = slice.get_mapped_range().map_err(|e| anyhow!("reading the image back: {e:?}"))?;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in data.chunks(bytes_per_row as usize) {
        pixels.extend_from_slice(&row[..(width * 4) as usize]);
    }
    image::save_buffer(path, &pixels, width, height, image::ExtendedColorType::Rgba8)
        .with_context(|| format!("writing {}", path.display()))?;
    println!(
        "{}: {} tiles drawn, finest z{}, {} labels, {} building tiles, {:.1} ms frame logic, {:.1} ms drawing, {} tiles missing on the server, adapter {}",
        path.display(),
        draw.len(),
        draw.iter().map(|k| k.z).max().unwrap_or(0),
        scene.labels_shown,
        r.visible_building_count(),
        frame_cpu.as_secs_f64() * 1000.0,
        frame_gpu.as_secs_f64() * 1000.0,
        scene.missing.len(),
        adapter.get_info().name,
    );
    let t = scene.fetcher.traffic();
    println!(
        "traffic: {} downloads ({}), {} from disk, {} from LAN, {} shared downloads",
        t.downloads,
        format_bytes(t.downloaded_bytes),
        t.disk_hits,
        t.lan_hits,
        t.shared_downloads,
    );
    for r in scene.fetcher.server_report() {
        if let Some(until) = r.paused_until {
            println!("timed out: {} for {} min", r.host, minutes_until(until, SystemTime::now()));
        }
    }
    Ok(())
}

/// What a recording notes about a drawn frame. Angles are in degrees and distances in metres,
/// as the command line takes them, so that a frame's view can be asked for again.
fn frame_state(scene: &Scene, size: (u32, u32), dt: f64, draw: &[TileKey]) -> serde_json::Value {
    let cam = &scene.nav.cam;
    let (map_lat, map_lon) = scene.map.view.centre_lat_lon();
    serde_json::json!({
        "dt": dt,
        "size": [size.0, size.1],
        "mode": format!("{:?}", scene.mode),
        "body": body::current().name,
        "camera": {
            "lat": cam.lat.to_degrees(),
            "lon": cam.lon.to_degrees(),
            "target_h": cam.target_h,
            "distance": cam.distance,
            "heading": cam.heading.to_degrees(),
            "tilt": cam.tilt.to_degrees(),
            "sky": cam.sky.to_degrees(),
        },
        "altitude": scene.nav.altitude,
        "lifted": scene.nav.lifted,
        "map": {
            "lat": map_lat.to_degrees(),
            "lon": map_lon.to_degrees(),
            "metres_per_pixel": scene.map.view.metres_per_pixel(),
            "projection": scene.map.view.projection.name(),
        },
        "clock": scene.clock.label(),
        "tiles_drawn": draw.len(),
        "finest_z": draw.iter().map(|k| k.z).max(),
        "tiles_waiting": scene.waiting,
        "tiles_retrying": scene.retrying,
        "loader_idle": scene.loader.is_idle(),
        "notice": scene.notice,
    })
}

struct Gfx {
    window: Arc<Window>,
    /// The format frames are drawn in: the surface's, or an sRGB view of it.
    draw_format: wgpu::TextureFormat,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    scene: Scene,
    last_frame: Instant,
    pacer: Pacer,
    meter: RateMeter,
    /// Something changed, so a frame should be drawn at the next allowed moment.
    redraw_wanted: bool,
    present_mode: wgpu::PresentMode,
    benchmark: Option<Benchmark>,
    recorder: Option<recorder::Recorder>,
}

/// Turns the view steadily, so that frame timing can be measured without any input.
struct Benchmark {
    end: Instant,
    last: Option<Instant>,
    intervals: Vec<f64>,
}

/// Events sent to the window from other threads.
#[derive(Debug)]
enum UserEvent {
    SpaceMouse,
    /// The browser has opened the graphics device.
    #[cfg(target_arch = "wasm32")]
    GpuReady,
    /// A request made from the page was answered.
    #[cfg(target_arch = "wasm32")]
    Answer,
}

#[cfg(target_arch = "wasm32")]
type OpenedGpu = Result<(Arc<Window>, wgpu::Surface<'static>, (wgpu::Adapter, wgpu::Device, wgpu::Queue))>;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PENDING_GPU: std::cell::RefCell<Option<OpenedGpu>> = const { std::cell::RefCell::new(None) };
}

struct App {
    args: Args,
    proxy: EventLoopProxy<UserEvent>,
    spacemouse: Option<spacemouse::Handle>,
    gfx: Option<Gfx>,
    error: Option<anyhow::Error>,
    mods: Mods,
    /// The browser is still opening the graphics device.
    #[cfg(target_arch = "wasm32")]
    opening: bool,
}

impl App {
    /// Opens the window and its surface; the graphics device comes after, in `finish_start`.
    fn open_window(&self, event_loop: &ActiveEventLoop) -> Result<Arc<Window>> {
        let (width, height) = if self.args.size_given { self.args.size } else { (1280, 800) };
        let attributes = Window::default_attributes().with_title("Orbis");
        // In a browser, the page decides the canvas's size.
        #[cfg(not(target_arch = "wasm32"))]
        let attributes = attributes.with_inner_size(PhysicalSize::new(width, height));
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            let _ = (width, height);
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes.with_canvas(web::canvas()).with_prevent_default(true).with_focusable(true)
        };
        Ok(Arc::new(event_loop.create_window(attributes)?))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&self, event_loop: &ActiveEventLoop) -> Result<Gfx> {
        let window = self.open_window(event_loop)?;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone())?;
        let (adapter, device, queue) = pollster::block_on(open_gpu(&instance, &surface))?;
        self.finish_start(window, surface, adapter, device, queue)
    }

    fn finish_start(
        &self,
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Gfx> {
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let mut config = surface
            .get_default_config(&adapter, w, h)
            .ok_or_else(|| anyhow!("this window's surface is not supported"))?;
        let caps = surface.get_capabilities(&adapter);
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);
        // A browser's canvas has no sRGB format, but can be drawn through an sRGB view of it.
        let draw_format = config.format.add_srgb_suffix();
        if draw_format != config.format {
            config.view_formats.push(draw_format);
        }
        config.present_mode = pacing::present_mode(&caps.present_modes);
        if self.args.record.is_some() {
            if !caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                bail!("--record cannot read this window's surface on this graphics adapter");
            }
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&device, &config);
        let samples = render::sample_count(&adapter, draw_format);
        let recorder = match &self.args.record {
            Some(dir) => Some(recorder::Recorder::start(dir, self.args.record_limit_gib, device.clone(), config.format)?),
            None => None,
        };
        let renderer = Renderer::new(device, queue, draw_format, samples);
        let mut scene = Scene::new(&self.args, DVec2::new(w as f64, h as f64));
        scene.start_feeds(&self.args);
        // Dark or light as the desktop is; without a word from it, dark suits the imagery.
        scene.gui.set_dark(window.theme() != Some(winit::window::Theme::Light));
        let now = Instant::now();
        let benchmark = self.args.benchmark.map(|seconds| Benchmark {
            end: now + Duration::from_secs_f64(seconds),
            last: None,
            intervals: Vec::new(),
        });
        Ok(Gfx {
            draw_format,
            present_mode: config.present_mode,
            window,
            surface,
            config,
            renderer,
            scene,
            last_frame: now,
            pacer: Pacer::new(self.args.settings.frame_rate, now),
            meter: RateMeter::default(),
            redraw_wanted: true,
            benchmark,
            recorder,
        })
    }

    fn redraw(gfx: &mut Gfx) {
        // A page's canvas may have been resized while the graphics device was still opening.
        #[cfg(target_arch = "wasm32")]
        {
            if let Some((width, height)) = web::fit_canvas().filter(|&s| s != (gfx.config.width, gfx.config.height)) {
                gfx.config.width = width;
                gfx.config.height = height;
                gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                gfx.scene.nav.size = DVec2::new(width as f64, height as f64);
            }
        }
        let now = Instant::now();
        gfx.pacer.frame_started(now);
        gfx.meter.record(now);
        let dt = gfx.last_frame.elapsed().as_secs_f64().min(0.1);
        gfx.last_frame = now;
        if let Some(b) = &mut gfx.benchmark {
            if let Some(last) = b.last {
                b.intervals.push((now - last).as_secs_f64());
            }
            b.last = Some(now);
            gfx.scene.nav.cam.heading += dt * 0.15;
        }
        let display_hz = gfx
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| mhz as f64 / 1000.0);
        gfx.scene.frame_status =
            Some(pacing::status_line(gfx.pacer.limit(), gfx.meter.rate(), display_hz));
        // In a browser, the work threads would do runs here, in the frame's spare time.
        #[cfg(target_arch = "wasm32")]
        {
            web::set_budget(Duration::from_millis(6));
            gfx.scene.loader.pump();
            web::set_budget(Duration::from_millis(4));
        }
        let (w, h) = (gfx.config.width, gfx.config.height);
        let aspect = w as f64 / h as f64;
        gfx.scene.update(&mut gfx.renderer, dt, (aspect, h as f64));
        let scale = gfx.window.scale_factor().round().max(1.0) as usize;
        if gfx.scene.set_panels(&gfx.renderer, w, h, gfx.window.scale_factor() as f32) {
            gfx.redraw_wanted = true;
        }
        gfx.scene.place_labels(&mut gfx.renderer, (w, h), scale as u32, dt);

        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                gfx.redraw_wanted = true;
                return;
            }
            other => {
                if !matches!(other, wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout) {
                    eprintln!("could not get a frame from the window");
                }
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(gfx.draw_format), ..Default::default() });
        let draw = gfx.scene.draw.clone();
        gfx.scene.prepare_draw(&mut gfx.renderer);
        gfx.scene.gui.prepare(&gfx.renderer.device, &gfx.renderer.queue, gfx.draw_format);
        let (cam, altitude) = (gfx.scene.nav.cam, gfx.scene.nav.altitude);
        let (number, slot) = gfx.recorder.as_mut().map_or((0, None), |rec| rec.begin(w, h));
        let gui = &gfx.scene.gui;
        gfx.renderer.draw(&view, (w, h), &cam, altitude, &draw, |encoder| {
            gui.render(encoder, &view);
            if let Some(slot) = &slot {
                slot.copy(encoder, &frame.texture);
            }
        });
        gfx.window.pre_present_notify();
        gfx.renderer.queue.present(frame);
        if let Some(rec) = &mut gfx.recorder {
            let has_image = slot.is_some();
            if let Some(slot) = slot {
                rec.submitted(slot);
            }
            rec.log_frame(number, has_image, frame_state(&gfx.scene, (w, h), dt, &draw));
        }

        // Tiles only waiting out a retry delay are picked up by the slow tick instead.
        let scene = &gfx.scene;
        let loading = !scene.loader.is_idle() || scene.waiting > scene.retrying;
        if scene.nav.is_animating() || scene.map.is_animating() || scene.clock.is_fast() || loading || scene.labels_changing() || scene.gui.animating() || gfx.benchmark.is_some() {
            gfx.redraw_wanted = true;
        }
    }

    /// Notes an input in the recording, before it is acted on. What is typed into a text box
    /// stays out of the log, and the pointer's way is kept with the frames, not move by move.
    fn record_input(gfx: &mut Gfx, mods: Mods, event: &WindowEvent) {
        let typing = gfx.scene.search.is_open();
        let Some(rec) = &mut gfx.recorder else { return };
        let mods = serde_json::json!({ "shift": mods.shift, "alt": mods.alt, "ctrl": mods.ctrl });
        let input = match event {
            WindowEvent::CursorMoved { position, .. } => {
                rec.cursor = Some((position.x, position.y));
                return;
            }
            WindowEvent::CursorLeft { .. } => {
                rec.cursor = None;
                return;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                serde_json::json!({ "kind": "button", "button": format!("{button:?}"), "state": format!("{state:?}"), "mods": mods, "cursor": rec.cursor })
            }
            WindowEvent::MouseWheel { delta, .. } => {
                serde_json::json!({ "kind": "wheel", "delta": format!("{delta:?}"), "mods": mods, "cursor": rec.cursor })
            }
            WindowEvent::KeyboardInput { event, .. } if typing => {
                serde_json::json!({ "kind": "typing", "state": format!("{:?}", event.state) })
            }
            WindowEvent::KeyboardInput { event, .. } => {
                serde_json::json!({ "kind": "key", "key": format!("{:?}", event.physical_key), "state": format!("{:?}", event.state), "repeat": event.repeat, "mods": mods })
            }
            WindowEvent::Resized(size) => serde_json::json!({ "kind": "resized", "width": size.width, "height": size.height }),
            WindowEvent::Focused(focused) => serde_json::json!({ "kind": "focused", "focused": focused }),
            WindowEvent::CloseRequested => serde_json::json!({ "kind": "close" }),
            _ => return,
        };
        rec.log_input(input);
    }

    /// Prints the measurements of a finished benchmark.
    fn report_benchmark(gfx: &Gfx, b: &Benchmark) {
        let display = gfx
            .window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map_or("unknown".to_string(), |mhz| format!("{:.1} Hz", mhz as f64 / 1000.0));
        println!(
            "limit {} fps, present mode {:?}, display {display}, {}x{}",
            gfx.pacer.limit().hz(),
            gfx.present_mode,
            gfx.config.width,
            gfx.config.height,
        );
        match pacing::summarize(&b.intervals) {
            Some(s) => println!(
                "{} frames, average {:.1} fps, median {:.2} ms, 99th percentile {:.2} ms, worst {:.2} ms",
                s.frames, s.average_fps, s.median_ms, s.p99_ms, s.worst_ms,
            ),
            None => println!("no frames were drawn; the window may have been hidden"),
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    #[cfg(target_arch = "wasm32")]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() || self.opening {
            return;
        }
        self.opening = true;
        let opened = self.open_window(event_loop);
        let proxy = self.proxy.clone();
        let wake = proxy.clone();
        web::set_waker(move || {
            let _ = wake.send_event(UserEvent::Answer);
        });
        wasm_bindgen_futures::spawn_local(async move {
            let result = async {
                let window = opened?;
                // WebGPU where the browser can do it, WebGL 2 otherwise.
                let instance = wgpu::util::new_instance_with_webgpu_detection(wgpu::InstanceDescriptor::new_without_display_handle()).await;
                let surface = instance.create_surface(window.clone())?;
                let gpu = open_gpu(&instance, &surface).await?;
                let info = gpu.0.get_info();
                web::log(&format!("graphics: {} through {:?}", info.name, info.backend));
                Ok((window, surface, gpu))
            }
            .await;
            PENDING_GPU.with(|p| *p.borrow_mut() = Some(result));
            let _ = proxy.send_event(UserEvent::GpuReady);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match self.start(event_loop) {
            Ok(g) => {
                if self.args.spacemouse && self.args.benchmark.is_none() && self.spacemouse.is_none() {
                    let proxy = self.proxy.clone();
                    self.spacemouse = Some(spacemouse::spawn(
                        spacemouse::Config::default(),
                        Arc::new(move || {
                            let _ = proxy.send_event(UserEvent::SpaceMouse);
                        }),
                    ));
                }
                g.window.request_redraw();
                self.gfx = Some(g);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        #[cfg(target_arch = "wasm32")]
        match event {
            UserEvent::GpuReady => {
                let Some(result) = PENDING_GPU.with(|p| p.borrow_mut().take()) else { return };
                match result.and_then(|(window, surface, (adapter, device, queue))| self.finish_start(window, surface, adapter, device, queue)) {
                    Ok(g) => {
                        web::started();
                        g.window.request_redraw();
                        self.gfx = Some(g);
                    }
                    Err(e) => web::fail(&format!("{e:#}")),
                }
                return;
            }
            UserEvent::Answer => {
                if let Some(g) = &mut self.gfx {
                    g.redraw_wanted = true;
                }
                return;
            }
            UserEvent::SpaceMouse => {}
        }
        let (UserEvent::SpaceMouse, Some(gfx), Some(sm)) = (event, &mut self.gfx, &self.spacemouse) else { return };
        let u = sm.shared.drain();
        let scene = &mut gfx.scene;
        scene.nav.set_device(u.axes, self.args.spacemouse_sensitivity);
        for a in u.actions {
            scene.spacemouse_action(a);
        }
        if let Some(n) = u.notice {
            scene.notice = Some(n);
        }
        gfx.redraw_wanted = true;
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // A slow tick picks up tiles whose retry time has come.
        let mut wake = now + Duration::from_secs(1);
        if let Some(g) = &mut self.gfx {
            // Tiles keep loading between frames too, which a hidden page hardly draws.
            #[cfg(target_arch = "wasm32")]
            {
                web::set_budget(Duration::from_millis(8));
                g.scene.loader.pump();
                if !g.scene.loader.is_idle() {
                    wake = wake.min(now + Duration::from_millis(50));
                }
            }
            // Whatever the user changed, by key, click or list, is noted and saved here.
            g.scene.persist();
            if g.pacer.limit() != g.scene.view.frame_rate {
                g.pacer.set_limit(g.scene.view.frame_rate);
            }
            if let Some(b) = &g.benchmark {
                if now >= b.end {
                    Self::report_benchmark(g, b);
                    event_loop.exit();
                    return;
                }
            }
            // Refreshes the on-screen rate once drawing has stopped.
            if g.scene.retry_tick(now) || g.meter.is_stale(now) {
                g.redraw_wanted = true;
            }
            // A single click waits out the double-click window before it counts.
            if let Some(at) = g.scene.poll_clicks() {
                wake = wake.min(at);
            }
            if g.scene.info_pending {
                wake = wake.min(now + Duration::from_millis(100));
            }
            if g.scene.poll_info() {
                g.redraw_wanted = true;
            }
            // While a search is out, the answer is looked for often enough to feel instant.
            if g.scene.search.is_searching() {
                wake = wake.min(now + Duration::from_millis(100));
                if g.scene.poll_search() {
                    g.redraw_wanted = true;
                }
            }
            if g.scene.feeds_pending > 0 {
                wake = wake.min(now + Duration::from_millis(100));
                if g.scene.poll_feeds() {
                    g.redraw_wanted = true;
                }
            }
            if g.scene.services.is_adding() {
                wake = wake.min(now + Duration::from_millis(100));
                if g.scene.services.poll() {
                    g.redraw_wanted = true;
                }
            }
            // The Sun moves slowly at real speed, so it asks for a redraw only now and then.
            match g.scene.sky_redraw_in() {
                Some(s) if s <= 0.0 => g.redraw_wanted = true,
                Some(s) => wake = wake.min(now + Duration::from_secs_f64(s)),
                None => {}
            }
            if g.redraw_wanted {
                if g.pacer.due(now) {
                    g.redraw_wanted = false;
                    g.window.request_redraw();
                } else {
                    wake = g.pacer.next_frame_at();
                }
            }
            // Frames on their way are picked up even if nothing more is drawn.
            if g.recorder.as_mut().is_some_and(recorder::Recorder::collect) {
                wake = wake.min(now + Duration::from_millis(5));
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gfx) = &mut self.gfx else { return };
        Self::record_input(gfx, self.mods, &event);
        let nav = &mut gfx.scene.nav;
        let map = &mut gfx.scene.map;
        let atlas = gfx.scene.mode == Mode::Atlas;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                Self::redraw(gfx);
                return;
            }
            WindowEvent::ThemeChanged(theme) => {
                gfx.scene.gui.set_dark(theme != winit::window::Theme::Light);
                gfx.redraw_wanted = true;
            }
            // In a browser the canvas is fitted on each frame instead (see `redraw`).
            #[cfg(target_arch = "wasm32")]
            WindowEvent::Resized(_) => gfx.redraw_wanted = true,
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    gfx.config.width = size.width;
                    gfx.config.height = size.height;
                    gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                    nav.size = DVec2::new(size.width as f64, size.height as f64);
                }
            }
            WindowEvent::Focused(false) => {
                nav.release_all();
                map.release();
                gfx.scene.mouse_lost();
            }
            WindowEvent::CursorLeft { .. } => {
                if gfx.scene.map_buttons == 0 {
                    gfx.scene.gui.pointer_moved(DVec2::new(-1.0, -1.0));
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Mods { shift: s.shift_key(), alt: s.alt_key(), ctrl: s.control_key() };
            }
            WindowEvent::CursorMoved { position: PhysicalPosition { x, y }, .. } => {
                // Only a drag or a change in the interface needs a new picture.
                if !gfx.scene.mouse_moved(DVec2::new(x, y)) {
                    return;
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => return,
                };
                match state {
                    ElementState::Pressed => gfx.scene.mouse_pressed(b, self.mods),
                    ElementState::Released => gfx.scene.mouse_released(b, self.mods),
                };
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                if !gfx.scene.mouse_wheel(lines, self.mods) {
                    return;
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) if gfx.scene.search.is_open() => {
                let action = gfx.scene.search.key(search::Key::Text(text));
                gfx.scene.search_action(action);
            }
            WindowEvent::KeyboardInput { event, .. } if gfx.scene.search.is_open() => {
                // While the box is open it takes every key.
                if event.state == ElementState::Pressed {
                    if let Some(key) = search_key(&event, self.mods) {
                        let action = gfx.scene.search.key(key);
                        gfx.scene.search_action(action);
                        if !gfx.scene.search.is_open() {
                            gfx.window.set_ime_allowed(false);
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if gfx.scene.place_list.is_open() && list_key(&event).is_some() =>
            {
                if event.state == ElementState::Pressed {
                    if let Some(key) = list_key(&event) {
                        gfx.scene.places_key(key);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if gfx.scene.services.is_open() && service_key(&event).is_some() =>
            {
                if event.state == ElementState::Pressed {
                    if let Some(cmd) = service_key(&event) {
                        if let service_ui::Done::OpenAddBox = gfx.scene.services.key(cmd) {
                            nav.release_all();
                            gfx.scene.search.open_add();
                            gfx.window.set_ime_allowed(true);
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if gfx.scene.import_list.is_open() && list_key(&event).is_some() =>
            {
                if event.state == ElementState::Pressed {
                    if let Some(key) = list_key(&event) {
                        gfx.scene.imports_key(key);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if gfx.scene.events.picker.is_open() && list_key(&event).is_some() =>
            {
                if event.state == ElementState::Pressed {
                    if let Some(key) = list_key(&event) {
                        gfx.scene.events_key(key);
                    }
                }
            }
            WindowEvent::DroppedFile(path) => gfx.scene.import_path(&path, true),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let pressed = event.state == ElementState::Pressed;
                let arrow = match code {
                    KeyCode::ArrowUp => Some(Arrow::Up),
                    KeyCode::ArrowDown => Some(Arrow::Down),
                    KeyCode::ArrowLeft => Some(Arrow::Left),
                    KeyCode::ArrowRight => Some(Arrow::Right),
                    _ => None,
                };
                if arrow.is_some() && atlas {
                    // The map is moved by dragging.
                } else if let Some(a) = arrow {
                    if !pressed {
                        nav.arrow_up(a);
                    } else if !event.repeat {
                        nav.arrow_down(a, self.mods);
                    }
                } else if pressed && !event.repeat {
                    let question = matches!(&event.logical_key, winit::keyboard::Key::Character(c) if c.as_str() == "?");
                    match code {
                        _ if question => gfx.scene.toggle_dialog(help::WINDOW),
                        KeyCode::KeyH | KeyCode::F1 if !self.mods.ctrl => gfx.scene.toggle_dialog(help::WINDOW),
                        KeyCode::KeyV if !self.mods.ctrl => gfx.scene.toggle_dialog(options::WINDOW),
                        KeyCode::KeyC if !self.mods.ctrl => gfx.scene.toggle_dialog(time_dialog::WINDOW),
                        KeyCode::Escape if gfx.scene.gui.is_visible(time_dialog::WINDOW) => {
                            gfx.scene.gui.set_visible(time_dialog::WINDOW, false);
                            gfx.scene.clock.release();
                        }
                        KeyCode::Escape if gfx.scene.gui.is_visible(info::WINDOW) => {
                            gfx.scene.gui.set_visible(info::WINDOW, false);
                            gfx.scene.info_id += 1;
                            gfx.scene.info_pending = false;
                        }
                        KeyCode::Escape if gfx.scene.gui.is_visible(options::WINDOW) => {
                            gfx.scene.gui.set_visible(options::WINDOW, false);
                        }
                        KeyCode::Escape if gfx.scene.gui.is_visible(help::WINDOW) => {
                            gfx.scene.gui.set_visible(help::WINDOW, false);
                        }
                        KeyCode::F12 => gfx.scene.debug_readout = !gfx.scene.debug_readout,
                        KeyCode::Escape => {
                            nav.stop();
                            map.stop();
                            gfx.scene.notice = None;
                        }
                        KeyCode::Digit0 if self.mods.ctrl && atlas => map.key(atlas_nav::Key::Whole),
                        KeyCode::Digit0 if self.mods.ctrl => nav.whole_body(),
                        KeyCode::KeyF if self.mods.ctrl => {
                            nav.release_all();
                            gfx.scene.search.open();
                            gfx.window.set_ime_allowed(true);
                        }
                        KeyCode::KeyS if self.mods.ctrl => {
                            nav.release_all();
                            gfx.scene.notice = None;
                            gfx.scene.place_list.close();
                            gfx.scene.import_list.close();
                            gfx.scene.events.picker.close();
                            gfx.scene.search.open_save();
                            gfx.window.set_ime_allowed(true);
                        }
                        KeyCode::KeyK if !self.mods.ctrl => gfx.scene.toggle_places(),
                        KeyCode::KeyG if !self.mods.ctrl => gfx.scene.toggle_services(),
                        KeyCode::KeyI if !self.mods.ctrl => gfx.scene.toggle_imports(),
                        KeyCode::KeyE if !self.mods.ctrl => gfx.scene.toggle_events(),
                        KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5
                            if !self.mods.ctrl && gfx.scene.events.picker.is_open() =>
                        {
                            let n = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5]
                                .iter()
                                .position(|k| *k == code);
                            if let Some(n) = n {
                                gfx.scene.events.toggle_filter(n);
                            }
                        }
                        KeyCode::KeyM if !self.mods.ctrl => gfx.scene.toggle_mode(),
                        _ if !self.mods.ctrl && sky_time::key_for(code, self.mods.shift).is_some() => {
                            if let Some(key) = sky_time::key_for(code, self.mods.shift) {
                                gfx.scene.sky_key(key);
                            }
                        }
                        KeyCode::KeyP if !self.mods.ctrl && atlas => {
                            map.key(atlas_nav::Key::CycleProjection { back: self.mods.shift });
                        }
                        KeyCode::BracketLeft | KeyCode::BracketRight if atlas => {
                            let east = code == KeyCode::BracketRight;
                            map.key(atlas_nav::Key::ShiftMeridian { east, fine: self.mods.shift });
                        }
                        KeyCode::KeyN if self.mods.ctrl && self.mods.alt => nav.straighten(),
                        KeyCode::KeyQ if self.mods.ctrl => event_loop.exit(),
                        KeyCode::KeyR if !self.mods.ctrl => {
                            gfx.scene.toggle_lines();
                        }
                        KeyCode::KeyF if !self.mods.ctrl => {
                            gfx.pacer.toggle();
                            gfx.scene.view.frame_rate = gfx.pacer.limit();
                        }
                        KeyCode::KeyB if !self.mods.ctrl => {
                            gfx.scene.view.buildings = !gfx.scene.view.buildings;
                        }
                        KeyCode::KeyW if !self.mods.ctrl => {
                            gfx.scene.view.energy = !gfx.scene.view.energy;
                        }
                        KeyCode::KeyO if !self.mods.ctrl => {
                            gfx.scene.view.stars = !gfx.scene.view.stars;
                        }
                        KeyCode::KeyL if !self.mods.ctrl => {
                            gfx.scene.toggle_names();
                        }
                        _ => {}
                    }
                }
            }
            _ => return,
        }
        gfx.redraw_wanted = true;
    }
}

/// What a key press means to an open list of places, if anything.
fn list_key(event: &KeyEvent) -> Option<places::Key> {
    use winit::keyboard::Key;
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => Some(places::Key::Escape),
        Key::Named(NamedKey::Enter) => Some(places::Key::Enter),
        Key::Named(NamedKey::Delete | NamedKey::Backspace) => Some(places::Key::Delete),
        Key::Named(NamedKey::ArrowUp) => Some(places::Key::Up),
        Key::Named(NamedKey::ArrowDown) => Some(places::Key::Down),
        _ => None,
    }
}

/// What a key press means to the list of map services, if anything.
fn service_key(event: &KeyEvent) -> Option<service_ui::Cmd> {
    use winit::keyboard::Key;
    use service_ui::Cmd;
    if let Some(key) = list_key(event) {
        return Some(Cmd::List(key));
    }
    match &event.logical_key {
        Key::Character(c) => match c.as_str() {
            "o" | "O" => Some(Cmd::ToggleOverlay),
            "+" | "=" => Some(Cmd::Louder),
            "-" | "_" => Some(Cmd::Quieter),
            "a" | "A" => Some(Cmd::Add),
            "g" | "G" => Some(Cmd::List(places::Key::Escape)),
            _ => None,
        },
        _ => None,
    }
}

/// What a key press means to the search box, if anything.
fn search_key(event: &KeyEvent, mods: Mods) -> Option<search::Key> {
    use winit::keyboard::Key;
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => Some(search::Key::Escape),
        Key::Named(NamedKey::Enter) => Some(search::Key::Enter),
        Key::Named(NamedKey::Backspace) => Some(search::Key::Backspace),
        Key::Named(NamedKey::ArrowUp) => Some(search::Key::Up),
        Key::Named(NamedKey::ArrowDown) => Some(search::Key::Down),
        Key::Named(NamedKey::Space) if !mods.ctrl => Some(search::Key::Text(" ".into())),
        _ if mods.ctrl => None,
        _ => event.text.as_ref().map(|t| search::Key::Text(t.to_string())),
    }
}

/// In a browser, the page's address holds the options: `?lat=46.5&lon=11.8` is `--lat 46.5 --lon 11.8`.
#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    let argv = web::address_args();
    let store = settings::Store::in_memory();
    let mut args = match parse_args(argv.into_iter(), &store.values) {
        Ok(Some(a)) => a,
        Ok(None) => return web::fail(USAGE),
        Err(e) => return web::fail(&format!("{e:#}")),
    };
    body::select(args.body);
    if !args.distance_given {
        args.cam.distance = nav::whole_body();
    }
    // Nothing is kept between visits, and nothing is shared on a LAN.
    args.spacemouse = false;
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => return web::fail(&format!("{e}")),
    };
    let app = App {
        args,
        proxy: event_loop.create_proxy(),
        spacemouse: None,
        gfx: None,
        error: None,
        mods: Mods::default(),
        opening: false,
    };
    use winit::platform::web::EventLoopExtWebSys;
    event_loop.spawn_app(app);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (dir, flag) = settings_dir(&argv);
    let store = dir.as_deref().map_or_else(settings::Store::in_memory, settings::Store::open);
    for w in &store.warnings {
        eprintln!("settings: {w}");
    }
    let Some(mut args) = parse_args(argv.into_iter(), &store.values)? else {
        println!("{USAGE}");
        return Ok(());
    };
    args.config_dir = dir;
    args.config_flag = flag;
    if args.settings.body != args.body.id {
        // Only reached by a flag that is not a body: the saved name is unknown.
        args.settings.body = args.body.id.to_string();
    }
    if args.services.base.is_none() && !store.values.base_service.is_empty() {
        if service_ui::Control::new(args.services.library(), true).has(&store.values.base_service) {
            args.services.base = Some(store.values.base_service.clone());
        } else {
            eprintln!("settings: the map service {:?} is not in your library, using the usual imagery", store.values.base_service);
        }
    }
    body::select(args.body);
    if !args.distance_given {
        args.cam.distance = nav::whole_body();
    }
    if args.cache_status {
        cache_status(&args);
        return Ok(());
    }
    if args.services.list {
        print!("{}", args.services.list_text());
        return Ok(());
    }
    if args.services.add.is_some() {
        let fetcher = make_fetcher(&args);
        println!("{}", args.services.run_add(&fetcher).map_err(|e| anyhow!(e))?);
        return Ok(());
    }
    // A service name that fits nothing is reported before any window or picture.
    service_ui::Control::new(args.services.library(), body::current().surface_data)
        .choose_at_start(args.services.base.as_deref(), args.services.overlay.as_deref(), args.services.opacity)
        .map_err(|e| anyhow!(e))?;
    if args.record.is_some() && (args.serve_cache || args.screenshot.is_some()) {
        bail!("--record records the window, so it cannot go with --screenshot or --serve-cache");
    }
    if args.serve_cache {
        return serve_cache(&args);
    }
    if let Some(path) = args.screenshot.clone() {
        return screenshot(&args, &path);
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().context("could not connect to the display")?;
    let mut app = App {
        args,
        proxy: event_loop.create_proxy(),
        spacemouse: None,
        gfx: None,
        error: None,
        mods: Mods::default(),
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[&str]) -> Result<Option<Args>> {
        parse_args(s.iter().map(|x| x.to_string()), &Settings::default())
    }

    #[test]
    fn spacemouse_options() {
        let a = parse(&[]).unwrap().unwrap();
        assert!(a.spacemouse && a.spacemouse_sensitivity == 1.0);
        let a = parse(&["--no-spacemouse", "--spacemouse-sensitivity", "2.5"]).unwrap().unwrap();
        assert!(!a.spacemouse && a.spacemouse_sensitivity == 2.5);
        assert!(parse(&["--spacemouse-sensitivity", "0"]).is_err());
        assert!(parse(&["--spacemouse-sensitivity", "-1"]).is_err());
    }

    #[test]
    fn options_parse() {
        let a = parse(&["--lat", "47", "--lon", "11", "--tilt", "60", "--size", "800x600"]).unwrap().unwrap();
        assert!((a.cam.lat.to_degrees() - 47.0).abs() < 1e-9);
        assert_eq!(a.size, (800, 600));
    }

    #[test]
    fn bad_options_are_reported() {
        assert!(parse(&["--nope", "1"]).is_err());
        assert!(parse(&["--lat"]).is_err());
        assert!(parse(&["--lat", "abc"]).is_err());
        assert!(parse(&["--lat", "NaN"]).is_err());
        assert!(parse(&["--size", "0x10"]).is_err());
        assert!(parse(&["--help"]).unwrap().is_none());
    }

    #[test]
    fn distances_read_well() {
        assert_eq!(format_distance(250.4), "250 m");
        assert_eq!(format_distance(2500.0), "2.5 km");
        assert_eq!(format_distance(120_000.0), "120 km");
    }

    #[test]
    fn both_credits_are_present() {
        assert!(CREDITS.contains("EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH"));
        assert!(CREDITS.contains("Copernicus Sentinel data 2024"));
        assert!(CREDITS.contains("Mapterhorn"));
        assert!(CREDITS.contains("OpenMapTiles") && CREDITS.contains("OpenStreetMap"));
    }

    #[test]
    fn building_tiles_follow_the_drawn_tiles() {
        let fine = TileKey::new(14, 300, 400);
        let finer = [TileKey::new(16, 1200, 1600), TileKey::new(15, 601, 801)];
        // Finer tiles share the mesh of the tile they lie in.
        assert_eq!(building_tiles(&[fine, finer[0], finer[1]]), [fine]);
        // One level coarser uses its four children; further out there are none.
        let coarse = TileKey::new(13, 100, 100);
        let mut children = coarse.children();
        children.sort_unstable();
        assert_eq!(building_tiles(&[coarse]), children);
        assert!(building_tiles(&[TileKey::new(12, 50, 50)]).is_empty());
    }

    #[test]
    fn cache_options() {
        let a = parse(&[
            "--cache-limit", "50", "--offline", "--share-cache", "--cache-port", "9000",
            "--cache-server", "nas.local:47813", "--cache-server", "10.0.0.2:47813",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(a.cache_limit_gib, 50);
        assert!(a.offline && a.share_cache && !a.serve_cache);
        assert_eq!(a.cache_port, 9000);
        assert_eq!(a.cache_servers, ["nas.local:47813", "10.0.0.2:47813"]);
        for bad in [
            ["--cache-limit", "0"],
            ["--cache-limit", "x"],
            ["--cache-port", "0"],
            ["--cache-server", "nas"],
            ["--cache-server", ":1"],
            ["--cache-server", "a/b:1"],
            ["--cache-server", "a:99999"],
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
        // Sharing is opt-in: nothing opens a port or looks for others by default.
        let d = parse(&[]).unwrap().unwrap();
        assert!(!d.serve_cache && !d.share_cache && !d.offline && d.cache_servers.is_empty());
    }

    #[test]
    fn credits_follow_what_is_on_screen() {
        assert_eq!(credits_for([""].into_iter()), CREDITS);
        let c = credits_for(["© swisstopo", "", "Grundkarte: basemap.at", "© swisstopo"].into_iter());
        assert!(c.starts_with(CREDITS));
        assert!(c.ends_with("Aerial images: Grundkarte: basemap.at, © swisstopo"), "{c}");
    }

    #[test]
    fn a_server_timeout_sets_the_retry_time_directly() {
        let mut retries = HashMap::new();
        let k = TileKey::new(3, 1, 1);
        let start = Instant::now();
        next_retry(&mut retries, k, None);
        assert!(retries[&k].0 < start + Duration::from_secs(5), "a short first delay");
        next_retry(&mut retries, k, Some(SystemTime::now() + Duration::from_secs(900)));
        assert!(retries[&k].0 > start + Duration::from_secs(890), "the server's timeout");
    }

    #[test]
    fn traffic_shows_on_screen_with_timed_out_servers() {
        let dir = fetcher::tests::temp_cache("hud");
        let f = Fetcher::new(dir, Peers::default(), true);
        let now = SystemTime::now();
        assert!(network_status(&f, now).is_empty());
        f.servers.failed(
            "tiles.example.org",
            servers::Failure::TooManyRequests { retry_after: None },
            now,
        );
        let lines = network_status(&f, now);
        assert_eq!(lines, ["timed out: tiles.example.org 15 min"]);
    }

    #[test]
    fn sizes_and_times_read_well() {
        assert_eq!(format_bytes(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(format_bytes(GIB * 2), "2.0 GB");
        let now = SystemTime::now();
        assert_eq!(minutes_until(now + Duration::from_secs(61), now), 2);
        assert_eq!(minutes_until(now, now + Duration::from_secs(5)), 0);
    }

    #[test]
    fn frame_options() {
        assert_eq!(parse(&[]).unwrap().unwrap().settings.frame_rate, FrameLimit::Fps120);
        assert_eq!(parse(&["--fps", "90"]).unwrap().unwrap().settings.frame_rate, FrameLimit::Fps90);
        assert!(parse(&["--fps", "60"]).is_err());
        assert_eq!(parse(&["--benchmark", "5"]).unwrap().unwrap().benchmark, Some(5.0));
        assert!(parse(&["--benchmark", "0"]).is_err());
    }

    #[test]
    fn record_options() {
        let a = parse(&[]).unwrap().unwrap();
        assert!(a.record.is_none());
        let a = parse(&["--record", "run1", "--record-limit", "0.5"]).unwrap().unwrap();
        assert_eq!(a.record, Some(PathBuf::from("run1")));
        assert_eq!(a.record_limit_gib, 0.5);
        assert!(parse(&["--record-limit", "0"]).is_err());
        assert!(parse(&["--record-limit", "NaN"]).is_err());
    }

    #[test]
    fn time_and_sun_flags() {
        let a = parse(&["--time", "2026-06-21T12:00:00Z", "--time-speed", "60", "--no-sun"]).unwrap().unwrap();
        assert_eq!((a.time, a.time_speed, a.settings.sunlight), (Some(1_782_043_200.0), 60.0, false));
        assert!(parse(&["--time", "noon"]).is_err());
        assert!(parse(&[]).unwrap().unwrap().settings.sunlight);
        assert!(parse(&[]).unwrap().unwrap().settings.cast_shadows);
        assert!(!parse(&["--no-shadows"]).unwrap().unwrap().settings.cast_shadows);
        assert!(parse(&[]).unwrap().unwrap().settings.stars);
        assert!(!parse(&["--no-stars"]).unwrap().unwrap().settings.stars);
        let marked = parse(&["--mark-underlay", "--no-stars"]).unwrap().unwrap();
        assert!(marked.mark_underlay && !marked.settings.stars);
        assert_eq!(parse(&["--tilt", "100"]).unwrap().unwrap().cam.sky.to_degrees().round(), 15.0);
    }

    #[test]
    fn atlas_options() {
        let a = parse(&["--atlas", "mollweide", "--central-meridian", "190"]).unwrap().unwrap();
        assert_eq!((a.settings.view, a.settings.projection), (ViewKind::Map, Projection::Mollweide));
        assert!((a.central_meridian.to_degrees() + 170.0).abs() < 1e-9);
        assert_eq!(parse(&[]).unwrap().unwrap().settings.view, ViewKind::Globe);
        assert!(parse(&["--atlas", "robinson"]).is_err());
        assert!(parse(&["--central-meridian", "x"]).is_err());
    }

    #[test]
    fn the_command_line_wins_over_saved_options_for_a_run() {
        let saved = Settings { trains: false, stars: false, projection: Projection::Mercator, ..Settings::default() };
        let run = |flags: &[&str]| parse_args(flags.iter().map(|x| x.to_string()), &saved).unwrap().unwrap();
        let a = run(&[]);
        assert!(!a.settings.trains && !a.settings.stars && a.settings.projection == Projection::Mercator);
        let a = run(&["--atlas", "mollweide", "--no-trails"]);
        assert_eq!((a.settings.view, a.settings.projection), (ViewKind::Map, Projection::Mollweide));
        assert!(!a.settings.trails && !a.settings.trains, "flag and saved value both apply");
        assert_eq!(run(&["--body", "mars"]).body.id, "mars");
        let moon = Settings { body: "moon".into(), view: ViewKind::Map, ..Settings::default() };
        let a = parse_args(["--globe".to_string()].into_iter(), &moon).unwrap().unwrap();
        assert_eq!((a.body.id, a.settings.view), ("moon", ViewKind::Globe));
    }

    #[test]
    fn pictures_ignore_saved_options_unless_the_folder_is_given() {
        let argv = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(settings_dir(&argv(&["--screenshot", "a.png"])), (None, None));
        let (dir, flag) = settings_dir(&argv(&["--screenshot", "a.png", "--config-dir", "/x"]));
        assert_eq!((dir, flag), (Some(PathBuf::from("/x")), Some(PathBuf::from("/x"))));
        assert_eq!(settings_dir(&argv(&["--config-dir", "/y"])).0, Some(PathBuf::from("/y")));
    }

    #[test]
    fn map_options() {
        let a = parse(&["--lang", "EN", "--no-roads", "--lat", "1"]).unwrap().unwrap();
        assert_eq!(a.lang, "en");
        assert!(!a.settings.streets && !a.settings.boundaries && a.settings.cities);
        assert!(parse(&["--lang", "../x"]).is_err());
        let b = parse(&["--no-labels"]).unwrap().unwrap();
        assert!(b.settings.streets && !b.settings.cities && !b.settings.other_names);
    }

    #[test]
    fn same_place_keeps_its_identity_across_zoom_levels() {
        let label = |lat, text: &str| vector::Label {
            text: text.into(),
            style: vector::LabelStyle::City,
            priority: 0,
            lat,
            lon: 0.2,
            along: None,
        };
        assert_eq!(label_id(&label(0.8001, "A")), label_id(&label(0.80011, "A")));
        assert_ne!(label_id(&label(0.8001, "A")), label_id(&label(0.8001, "B")));
    }

    #[test]
    fn saved_image_waits_for_a_flight() {
        assert!(!screenshot_busy(true, 0, true, 0, false, false));
        assert!(screenshot_busy(true, 0, true, 0, true, false));
        assert!(screenshot_busy(false, 0, true, 0, false, false));
        assert!(screenshot_busy(true, 3, true, 0, false, false));
        assert!(screenshot_busy(true, 0, true, 1, false, false));
        assert!(screenshot_busy(true, 0, true, 0, false, true));
    }

    #[test]
    fn the_display_tells_finished_from_still_loading() {
        assert_eq!(loading_status(0, 0), "all loaded");
        assert_eq!(loading_status(5, 0), "loading 5");
        assert_eq!(loading_status(3, 3), "3 waiting to retry");
        assert_eq!(loading_status(5, 2), "loading 3, 2 waiting to retry");
    }
}
