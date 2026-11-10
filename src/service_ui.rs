//! The map service panel: the library and directory as one list, which services are the base
//! and the overlay, and adding a service on a thread of its own. Independent of the window.

use crate::fetcher::{FetchError, Fetcher};
use crate::places::{Key, Pick, Picker};
use crate::service_lib::{self, AddOptions, Library};
use crate::services::Service;
use crate::worker::Worker;
use crate::wtime::Duration;
use std::sync::Arc;

/// Keys the panel understands besides those of any list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    List(Key),
    ToggleOverlay,
    Louder,
    Quieter,
    /// The caller opens the box to type an address.
    Add,
}

pub enum Done {
    Nothing,
    OpenAddBox,
}

/// What the command line says about map services.
#[derive(Default)]
pub struct Cli {
    pub file: Option<std::path::PathBuf>,
    pub add: Option<String>,
    pub name: Option<String>,
    pub credit: Option<String>,
    pub layer: Option<String>,
    pub tms: bool,
    pub max_zoom: Option<u8>,
    pub base: Option<String>,
    pub overlay: Option<String>,
    pub opacity: Option<f32>,
    pub panel: bool,
    pub list: bool,
}

impl Cli {
    pub fn library(&self) -> Library {
        match self.file.clone().or_else(service_lib::default_path) {
            Some(path) => Library::load(path),
            None => Library::in_memory(),
        }
    }

    /// Adds the service named by `--add-service` to the library and says what happened.
    pub fn run_add(&self, fetcher: &Fetcher) -> Result<String, String> {
        let typed = self.add.as_deref().unwrap_or("");
        let opts = AddOptions { name: self.name.clone(), credit: self.credit.clone(), layer: self.layer.clone(), tms: self.tms, max_zoom: self.max_zoom };
        let p = service_lib::prepare(typed, &opts, &|url| fetch_text(fetcher, url))?;
        let mut library = self.library();
        library.add(p.service.clone())?;
        let mut out = format!(
            "Added {} ({}), credit shown: {}",
            p.service.name,
            p.service.kind.name().to_uppercase(),
            p.service.shown_credit()
        );
        if p.layers.len() > 1 {
            out.push_str(&format!("\nIt offers {} layers; this one is {}. Add another with --service-layer NAME:", p.layers.len(), p.service.layer));
            for (id, title) in p.layers.iter().take(40) {
                out.push_str(&format!("\n  {id}  {title}"));
            }
        }
        Ok(out)
    }

    /// The library and the directory, for `--list-services`.
    pub fn list_text(&self) -> String {
        let library = self.library();
        let mut out = String::from("Your library:\n");
        for s in library.services() {
            out.push_str(&format!("  {} [{}]  credit: {}\n", s.name, s.kind.name(), s.shown_credit()));
        }
        if library.services().is_empty() {
            out.push_str("  (none; add one with --add-service ADDRESS)\n");
        }
        out.push_str("Directory:\n");
        for s in service_lib::directory() {
            out.push_str(&format!("  {} [{}]  credit: {}\n", s.name, s.kind.name(), s.shown_credit()));
        }
        out
    }
}

struct Row {
    service: Service,
    saved: bool,
}

pub struct Control {
    library: Library,
    pub picker: Picker,
    base: Option<String>,
    overlay: Option<String>,
    opacity: f32,
    pub notice: Option<String>,
    /// Reads service descriptions; made on the first addition.
    adder: Option<Worker<String, Result<service_lib::Prepared, String>>>,
    adding: bool,
    earth: bool,
    /// The base and overlay chosen on Earth, put back on returning there.
    parked: Option<(Option<String>, Option<String>)>,
}

/// Looks a name up among the library and the directory.
fn find(library: &Library, name: &str) -> Result<Service, String> {
    let all: Vec<Service> = library.services().iter().chain(service_lib::directory()).cloned().collect();
    Library::find(&all, name).cloned()
}

impl Control {
    /// `earth` is false on other bodies, where services cannot be shown.
    pub fn new(library: Library, earth: bool) -> Self {
        Self {
            notice: library.notice.clone(),
            library,
            picker: Picker::default(),
            base: None,
            overlay: None,
            opacity: service_lib::DEFAULT_OPACITY,
            adder: None,
            adding: false,
            earth,
            parked: None,
        }
    }

