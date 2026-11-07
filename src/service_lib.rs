//! The user's library of map services, the public directory shipped with the program, and
//! which services are in use. The tile sources made for services live as long as the program.

use crate::places::{read_limited, write_atomically};
use crate::services::Service;
use crate::sources::{Kind, TileSource};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::RwLock;
use std::sync::{Mutex, OnceLock};

const VERSION: u64 = 1;
pub const MAX_SERVICES: usize = 200;
/// A service gets few simultaneous requests, as its capacity is unknown.
const CONCURRENCY: usize = 2;
/// Starts the credit of a user-added service, so the credit panel can list it apart.
pub const CREDIT_MARK: char = '\u{2}';
/// Between the parts of a credit made of several.
pub const CREDIT_SEP: char = '\u{1f}';
/// What a service's tile source ids start with.
pub const ID_PREFIX: &str = "svc-";

pub fn default_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("orbis").join("services.json"))
}

/// The services the user has added, kept in a JSON file that is written atomically.
pub struct Library {
    path: Option<PathBuf>,
    services: Vec<Service>,
    locked: bool,
    pub notice: Option<String>,
}

impl Library {
    pub fn in_memory() -> Self {
        Self { path: None, services: Vec::new(), locked: false, notice: None }
    }

    /// Reads the file. A missing file is an empty library; one that cannot be understood is left
    /// where it is, never overwritten, and the library is not saved.
    pub fn load(path: PathBuf) -> Self {
        let mut lib = Self { path: Some(path.clone()), services: Vec::new(), locked: false, notice: None };
        match read_limited(&path) {
            Ok(None) => {}
            Ok(Some(bytes)) => match parse(&bytes) {
                Ok(list) => lib.services = list,
                Err(e) => lib.lock(&e),
            },
            Err(e) => lib.lock(&e),
        }
        lib
    }

    fn lock(&mut self, why: &str) {
        self.locked = true;
        self.notice = Some(format!("The map service library could not be read ({why}). It will not be changed."));
    }

    pub fn services(&self) -> &[Service] {
        &self.services
    }

    pub fn add(&mut self, service: Service) -> Result<(), String> {
        if self.services.iter().any(|s| s.id == service.id) {
            return Err(format!("{} is already in the library.", service.name));
        }
        if self.services.len() >= MAX_SERVICES {
            return Err(format!("At most {MAX_SERVICES} services can be kept."));
        }
        self.services.push(service);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        self.services.retain(|s| s.id != id);
        self.save()
    }

    /// The service with this name: exactly, ignoring case, or else the only one containing it.
    pub fn find<'a>(list: &'a [Service], name: &str) -> Result<&'a Service, String> {
        let want = name.trim().to_lowercase();
        if let Some(s) = list.iter().find(|s| s.name.to_lowercase() == want || s.id == want) {
            return Ok(s);
        }
        let partial: Vec<_> = list.iter().filter(|s| s.name.to_lowercase().contains(&want)).collect();
        match partial.as_slice() {
            [one] => Ok(one),
            [] => Err(format!("No map service is called {name}. --list-services shows the choices.")),
            many => Err(format!("{name} fits {} services: {}", many.len(), many.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if self.locked {
            return Err("The library is not written because the file could not be read.".into());
        }
        let doc = json!({ "version": VERSION, "services": self.services.iter().map(Service::to_json).collect::<Vec<_>>() });
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        write_atomically(path, text.as_bytes()).map_err(|e| format!("The library could not be written: {e}"))
    }
}

fn parse(bytes: &[u8]) -> Result<Vec<Service>, String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|_| "not valid JSON".to_string())?;
    match doc.get("version").and_then(Value::as_u64) {
        Some(VERSION) => {}
        Some(_) => return Err("written by a newer version".into()),
        None => return Err("no version".into()),
    }
    let list = doc.get("services").and_then(Value::as_array).ok_or("no list of services")?;
    if list.len() > MAX_SERVICES {
        return Err("too many services".into());
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for v in list {
        let s = Service::from_json(v).map_err(|e| format!("an entry is damaged: {e}"))?;
        if seen.insert(s.id.clone()) {
            out.push(s);
        }
    }
    Ok(out)
}

/// The public services the program knows about, from `data/services.json`.
pub fn directory() -> &'static [Service] {
    static DIRECTORY: OnceLock<Vec<Service>> = OnceLock::new();
    DIRECTORY.get_or_init(|| match parse(include_str!("../data/services.json").as_bytes()) {
        Ok(list) => list,
        Err(e) => {
            eprintln!("the built-in directory of map services is damaged: {e}");
            Vec::new()
        }
    })
}

// ---- Adding a service ----

