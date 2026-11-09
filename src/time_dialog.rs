//! The time icon and dialog: three sliders (year, day of the year, time of day), the speed and
//! direction the clock plays at, the buttons for the clock and for the eclipses, and what is less
//! accurate at the date.
//!
//! The dialog shows a [`Model`] shared with the scene, which fills it from the clock; what the
//! user does comes back from [`show`] as an [`Action`] for the scene to carry out on the clock.
//! The date arithmetic is in `time_slider`.

use crate::gui::{self, Gui, Slide};
use crate::sky_time::{self, RATES};
use crate::time_slider::{self, Level, Track};
use std::cell::RefCell;
use std::rc::Rc;

pub const WINDOW: &str = "time-window";

const TRACKS: [(Track, &str); 3] = [(Track::Year, "Year"), (Track::Day, "Day"), (Track::Time, "Time of day")];

/// What the dialog shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// The clock, to the minute: where the thumbs stand.
    pub time: f64,
    /// The full reading of the clock with its seconds and its state.
    pub reading: String,
    pub running: bool,
    /// Clock seconds per real second; below zero the clock plays backwards.
    pub rate: f64,
    /// Eclipses are worked out for the Earth only.
    pub earth: bool,
    /// The eclipse going on at the clock's time, if any.
    pub eclipse: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        Self { time: 0.0, reading: String::new(), running: true, rate: 1.0, earth: true, eclipse: None }
    }
}

pub type Shared = Rc<RefCell<Model>>;

/// What the user asked of the clock.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A drag begins: the clock stands still. On the bar beside the thumb the value jumps there.
    Begin(Track, Option<f64>),
    Set(Track, f64),
    /// The drag is over: the clock goes on as it was.
    End,
    Step(Track, i32),
    Now,
    /// Plays forwards, or backwards, at the chosen speed.
    Play { backwards: bool },
    Pause,
    /// One of `RATES`; the direction stays.
    Speed(f64),
    PreviousEclipse,
    NextEclipse,
}

/// The dialog, while it is open; what the user did comes back as actions.
pub fn show(ctx: &egui::Context, gui: &mut Gui, model: &Model) -> Vec<Action> {
    let mut open = gui.is_visible(WINDOW);
    let mut actions = Vec::new();
    if !open {
        return actions;
    }
    let screen = ctx.content_rect();
    let shown = gui::window("Time", WINDOW)
        .open(&mut open)
        // Above its icon at the bottom right.
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(egui::pos2(screen.right() - 10.0, screen.bottom() - 2.0 * crate::compass::MARGIN - crate::compass::ICON - 6.0))
        .default_width(340.0f32.min(screen.width() - 40.0).max(220.0))
        .show(ctx, |ui| {
            const GAP: f32 = 8.0;
            ui.label(egui::RichText::new(&model.reading).strong());
            ui.add_space(4.0);
            gui::group(ui, "Date and time", |ui, width| {
                for (i, (track, name)) in TRACKS.into_iter().enumerate() {
                    if i > 0 {
                        ui.add_space(2.0);
                    }
                    ui.horizontal(|ui| {
                        ui.label(name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(time_slider::value_label(track, model.time)).color(gui::accent()));
                        });
                    });
                    for slide in gui::track(ui, time_slider::position(track, model.time), width) {
                        actions.push(match slide {
                            Slide::Grab => Action::Begin(track, None),
                            Slide::Jump(p) => Action::Begin(track, Some(p)),
                            Slide::Move(p) => Action::Set(track, p),
                            Slide::End => Action::End,
                            Slide::Step(n) => Action::Step(track, n),
                        });
                    }
                    let (first, last) = time_slider::end_labels(track);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(first).weak().small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(egui::RichText::new(last).weak().small()));
                    });
                }
                ui.add_space(4.0);
                if ui.add_sized([width, 26.0], egui::Button::new("Now")).on_hover_text("Back to the real time, running at real speed").clicked() {
                    actions.push(Action::Now);
                }
            });
            ui.add_space(GAP);
            gui::group(ui, "Playback", |ui, width| {
                let gap = ui.spacing().item_spacing.x;
                let third = (width - 2.0 * gap) / 3.0;
                let backwards = model.running && model.rate < 0.0;
                let forwards = model.running && model.rate >= 0.0;
                ui.horizontal(|ui| {
                    let button = |ui: &mut egui::Ui, text: &str, on: bool| ui.add_sized([third, 26.0], egui::Button::new(text).selected(on)).clicked();
                    if button(ui, "\u{25c0} Backwards", backwards) {
                        actions.push(Action::Play { backwards: true });
                    }
                    if button(ui, "Pause", !model.running) {
                        actions.push(Action::Pause);
                    }
                    if button(ui, "Forwards \u{25b6}", forwards) {
                        actions.push(Action::Play { backwards: false });
                    }
                });
                ui.add_space(4.0);
                let speed_at = RATES.iter().position(|r| *r >= model.rate.abs() - 1e-9).unwrap_or(0);
                let last = RATES.len() - 1;
                ui.horizontal(|ui| {
                    ui.label("Speed");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(sky_time::rate_name(RATES[speed_at])).color(gui::accent()));
                    });
                });
                for slide in gui::track(ui, speed_at as f64 / last as f64, width) {
                    let to = match slide {
                        Slide::Jump(p) | Slide::Move(p) => (p * last as f64).round() as usize,
                        Slide::Step(n) => speed_at.saturating_add_signed(n as isize).min(last),
                        Slide::Grab | Slide::End => continue,
                    };
                    if to != speed_at {
                        actions.push(Action::Speed(RATES[to]));
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(sky_time::rate_name(RATES[0])).weak().small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(sky_time::rate_name(RATES[last])).weak().small())
                    });
                });
            });
            if model.earth {
                ui.add_space(GAP);
                gui::group(ui, "Eclipses", |ui, width| {
                    let half = (width - ui.spacing().item_spacing.x) / 2.0;
                    ui.horizontal(|ui| {
                        if ui.add_sized([half, 26.0], egui::Button::new("\u{25c0} Previous")).clicked() {
                            actions.push(Action::PreviousEclipse);
                        }
                        if ui.add_sized([half, 26.0], egui::Button::new("Next \u{25b6}")).clicked() {
                            actions.push(Action::NextEclipse);
                        }
                    });
                    match &model.eclipse {
                        Some(e) => {
                            ui.label(egui::RichText::new(e).color(gui::accent()));
                        }
                        None => gui::note(ui, "No eclipse at this moment"),
                    }
                });
            }
            ui.add_space(4.0);
            for (level, note) in time_slider::notices(model.time) {
                if level == Level::Warning {
                    ui.label(egui::RichText::new(note).color(gui::warn_colour(ui)));
                } else {
                    gui::note(ui, note);
                }
            }
        });
    if let Some(r) = shown {
        gui.cover(r.response.rect);
    }
    gui.set_visible(WINDOW, open);
    actions
}