    /// Follows a change of body: services are put away while another body is in view, and
    /// what was chosen comes back with Earth.
    pub fn set_earth(&mut self, earth: bool) {
        if earth == self.earth {
            return;
        }
        self.earth = earth;
        if earth {
            let (base, overlay) = self.parked.take().unwrap_or_default();
            if let Some(s) = base.and_then(|id| find(&self.library, &id).ok()) {
                self.set_base(Some(&s));
            }
            if let Some(s) = overlay.and_then(|id| find(&self.library, &id).ok()) {
                self.set_overlay(Some(&s));
            }
        } else {
            self.parked = Some((self.base.clone(), self.overlay.clone()));
            if self.base.is_some() {
                self.set_base(None);
            }
            if self.overlay.is_some() {
                self.set_overlay(None);
            }
            self.picker.close();
        }
    }

    /// Applies what the command line chose. Names must exist, or the program says so and stops.
    pub fn choose_at_start(&mut self, base: Option<&str>, overlay: Option<&str>, opacity: Option<f32>) -> Result<(), String> {
        if (base.is_some() || overlay.is_some()) && !self.earth {
            return Err("Map services are shown on Earth only.".into());
        }
        if let Some(o) = opacity {
            self.opacity = o.clamp(0.0, 1.0);
            service_lib::set_opacity(self.opacity);
        }
        if let Some(n) = base {
            let s = find(&self.library, n)?;
            self.set_base(Some(&s));
        }
        if let Some(n) = overlay {
            let s = find(&self.library, n)?;
            self.set_overlay(Some(&s));
        }
        Ok(())
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self.library.services().iter().map(|s| Row { service: s.clone(), saved: true }).collect();
        for d in service_lib::directory() {
            if !rows.iter().any(|r| r.service.id == d.id) {
                rows.push(Row { service: d.clone(), saved: false });
            }
        }
        rows
    }

    /// Whether the library or the directory has a service of this name or identifier.
    pub fn has(&self, name: &str) -> bool {
        find(&self.library, name).is_ok()
    }

    /// The identifier of the service in use instead of the usual imagery, if any.
    pub fn base_id(&self) -> Option<&str> {
        self.base.as_deref()
    }

    /// The services of the library, as (identifier, name), for choosing a base.
    pub fn library_choices(&self) -> Vec<(String, String)> {
        self.library.services().iter().map(|s| (s.id.clone(), s.name.clone())).collect()
    }

    /// Uses the library service with this identifier as the base, or the usual imagery for
    /// `None`. Earth only.
    pub fn choose_base(&mut self, id: Option<&str>) {
        let service = id.and_then(|id| find(&self.library, id).ok());
        if id.is_some() && (service.is_none() || !self.earth) {
            return;
        }
        self.set_base(service.as_ref());
    }

    fn set_base(&mut self, s: Option<&Service>) {
        self.base = s.map(|s| s.id.clone());
        if let Some(s) = s {
            service_lib::clear_problems(service_lib::source_for(s));
        }
        service_lib::select_base(s);
    }

    fn set_overlay(&mut self, s: Option<&Service>) {
        self.overlay = s.map(|s| s.id.clone());
        if let Some(s) = s {
            service_lib::clear_problems(service_lib::source_for(s));
        }
        service_lib::select_overlay(s);
    }

    pub fn is_adding(&self) -> bool {
        self.adding
    }

    pub fn is_open(&self) -> bool {
        self.picker.is_open()
    }

    pub fn toggle(&mut self) {
        self.notice = self.library.notice.clone().filter(|_| !self.picker.is_open());
        if !self.earth {
            self.notice = Some("Map services are shown on Earth only.".into());
        }
        self.picker.toggle();
    }

    pub fn key(&mut self, cmd: Cmd) -> Done {
        self.notice = None;
        let rows = self.rows();
        match cmd {
            Cmd::Add => return Done::OpenAddBox,
            Cmd::List(key) => match self.picker.key(key, rows.len()) {
                Pick::Fly(i) => self.use_as_base(&rows[i]),
                Pick::Delete(i) => self.remove(&rows[i]),
                Pick::None => {}
            },
            Cmd::ToggleOverlay => {
                // The chosen row, which the picker only reveals through its key handling.
                if let Some(row) = self.chosen(&rows) {
                    if self.overlay.as_deref() == Some(&row.service.id) {
                        self.set_overlay(None);
                    } else {
                        self.save_if_new(row);
                        self.set_overlay(Some(&row.service));
                    }
                }
            }
            Cmd::Louder | Cmd::Quieter => {
                let step = if cmd == Cmd::Louder { 0.1 } else { -0.1 };
                self.opacity = ((self.opacity + step) * 10.0).round() / 10.0;
                self.opacity = self.opacity.clamp(0.0, 1.0);
                service_lib::set_opacity(self.opacity);
            }
        }
        Done::Nothing
    }