/// What the user may say about a service they add, besides its address.
#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    pub name: Option<String>,
    pub credit: Option<String>,
    /// For a WMS or WMTS service: the layer to use; else the first one that can be used.
    pub layer: Option<String>,
    /// For an XYZ address: rows count from the south.
    pub tms: bool,
    pub max_zoom: Option<u8>,
}

#[derive(Debug)]
pub struct Prepared {
    pub service: Service,
    /// All the layers the service offers, as (identifier, title); empty for XYZ.
    pub layers: Vec<(String, String)>,
}

/// Works out a service from what was typed: an XYZ address template, or the address of a WMTS
/// or WMS service, optionally followed by a layer name. `fetch` gets the capabilities
/// document, and says in words why it could not.
pub fn prepare(typed: &str, opts: &AddOptions, fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>) -> Result<Prepared, String> {
    let mut parts = typed.split_whitespace();
    let url = parts.next().ok_or("Type the address of a map service.")?;
    let mut opts = opts.clone();
    if opts.layer.is_none() {
        opts.layer = parts.next().map(str::to_string);
    }
    let (mut service, layers) = if crate::services::looks_like_template(url) {
        (crate::services::xyz_service(url, opts.tms)?, Vec::new())
    } else {
        let request = crate::services::capabilities_request(url);
        crate::services::check_url(&request)?;
        let bytes = fetch(&request)?;
        let text = String::from_utf8_lossy(&bytes);
        let doc = crate::capabilities::parse(&text)?;
        let service = crate::services::from_capabilities(&doc, url, opts.layer.as_deref())?;
        (service, crate::services::layer_list(&doc))
    };
    if let Some(name) = opts.name.as_deref().map(crate::services::clean).filter(|n| !n.is_empty()) {
        service.name = name;
    }
    if let Some(credit) = &opts.credit {
        service.credit = crate::services::clean(credit);
    }
    if let Some(z) = opts.max_zoom {
        service.max_z = z.min(crate::sources::ABSOLUTE_MAX_Z).max(service.min_z);
    }
    service.assign_id();
    Ok(Prepared { service, layers })
}

// ---- Tile sources for services ----

struct Made {
    source: &'static TileSource,
    tile_size: u32,
}

fn made() -> &'static Mutex<HashMap<String, Made>> {
    static MADE: OnceLock<Mutex<HashMap<String, Made>>> = OnceLock::new();
    MADE.get_or_init(Mutex::default)
}

/// A string that lives for the rest of the program; the same text is kept once.
pub fn intern(text: String) -> &'static str {
    static ALL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut all = ALL.get_or_init(Mutex::default).lock().unwrap();
    if let Some(known) = all.get(text.as_str()) {
        return known;
    }
    let leaked: &'static str = Box::leak(text.into_boxed_str());
    all.insert(leaked);
    leaked
}

/// The tile source for a service, made once. One mapping per service, so memory use is
/// bounded by how many services are tried.
pub fn source_for(s: &Service) -> &'static TileSource {
    let id = format!("{ID_PREFIX}{}", s.id);
    let mut made = made().lock().unwrap();
    if let Some(m) = made.get(&id) {
        return m.source;
    }
    let bounds: &'static [[f64; 4]] = match s.bounds {
        Some(b) => Box::leak(Box::new([b])),
        None => &[],
    };
    let source: &'static TileSource = Box::leak(Box::new(TileSource {
        id: intern(id.clone()),
        kind: Kind::Imagery,
        url: intern(s.tiles.clone()),
        cache: intern(format!("{id}/{{z}}/{{x}}/{{y}}.tile")),
        content_type: intern(if s.format.is_empty() { "image/png".into() } else { s.format.clone() }),
        min_z: s.min_z,
        max_z: s.max_z,
        bounds,
        credit: intern(format!("{CREDIT_MARK}{}", s.shown_credit())),
        concurrency: CONCURRENCY,
        pixels: 256,
        grid: crate::geo::Grid::Mercator,
    }));
    made.insert(id, Made { source, tile_size: s.tile_size });
    source
}

pub fn is_service(source: &TileSource) -> bool {
    source.id.starts_with(ID_PREFIX)
}

fn expected_size(source: &TileSource) -> Option<u32> {
    made().lock().unwrap().get(source.id).map(|m| m.tile_size)
}

/// Whether a decoded picture is of the size the service declared.
pub fn check_size(source: &TileSource, width: u32, height: u32) -> Result<(), String> {
    match expected_size(source) {
        Some(n) if width != n || height != n => Err(format!("sent {width} by {height} pixel pictures, not {n} by {n}")),
        _ => Ok(()),
    }
}

// ---- Which services are shown ----

#[derive(Clone, Copy)]
pub struct Active {
    pub base: Option<&'static TileSource>,
    pub overlay: Option<&'static TileSource>,
    pub opacity: f32,
    /// Changes whenever anything above does, so that tiles made earlier can be told apart.
    pub generation: u64,
}

pub const DEFAULT_OPACITY: f32 = 0.6;
const NONE: Active = Active { base: None, overlay: None, opacity: DEFAULT_OPACITY, generation: 0 };

// Tests run on threads of their own and must not see each other's choices.
#[cfg(not(test))]
static ACTIVE: RwLock<Active> = RwLock::new(NONE);
#[cfg(test)]
thread_local! {
    static ACTIVE_HERE: std::cell::Cell<Active> = const { std::cell::Cell::new(NONE) };
}

pub fn active() -> Active {
    #[cfg(not(test))]
    return *ACTIVE.read().unwrap();
    #[cfg(test)]
    ACTIVE_HERE.with(|a| a.get())
}

fn change(f: impl FnOnce(&mut Active)) {
    #[cfg(not(test))]
    {
        let mut a = ACTIVE.write().unwrap();
        f(&mut a);
        a.generation += 1;
    }
    #[cfg(test)]
    ACTIVE_HERE.with(|cell| {
        let mut a = cell.get();
        f(&mut a);
        a.generation += 1;
        cell.set(a);
    });
}

/// Replaces the Earth imagery with a service, or restores the default.
pub fn select_base(s: Option<&Service>) {
    let src = s.map(source_for);
    change(|a| a.base = src);
}

/// Lays a service over the imagery, or removes it.
pub fn select_overlay(s: Option<&Service>) {
    let src = s.map(source_for);
    change(|a| a.overlay = src);
}

pub fn set_opacity(v: f32) {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { DEFAULT_OPACITY };
    change(|a| a.opacity = v);
}

// ---- What went wrong with a service ----

#[derive(Default)]
struct Health {
    problem: Option<String>,
    failed: u64,
    good: u64,
}

fn health() -> &'static Mutex<HashMap<&'static str, Health>> {
    static H: OnceLock<Mutex<HashMap<&'static str, Health>>> = OnceLock::new();
    H.get_or_init(Mutex::default)
}

pub fn note_problem(source: &TileSource, why: String) {
    let mut h = health().lock().unwrap();
    let e = h.entry(source.id).or_default();
    e.failed += 1;
    e.problem = Some(why);
}

pub fn note_good(source: &TileSource) {
    health().lock().unwrap().entry(source.id).or_default().good += 1;
}

/// A line for the panel about trouble with a source, if there has been any.
pub fn problem_text(source: &TileSource) -> Option<String> {
    let h = health().lock().unwrap();
    let e = h.get(source.id)?;
    let why = e.problem.as_ref()?;
    let kept = if e.good > 0 { "coarser tiles are kept" } else { "nothing has been shown from it" };
    Some(format!("{} failed: the service {why} ({kept})", e.failed))
}