    fn chosen<'a>(&self, rows: &'a [Row]) -> Option<&'a Row> {
        rows.get(self.picker.selected_index())
    }

    fn save_if_new(&mut self, row: &Row) {
        if !row.saved {
            if let Err(e) = self.library.add(row.service.clone()) {
                self.notice = Some(e);
            }
        }
    }

    fn use_as_base(&mut self, row: &Row) {
        if self.base.as_deref() == Some(&row.service.id) {
            self.set_base(None);
            return;
        }
        self.save_if_new(row);
        self.set_base(Some(&row.service));
    }

    fn remove(&mut self, row: &Row) {
        if !row.saved {
            self.notice = Some(format!("{} is in the built-in directory and cannot be removed.", row.service.name));
            return;
        }
        if self.base.as_deref() == Some(&row.service.id) {
            self.set_base(None);
        }
        if self.overlay.as_deref() == Some(&row.service.id) {
            self.set_overlay(None);
        }
        if let Err(e) = self.library.remove(&row.service.id) {
            self.notice = Some(e);
        }
        let n = self.rows().len();
        self.picker.clamp(n);
    }

    /// Starts adding the service typed in the box. The answer comes through `poll`.
    pub fn start_add(&mut self, fetcher: &Arc<Fetcher>, typed: &str) {
        if self.adding {
            self.notice = Some("A service is still being added.".into());
            return;
        }
        self.adding = true;
        self.notice = Some("Reading the service description...".into());
        let fetcher = fetcher.clone();
        let adder = self.adder.get_or_insert_with(|| {
            Worker::new(false, Duration::ZERO, move |typed: String| {
                service_lib::prepare(&typed, &AddOptions::default(), &|url| fetch_text(&fetcher, url))
            })
        });
        adder.submit(typed.to_string());
    }

    /// Takes in a finished addition. True if the panel changed.
    pub fn poll(&mut self) -> bool {
        let Some(result) = self.adder.as_ref().and_then(Worker::poll) else { return false };
        self.adding = false;
        self.notice = Some(match result {
            Ok(p) => match self.library.add(p.service.clone()) {
                Ok(()) => {
                    self.picker.open_at(self.library.services().len() - 1);
                    format!("Added {}. Enter uses it as the base layer.", p.service.name)
                }
                Err(e) => e,
            },
            Err(e) => format!("Could not add it: {e}"),
        });
        true
    }

    pub fn panel_text(&self) -> String {
        if !self.picker.is_open() {
            return String::new();
        }
        let rows = self.rows();
        let row = |i: usize| {
            let r = &rows[i];
            let role = if self.base.as_deref() == Some(&r.service.id) {
                " (base)".to_string()
            } else if self.overlay.as_deref() == Some(&r.service.id) {
                format!(" (overlay {:.0}%)", self.opacity * 100.0)
            } else {
                String::new()
            };
            format!("{}{} [{}]{role}", if r.saved { "" } else { "+ " }, r.service.name, r.service.kind.name())
        };
        let mut text = self.picker.panel_text(
            "Map services",
            rows.len(),
            row,
            "No services.",
            "Enter: base   O: overlay   +/-: opacity   A: add   Delete: remove\nUp/Down: choose   G or Esc: close   (+ : from the directory)",
        );
        for (what, id) in [("Base", &self.base), ("Overlay", &self.overlay)] {
            let Some(r) = id.as_ref().and_then(|id| rows.iter().find(|r| &r.service.id == id)) else { continue };
            let source = service_lib::source_for(&r.service);
            if let Some(p) = service_lib::problem_text(source) {
                text.push_str(&format!("\n{what} {}: {p}", r.service.name));
            }
        }
        if let Some(n) = &self.notice {
            text.push('\n');
            text.push_str(n);
        }
        text
    }
}

/// The text of one answer from a service, or why there was none.
pub fn fetch_text(fetcher: &Fetcher, url: &str) -> Result<Vec<u8>, String> {
    fetcher.fetch_service(url).map_err(|e| match e {
        FetchError::Missing => "The service says there is nothing at that address.".to_string(),
        FetchError::Paused(_) => "The server is being left alone for a while after errors.".to_string(),
        FetchError::Transient if fetcher.is_offline() => "Offline: the service cannot be asked.".to_string(),
        FetchError::Transient | FetchError::Busy(_) => "The server did not answer or sent an error.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::xyz_service;

    fn control_with(n: usize) -> Control {
        let mut lib = Library::in_memory();
        for i in 0..n {
            let mut s = xyz_service(&format!("https://ui{i}.example/{{z}}/{{x}}/{{y}}.png"), false).unwrap();
            s.name = format!("Ui {i}");
            s.assign_id();
            lib.add(s).unwrap();
        }
        let mut c = Control::new(lib, true);
        c.picker.toggle();
        c
    }

    #[test]
    fn the_list_shows_the_library_then_the_directory() {
        let c = control_with(2);
        let text = c.panel_text();
        assert!(text.starts_with("Map services\n"));
        assert!(text.contains("> Ui 0 [xyz]") && text.contains("  Ui 1 [xyz]"), "{text}");
        // Every directory entry is listed, after the library's own services.
        let library_end = text.find("  Ui 1 [xyz]").unwrap();
        let directory = service_lib::directory();
        assert!(!directory.is_empty());
        for d in directory {
            let at = text.find(&format!("+ {} [", d.name)).unwrap_or_else(|| panic!("{} is not listed: {text}", d.name));
            assert!(at > library_end, "{} is listed before the library: {text}", d.name);
        }
    }

    #[test]
    fn enter_chooses_the_base_and_again_clears_it() {
        let mut c = control_with(2);
        c.key(Cmd::List(Key::Down));
        c.key(Cmd::List(Key::Enter));
        assert!(c.panel_text().contains("Ui 1 [xyz] (base)"));
        c.key(Cmd::List(Key::Enter));
        assert!(!c.panel_text().contains("(base)"));
    }

    #[test]
    fn the_overlay_and_its_opacity_are_separate_from_the_base() {
        let mut c = control_with(2);
        c.key(Cmd::List(Key::Enter));
        c.key(Cmd::List(Key::Down));
        c.key(Cmd::ToggleOverlay);
        let t = c.panel_text();
        assert!(t.contains("Ui 0 [xyz] (base)") && t.contains("Ui 1 [xyz] (overlay 60%)"), "{t}");
        c.key(Cmd::Louder);
        c.key(Cmd::Louder);
        assert!(c.panel_text().contains("overlay 80%"));
        for _ in 0..20 {
            c.key(Cmd::Quieter);
        }
        assert!(c.panel_text().contains("overlay 0%"));
        c.key(Cmd::ToggleOverlay);
        assert!(!c.panel_text().contains("(overlay"));
    }

    #[test]
    fn removing_a_service_clears_its_roles_and_the_directory_stays() {
        let mut c = control_with(1);
        c.key(Cmd::List(Key::Enter));
        c.key(Cmd::List(Key::Delete));
        let t = c.panel_text();
        assert!(!t.contains("Ui 0") && !t.contains("(base)"), "{t}");
        // A directory entry cannot be deleted, and stays listed.
        let first = &service_lib::directory()[0];
        c.key(Cmd::List(Key::Delete));
        let t = c.panel_text();
        assert!(t.contains("cannot be removed") && t.contains(&format!("> + {} [", first.name)), "{t}");
    }

    #[test]
    fn using_a_directory_entry_keeps_it_in_the_library() {
        let first = &service_lib::directory()[0];
        let mut c = control_with(0);
        assert!(c.library.services().is_empty());
        c.key(Cmd::List(Key::Enter));
        let saved = c.library.services();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, first.id, "the chosen entry is the one saved");
        let t = c.panel_text();
        assert!(t.contains(&format!("{} [", first.name)) && t.contains("(base)") && !t.contains(&format!("+ {} [", first.name)), "{t}");
    }

    #[test]
    fn the_add_key_asks_for_the_box() {
        let mut c = control_with(0);
        assert!(matches!(c.key(Cmd::Add), Done::OpenAddBox));
    }

    #[test]
    fn start_up_choices_must_name_real_services_on_earth() {
        let mut c = control_with(1);
        assert!(c.choose_at_start(Some("nothing like it"), None, None).is_err());
        assert!(c.choose_at_start(Some("ui 0"), Some("ui"), Some(0.25)).is_ok());
        assert!(c.panel_text().contains("(base)"));
        let mut moon = Control::new(Library::in_memory(), false);
        assert!(moon.choose_at_start(Some("x"), None, None).unwrap_err().contains("Earth"));
    }
}