/// Forgets earlier trouble, as when a service is chosen again.
pub fn clear_problems(source: &TileSource) {
    health().lock().unwrap().remove(source.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::xyz_service;

    fn svc(n: &str) -> Service {
        let mut s = xyz_service(&format!("https://t{n}.example/{{z}}/{{x}}/{{y}}.png"), false).unwrap();
        s.name = format!("Service {n}");
        s.assign_id();
        s
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("orbis-svc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("services.json")
    }

    fn serve(doc: &'static str) -> impl Fn(&str) -> Result<Vec<u8>, String> {
        move |url: &str| {
            assert!(url.contains("GetCapabilities") || url.ends_with(".xml"), "{url}");
            Ok(doc.as_bytes().to_vec())
        }
    }

    #[test]
    fn preparing_detects_the_kind_and_applies_the_users_words() {
        use crate::capabilities::fixtures;
        let no_net = |_: &str| -> Result<Vec<u8>, String> { panic!("an XYZ address needs no request") };
        let p = prepare("https://t.example/{z}/{x}/{y}.png", &AddOptions { name: Some("My tiles".into()), credit: Some("© Me".into()), max_zoom: Some(30), ..Default::default() }, &no_net).unwrap();
        assert_eq!((p.service.kind, p.service.name.as_str(), p.service.credit.as_str()), (crate::services::Kind::Xyz, "My tiles", "© Me"));
        assert_eq!(p.service.max_z, crate::sources::ABSOLUTE_MAX_Z);
        assert!(p.service.id.starts_with("my-tiles-"));
        let p = prepare("https://w.example/wms", &AddOptions::default(), &serve(fixtures::WMS130)).unwrap();
        assert_eq!(p.service.kind, crate::services::Kind::Wms);
        assert_eq!(p.layers.len(), 2);
        let p = prepare("https://w.example/wms dem", &AddOptions::default(), &serve(fixtures::WMS130)).unwrap();
        assert_eq!(p.service.layer, "dem", "a layer name may follow the address");
        let p = prepare("https://t.example/wmts", &AddOptions::default(), &serve(fixtures::WMTS)).unwrap();
        assert_eq!(p.service.kind, crate::services::Kind::Wmts);
    }

    #[test]
    fn preparing_reports_bad_input_and_bad_answers_in_words() {
        let page = |_: &str| Ok(b"<html><body>Please log in</body></html>".to_vec());
        assert!(prepare("https://h.example/wms", &AddOptions::default(), &page).unwrap_err().contains("web page"));
        let down = |_: &str| Err("The server did not answer.".to_string());
        assert_eq!(prepare("https://h.example/wms", &AddOptions::default(), &down).unwrap_err(), "The server did not answer.");
        assert!(prepare("", &AddOptions::default(), &down).is_err());
        let never = |_: &str| -> Result<Vec<u8>, String> { panic!("no request may be made") };
        assert!(prepare("file:///etc/passwd", &AddOptions::default(), &never).is_err());
        assert!(prepare("https://u:p@h.example/wms", &AddOptions::default(), &never).is_err());
        assert!(prepare("ftp://h.example/{z}/{x}/{y}", &AddOptions::default(), &never).is_err());
    }

    #[test]
    fn the_library_survives_a_restart() {
        let path = temp("restart");
        let mut lib = Library::load(path.clone());
        assert!(lib.services().is_empty() && lib.notice.is_none());
        let mut s = svc("1");
        s.credit = "Me".into();
        lib.add(s.clone()).unwrap();
        lib.add(svc("2")).unwrap();
        assert!(lib.add(s.clone()).unwrap_err().contains("already"));
        let again = Library::load(path.clone());
        assert_eq!(again.services().len(), 2);
        assert_eq!(again.services()[0], s);
        let mut lib = again;
        lib.remove(&s.id).unwrap();
        assert_eq!(Library::load(path.clone()).services().len(), 1);
        assert!(!path.with_extension("json.tmp").exists(), "no temporary file is left");
    }

    #[test]
    fn a_damaged_file_is_never_overwritten() {
        let path = temp("damaged");
        std::fs::write(&path, "{ not json").unwrap();
        let mut lib = Library::load(path.clone());
        assert!(lib.notice.as_deref().unwrap().contains("not valid JSON"));
        assert!(lib.add(svc("1")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        std::fs::write(&path, r#"{"version":9,"services":[]}"#).unwrap();
        assert!(Library::load(path.clone()).notice.unwrap().contains("newer"));
        std::fs::write(&path, r#"{"version":1,"services":[{"name":"x"}]}"#).unwrap();
        assert!(Library::load(path).notice.unwrap().contains("damaged"));
    }

    #[test]
    fn names_are_found_exactly_then_loosely() {
        let list = vec![svc("1"), svc("2")];
        assert_eq!(Library::find(&list, "service 1").unwrap().name, "Service 1");
        assert!(Library::find(&list, "service").unwrap_err().contains("fits 2"));
        assert!(Library::find(&list, "nothing").is_err());
        assert_eq!(Library::find(&list, &list[1].id).unwrap().name, "Service 2");
    }

    #[test]
    fn a_service_source_is_made_once_with_a_separate_cache_and_a_credit() {
        let s = svc("9");
        let a = source_for(&s);
        let b = source_for(&s);
        assert!(std::ptr::eq(a, b));
        assert!(is_service(a) && !is_service(&crate::sources::EOX));
        assert!(a.cache_name(crate::geo::TileKey::new(3, 2, 1)).starts_with(&format!("svc-{}/3/2/1", s.id)));
        assert_eq!(a.concurrency, 2);
        assert_eq!(a.credit, "\u{2}t9.example", "the host stands in for a missing credit");
        let mut bounded = svc("10");
        bounded.bounds = Some([5.0, 45.0, 11.0, 48.0]);
        assert_eq!(source_for(&bounded).bounds.len(), 1);
        assert!(!source_for(&bounded).blank_means_missing(), "a blank picture from a service is still a picture");
        assert!(check_size(a, 256, 256).is_ok());
        assert!(check_size(a, 512, 512).unwrap_err().contains("512 by 512"));
    }

    #[test]
    fn selecting_changes_the_generation_and_clamps_opacity() {
        let before = active().generation;
        set_opacity(7.0);
        assert_eq!(active().opacity, 1.0);
        set_opacity(f32::NAN);
        assert_eq!(active().opacity, DEFAULT_OPACITY);
        assert!(active().generation >= before + 2);
    }

    #[test]
    fn trouble_is_reported_with_its_consequence() {
        let s = svc("7");
        let src = source_for(&s);
        assert_eq!(problem_text(src), None);
        note_problem(src, "sent web pages, not pictures".into());
        assert!(problem_text(src).unwrap().contains("nothing has been shown"));
        note_good(src);
        assert!(problem_text(src).unwrap().contains("coarser tiles are kept"));
        clear_problems(src);
        assert_eq!(problem_text(src), None);
    }
}
