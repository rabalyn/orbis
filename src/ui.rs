//! The mouse-driven interface, without a window: floating panels of widgets laid out in
//! pixels, with hit testing, hover, press and release, wheel scrolling and scroll bar dragging.
//!
//! To add a dialog, build a [`Panel`] whose `layout` function places its widgets for the
//! screen size, and give it to [`Ui::add`]. Show it with [`Ui::set_visible`]. Clicks come back
//! as [`UiEvent::Clicked`] from [`Ui::release`]. To add a kind of widget, add a [`Kind`]
//! variant, and handle it in `Widget::clickable`, in `ui_draw` and in the dialog's events.
//!
//! The credits at the bottom of the screen are shown while their data is, so no panel may cover
//! them. The scene reports their height with [`Ui::set_credit_band`]; a layout function keeps its
//! panel above [`Metrics::floor`], and [`Panel::in_band`] marks the icons that sit in the band
//! itself, where the credits wrap around them.

use crate::nav::Button;
use glam::DVec2;
use std::rc::Rc;

pub type WidgetId = u32;
pub type PanelId = &'static str;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    pub fn contains(&self, p: DVec2) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Heading,
    Key,
    Dim,
    /// Something is wrong or approximate and should be seen.
    Warn,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Label { text: String, tone: Tone },
    /// `text_mult` enlarges the label's glyphs.
    Button { label: String, text_mult: usize },
    /// A box that is ticked or not, with its label. The whole row clicks.
    Checkbox { label: String, checked: bool, enabled: bool },
    /// One option of a group of which one is chosen. The whole row clicks.
    Choice { label: String, chosen: bool, enabled: bool },
    /// A bar with a thumb, `pos` from 0 to 1 along it. The thumb is `thumb_w` wide, so the
    /// thumb's left edge moves over `rect.w - thumb_w`. Dragged by the pointer; a wheel notch steps it.
    Slider { pos: f64, thumb_w: f64 },
    /// A clock face that clicks like a button, for the icon that opens the time dialog.
    ClockFace,
    /// A compass needle that clicks like a button; `heading` is the view's, in radians clockwise
    /// from north, so north on the screen lies that far anticlockwise from up.
    Compass { heading: f64 },
    /// A chart over one day, drawn into the rect; see [`Chart`].
    Chart(Box<Chart>),
    /// A patch of colour for a legend, with straight alpha, hatched for a fossil source.
    Swatch { colour: [u8; 4], hatched: bool },
}

/// One stacked area of a [`Chart`]: colour, name, values, and whether it is hatched rather than solid.
pub type ChartLayer = ([u8; 3], String, Vec<Option<f32>>, bool);

/// A day of stacked areas with a line over them, as drawn by [`Kind::Chart`]: values per equal
/// step from midnight to midnight, `None` where there is none.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// Stacked from the bottom.
    pub layers: Vec<ChartLayer>,
    /// Drawn over the areas in white: name and values.
    pub line: (String, Vec<Option<f32>>),
    /// Where a vertical marker stands, from 0 (midnight) to 1 (the next midnight).
    pub marker: Option<f64>,
    /// The values' unit for the axis, and how many values one unit is: ("GW", 1000.0) for MW.
    pub unit: (String, f32),
}

impl Chart {
    /// Legend rows for a chart `width` wide at `scale`: the items per row and the row count.
    pub fn legend_rows(&self, width: f64, scale: f64) -> (usize, usize) {
        let per_row = ((width / (CHART_LEGEND_COLS as f64 * 8.0 * scale)) as usize).max(1);
        (per_row, (self.layers.len() + 1).div_ceil(per_row))
    }
}

/// Characters per legend item, swatch included.
pub const CHART_LEGEND_COLS: usize = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub id: WidgetId,
    /// In the panel, or in the scrolled content for a widget of the body.
    pub rect: Rect,
    pub kind: Kind,
}

impl Widget {
    pub fn label(rect: Rect, text: &str, tone: Tone) -> Self {
        Self { id: 0, rect, kind: Kind::Label { text: text.to_string(), tone } }
    }

    pub fn button(id: WidgetId, rect: Rect, label: &str, text_mult: usize) -> Self {
        Self { id, rect, kind: Kind::Button { label: label.to_string(), text_mult } }
    }

    pub fn checkbox(id: WidgetId, rect: Rect, label: &str, checked: bool, enabled: bool) -> Self {
        Self { id, rect, kind: Kind::Checkbox { label: label.to_string(), checked, enabled } }
    }

    pub fn choice(id: WidgetId, rect: Rect, label: &str, chosen: bool, enabled: bool) -> Self {
        Self { id, rect, kind: Kind::Choice { label: label.to_string(), chosen, enabled } }
    }

    pub fn slider(id: WidgetId, rect: Rect, pos: f64, thumb_w: f64) -> Self {
        Self { id, rect, kind: Kind::Slider { pos, thumb_w } }
    }

    pub fn clock_face(id: WidgetId, rect: Rect) -> Self {
        Self { id, rect, kind: Kind::ClockFace }
    }

    pub fn compass(id: WidgetId, rect: Rect, heading: f64) -> Self {
        Self { id, rect, kind: Kind::Compass { heading } }
    }

    pub fn swatch(rect: Rect, colour: [u8; 4], hatched: bool) -> Self {
        Self { id: 0, rect, kind: Kind::Swatch { colour, hatched } }
    }

    pub fn chart(rect: Rect, chart: Chart) -> Self {
        Self { id: 0, rect, kind: Kind::Chart(Box::new(chart)) }
    }

    fn clickable(&self) -> bool {
        match self.kind {
            Kind::Label { .. } | Kind::Chart(_) | Kind::Swatch { .. } => false,
            Kind::Button { .. } | Kind::Slider { .. } | Kind::ClockFace | Kind::Compass { .. } => true,
            Kind::Checkbox { enabled, .. } | Kind::Choice { enabled, .. } => enabled,
        }
    }
}

/// The thumb of a slider in `rect`, for a position from 0 to 1.
pub fn slider_thumb(rect: Rect, thumb_w: f64, pos: f64) -> Rect {
    let travel = (rect.w - thumb_w).max(0.0);
    Rect::new(rect.x + pos.clamp(0.0, 1.0) * travel, rect.y, thumb_w.min(rect.w), rect.h)
}

/// The position a slider takes when the pointer is at `x` (in the slider's coordinates) holding
/// the thumb `grab` from its left edge.
pub fn slider_pos(rect: Rect, thumb_w: f64, x: f64, grab: f64) -> f64 {
    let travel = rect.w - thumb_w;
    if travel <= 0.0 {
        return 0.0;
    }
    ((x - grab - rect.x) / travel).clamp(0.0, 1.0)
}

/// The scrolling part of a panel.
#[derive(Clone, Debug, Default)]
pub struct Body {
    /// In the panel.
    pub viewport: Rect,
    pub content_h: f64,
    pub offset: f64,
    /// Pixels moved by one notch of the wheel.
    pub step: f64,
    /// Width of the scroll bar, inside the right edge of the viewport.
    pub bar_w: f64,
    pub widgets: Vec<Widget>,
}

impl Body {
    pub fn max_offset(&self) -> f64 {
        (self.content_h - self.viewport.h).max(0.0)
    }

    pub fn scrolls(&self) -> bool {
        self.max_offset() > 0.0
    }

    fn clamp(&mut self) {
        self.offset = self.offset.clamp(0.0, self.max_offset());
    }

    /// The bar's track and thumb, in the panel.
    pub fn bar(&self) -> (Rect, Rect) {
        let v = self.viewport;
        let track = Rect::new(v.x + v.w - self.bar_w, v.y, self.bar_w, v.h);
        let thumb_h = (track.h * v.h / self.content_h.max(1.0)).clamp(16.0_f64.min(track.h), track.h);
        let max = self.max_offset();
        let at = if max > 0.0 { (track.h - thumb_h) * self.offset / max } else { 0.0 };
        (track, Rect::new(track.x, track.y + at, track.w, thumb_h))
    }
}

pub struct Metrics {
    pub screen: DVec2,
    /// Whole pixels per glyph pixel, and so per design pixel.
    pub scale: f64,
    /// Height of the strip at the bottom that the credits and the icons use.
    pub credit_band: f64,
}

/// Side of the square icons, and the space around the edge of the screen, in design pixels.
const ICON: f64 = 28.0;
const EDGE: f64 = 8.0;

pub fn icon_size(scale: f64) -> f64 {
    ICON * scale
}

impl Metrics {
    /// Where icon number `slot` sits, counting from the bottom right corner. The credits end
    /// where the row of icons begins.
    pub fn icon_rect(&self, slot: usize) -> Rect {
        let (size, edge) = (icon_size(self.scale), EDGE * self.scale);
        let x = self.screen.x - edge - (slot as f64 + 1.0) * size - slot as f64 * edge / 2.0;
        Rect::new(x, self.screen.y - edge - size, size, size)
    }

    /// Width the row of `count` icons takes, with the space before it.
    pub fn icons_width(&self, count: usize) -> f64 {
        (self.screen.x - self.icon_rect(count.saturating_sub(1)).x) + EDGE * self.scale
    }

    /// The bottom of the area panels may use: above the credits, and above the icons.
    pub fn floor(&self) -> f64 {
        self.screen.y - self.credit_band.max((ICON + 2.0 * EDGE) * self.scale)
    }
}

/// Places a panel and its widgets for the screen.
pub type Layout = Rc<dyn Fn(&mut Panel, &Metrics)>;

pub struct Panel {
    pub id: PanelId,
    /// On the screen.
    pub rect: Rect,
    pub visible: bool,
    /// Draws the translucent background and border.
    pub frame: bool,
    /// An icon in the strip at the bottom, which the credits leave room for.
    pub in_band: bool,
    pub fixed: Vec<Widget>,
    pub body: Option<Body>,
    /// Sets `rect`, the widgets and the body's size for the screen. Scroll position is kept.
    pub layout: Layout,
    /// Counts changes of what the panel looks like.
    pub(crate) version: u64,
    pub(crate) drawn: u64,
}

impl Panel {
    pub fn new(id: PanelId, frame: bool, layout: impl Fn(&mut Panel, &Metrics) + 'static) -> Self {
        Self {
            id,
            rect: Rect::default(),
            visible: false,
            frame,
            in_band: false,
            fixed: Vec::new(),
            body: None,
            layout: Rc::new(layout),
            version: 1,
            drawn: 0,
        }
    }

    /// The point in panel coordinates.
    fn local(&self, p: DVec2) -> DVec2 {
        p - DVec2::new(self.rect.x, self.rect.y)
    }
}

/// How a slider drag begins and ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    /// The pointer went down on the thumb: the value stays where it is.
    Grab,
    /// The pointer went down on the bar beside the thumb: the thumb jumps there and follows.
    Jump,
    Move,
    /// The button was let go.
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiEvent {
    Clicked { panel: PanelId, widget: WidgetId },
    /// A slider was dragged to `pos` (0 to 1).
    Slid { panel: PanelId, widget: WidgetId, pos: f64, drag: Drag },
    /// The wheel over a slider: `steps` notches, positive for up.
    Stepped { panel: PanelId, widget: WidgetId, steps: i32 },
}

/// What the interface made of an input.
#[derive(Debug, Default, PartialEq)]
pub struct Response {
    /// The input is the interface's, and must not reach the map.
    pub consumed: bool,
    /// Something on screen changed.
    pub redraw: bool,
    pub event: Option<UiEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Widget(WidgetId),
    Thumb,
    Track,
    Surface,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    Widget { panel: usize, id: WidgetId },
    /// `grab` is the pointer's distance below the thumb's top.
    Thumb { panel: usize, grab: f64 },
    /// `grab` is the pointer's distance right of the slider thumb's left edge.
    Slider { panel: usize, id: WidgetId, grab: f64 },
    Surface,
}

pub struct Ui {
    panels: Vec<Panel>,
    metrics: Metrics,
    hover: Option<(usize, WidgetId)>,
    /// What each button (left, right) went down on, if the interface took it.
    held: [Option<Held>; 2],
}

fn slot(b: Button) -> usize {
    (b == Button::Right) as usize
}

impl Ui {
    pub fn new() -> Self {
        Self {
            panels: Vec::new(),
            metrics: Metrics { screen: DVec2::new(800.0, 600.0), scale: 1.0, credit_band: 0.0 },
            hover: None,
            held: [None; 2],
        }
    }

    /// Adds a panel on top of the others and returns its index. Panels are drawn in order.
    pub fn add(&mut self, mut panel: Panel) -> usize {
        let layout = panel.layout.clone();
        layout(&mut panel, &self.metrics);
        self.panels.push(panel);
        self.panels.len() - 1
    }

    /// Width the row of icons takes at the bottom right, with the space before it.
    pub fn icons_width(&self) -> f64 {
        self.metrics.icons_width(self.panels.iter().filter(|p| p.in_band).count())
    }

    /// Visible panels that reach into the credit strip. Always empty, if layouts keep the rule.
    #[cfg(test)]
    pub fn covering_credits(&self) -> Vec<PanelId> {
        let floor = self.metrics.floor();
        let eps = 0.01;
        self.panels.iter().filter(|p| p.visible && !p.in_band && p.rect.y + p.rect.h > floor + eps).map(|p| p.id).collect()
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    pub fn resize(&mut self, screen: DVec2, scale: f64) {
        if self.metrics.screen == screen && self.metrics.scale == scale {
            return;
        }
        self.metrics = Metrics { screen, scale, credit_band: self.metrics.credit_band };
        self.relayout();
    }

    /// Tells the interface how tall the credits are. Panels are laid out again if it changed.
    pub fn set_credit_band(&mut self, height: f64) {
        if self.metrics.credit_band == height {
            return;
        }
        self.metrics.credit_band = height;
        self.relayout();
    }

    /// Lays the panel out again, because what it shows changed behind its back.
    pub fn refresh(&mut self, id: PanelId) {
        let metrics = &self.metrics;
        for p in self.panels.iter_mut().filter(|p| p.id == id) {
            let layout = p.layout.clone();
            layout(p, metrics);
            if let Some(b) = &mut p.body {
                b.clamp();
            }
            p.version += 1;
        }
    }

    #[cfg(test)]
    pub fn relayout_for_test(&mut self) {
        self.relayout();
    }

    fn relayout(&mut self) {
        for p in &mut self.panels {
            let layout = p.layout.clone();
            layout(p, &self.metrics);
            if let Some(b) = &mut p.body {
                b.clamp();
            }
            p.version += 1;
        }
    }

    /// Hides each of `others` that overlaps the panel `opened`, for screens too small for both.
    pub fn close_overlapping(&mut self, opened: PanelId, others: &[PanelId]) {
        let Some(open) = self.panels.iter().find(|p| p.id == opened && p.visible).map(|p| p.rect) else { return };
        for id in others.iter().filter(|id| **id != opened) {
            let overlaps = self.panels.iter().any(|p| p.id == *id && p.visible && p.rect.intersects(&open));
            if overlaps {
                self.set_visible(id, false);
            }
        }
    }

    pub fn is_visible(&self, id: PanelId) -> bool {
        self.panels.iter().any(|p| p.id == id && p.visible)
    }

    pub fn set_visible(&mut self, id: PanelId, visible: bool) {
        let Some(i) = self.panels.iter().position(|p| p.id == id) else { return };
        if self.panels[i].visible == visible {
            return;
        }
        self.panels[i].visible = visible;
        self.panels[i].version += 1;
        if !visible {
            self.forget(i);
        }
    }

    pub fn toggle(&mut self, id: PanelId) {
        self.set_visible(id, !self.is_visible(id));
    }

    /// Drops hover and holds that point into a panel that went away.
    fn forget(&mut self, i: usize) {
        if self.hover.is_some_and(|h| h.0 == i) {
            self.hover = None;
        }
        for h in &mut self.held {
            if matches!(*h, Some(Held::Widget { panel, .. } | Held::Thumb { panel, .. } | Held::Slider { panel, .. }) if panel == i) {
                *h = Some(Held::Surface);
            }
        }
    }

    pub(crate) fn hovered(&self, panel: usize) -> Option<WidgetId> {
        self.hover.filter(|h| h.0 == panel).map(|h| h.1)
    }

    pub(crate) fn pressed(&self, panel: usize) -> Option<WidgetId> {
        match self.held[0] {
            Some(Held::Widget { panel: p, id }) if p == panel && self.hover == Some((p, id)) => Some(id),
            Some(Held::Slider { panel: p, id, .. }) if p == panel => Some(id),
            _ => None,
        }
    }

    /// The slider being dragged, if any.
    pub fn slider_held(&self) -> Option<(PanelId, WidgetId)> {
        match self.held[0] {
            Some(Held::Slider { panel, id, .. }) => Some((self.panels[panel].id, id)),
            _ => None,
        }
    }

    /// The slider with this id in the panel's fixed widgets: its rectangle, thumb width and position.
    fn slider_of(&self, panel: usize, id: WidgetId) -> Option<(Rect, f64, f64)> {
        self.panels[panel].fixed.iter().find_map(|w| match w.kind {
            Kind::Slider { pos, thumb_w } if w.id == id => Some((w.rect, thumb_w, pos)),
            _ => None,
        })
    }

    pub(crate) fn thumb_held(&self, panel: usize) -> bool {
        matches!(self.held[0], Some(Held::Thumb { panel: p, .. }) if p == panel)
    }

    /// The top panel under the point, and what is there.
    fn hit(&self, p: DVec2) -> Option<(usize, Target)> {
        let i = self.panels.iter().rposition(|q| q.visible && q.rect.contains(p))?;
        let panel = &self.panels[i];
        let l = panel.local(p);
        if let Some(w) = panel.fixed.iter().rev().find(|w| w.clickable() && w.rect.contains(l)) {
            return Some((i, Target::Widget(w.id)));
        }
        if let Some(body) = &panel.body {
            if body.viewport.contains(l) {
                if body.scrolls() {
                    let (track, thumb) = body.bar();
                    if thumb.contains(l) {
                        return Some((i, Target::Thumb));
                    }
                    if track.contains(l) {
                        return Some((i, Target::Track));
                    }
                }
                let content = DVec2::new(l.x - body.viewport.x, l.y - body.viewport.y + body.offset);
                if let Some(w) = body.widgets.iter().rev().find(|w| w.clickable() && w.rect.contains(content)) {
                    return Some((i, Target::Widget(w.id)));
                }
            }
        }
        Some((i, Target::Surface))
    }

    /// Sets the scroll position so the thumb's top is at `top` (in the panel).
    fn drag_thumb(&mut self, i: usize, top: f64) -> bool {
        let Some(body) = &mut self.panels[i].body else { return false };
        let (track, thumb) = body.bar();
        let room = track.h - thumb.h;
        let before = body.offset;
        if room > 0.0 {
            body.offset = (top - track.y) / room * body.max_offset();
        }
        body.clamp();
        let changed = body.offset != before;
        if changed {
            self.panels[i].version += 1;
        }
        changed
    }

    /// Call on every pointer move. Not `consumed` unless a scroll bar is being dragged.
    pub fn pointer_moved(&mut self, p: DVec2) -> Response {
        if let Some(Held::Thumb { panel, grab }) = self.held[0] {
            let top = self.panels[panel].local(p).y - grab;
            let redraw = self.drag_thumb(panel, top);
            return Response { consumed: true, redraw, event: None };
        }
        if let Some(Held::Slider { panel, id, grab }) = self.held[0] {
            let event = self.slider_of(panel, id).map(|(rect, thumb_w, _)| UiEvent::Slid {
                panel: self.panels[panel].id,
                widget: id,
                pos: slider_pos(rect, thumb_w, self.panels[panel].local(p).x, grab),
                drag: Drag::Move,
            });
            return Response { consumed: true, redraw: event.is_some(), event };
        }
        let now = match self.hit(p) {
            Some((i, Target::Widget(id))) => Some((i, id)),
            _ => None,
        };
        if now == self.hover {
            return Response::default();
        }
        for h in [self.hover, now].into_iter().flatten() {
            self.panels[h.0].version += 1;
        }
        self.hover = now;
        Response { redraw: true, ..Default::default() }
    }

    pub fn press(&mut self, b: Button, p: DVec2) -> Response {
        let Some((i, target)) = self.hit(p) else { return Response::default() };
        let left = b == Button::Left;
        let mut held = Held::Surface;
        let mut event = None;
        if left {
            match target {
                Target::Widget(id) => {
                    held = Held::Widget { panel: i, id };
                    if let Some((rect, thumb_w, pos)) = self.slider_of(i, id) {
                        let x = self.panels[i].local(p).x;
                        let thumb = slider_thumb(rect, thumb_w, pos);
                        let on_thumb = x >= thumb.x && x < thumb.x + thumb.w;
                        let grab = if on_thumb { x - thumb.x } else { thumb_w / 2.0 };
                        held = Held::Slider { panel: i, id, grab };
                        event = Some(UiEvent::Slid {
                            panel: self.panels[i].id,
                            widget: id,
                            pos: if on_thumb { pos } else { slider_pos(rect, thumb_w, x, grab) },
                            drag: if on_thumb { Drag::Grab } else { Drag::Jump },
                        });
                    }
                }
                Target::Thumb | Target::Track => {
                    if target == Target::Track {
                        // A click on the track centres the thumb there, and carries on as a drag.
                        let thumb_h = self.panels[i].body.as_ref().map_or(0.0, |b| b.bar().1.h);
                        let top = self.panels[i].local(p).y - thumb_h / 2.0;
                        self.drag_thumb(i, top);
                    }
                    let thumb_top = self.panels[i].body.as_ref().map_or(0.0, |b| b.bar().1.y);
                    held = Held::Thumb { panel: i, grab: self.panels[i].local(p).y - thumb_top };
                }
                Target::Surface => {}
            }
        }
        self.held[slot(b)] = Some(held);
        self.panels[i].version += 1;
        Response { consumed: true, redraw: true, event }
    }

    pub fn release(&mut self, b: Button, p: DVec2) -> Response {
        let Some(held) = self.held[slot(b)].take() else { return Response::default() };
        let mut event = None;
        if let Held::Widget { panel, id } = held {
            self.panels[panel].version += 1;
            if self.hit(p).is_some_and(|h| h == (panel, Target::Widget(id))) {
                event = Some(UiEvent::Clicked { panel: self.panels[panel].id, widget: id });
            }
        } else if let Held::Thumb { panel, .. } = held {
            self.panels[panel].version += 1;
        } else if let Held::Slider { panel, id, grab } = held {
            self.panels[panel].version += 1;
            if let Some((rect, thumb_w, _)) = self.slider_of(panel, id) {
                let pos = slider_pos(rect, thumb_w, self.panels[panel].local(p).x, grab);
                event = Some(UiEvent::Slid { panel: self.panels[panel].id, widget: id, pos, drag: Drag::End });
            }
        }
        Response { consumed: true, redraw: true, event }
    }

    /// The wheel over a panel scrolls its body, if it has one that needs it. Positive is up.
    pub fn wheel(&mut self, lines: f64, p: DVec2) -> Response {
        let Some((i, target)) = self.hit(p) else { return Response::default() };
        if let Target::Widget(id) = target {
            if self.slider_of(i, id).is_some() && lines != 0.0 {
                // A small scroll still moves one notch.
                let steps = if lines.abs() < 1.0 { lines.signum() as i32 } else { lines.round() as i32 };
                let event = UiEvent::Stepped { panel: self.panels[i].id, widget: id, steps };
                return Response { consumed: true, redraw: false, event: Some(event) };
            }
        }
        let mut redraw = false;
        if let Some(body) = &mut self.panels[i].body {
            let before = body.offset;
            body.offset -= lines * body.step;
            body.clamp();
            redraw = body.offset != before;
        }
        if redraw {
            self.panels[i].version += 1;
        }
        Response { consumed: true, redraw, event: None }
    }

    /// True if the point is over something visible that belongs to the interface.
    #[cfg(test)]
    pub fn covers(&self, p: DVec2) -> bool {
        self.hit(p).is_some()
    }

    /// Panels that must be drawn again, as `(index, visible)`. Marks them as drawn.
    pub fn take_changes(&mut self) -> Vec<(usize, bool)> {
        let mut out = Vec::new();
        for (i, p) in self.panels.iter_mut().enumerate() {
            if p.version != p.drawn {
                p.drawn = p.version;
                out.push((i, p.visible));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BTN: WidgetId = 7;

    fn layout(p: &mut Panel, m: &Metrics) {
        // A 100x60 panel at the bottom right: a button, and a body of 10 rows 20 px each.
        p.rect = Rect::new(m.screen.x - 110.0, m.screen.y - 70.0, 100.0, 60.0);
        p.fixed = vec![Widget::button(BTN, Rect::new(80.0, 0.0, 20.0, 20.0), "x", 1)];
        let old = p.body.as_ref().map_or(0.0, |b| b.offset);
        let widgets = (0..10)
            .map(|i| Widget::button(100 + i, Rect::new(0.0, i as f64 * 20.0, 80.0, 20.0), "row", 1))
            .collect();
        p.body = Some(Body {
            viewport: Rect::new(0.0, 20.0, 100.0, 40.0),
            content_h: 200.0,
            offset: old,
            step: 20.0,
            bar_w: 8.0,
            widgets,
        });
    }

    fn ui() -> Ui {
        let mut ui = Ui::new();
        ui.resize(DVec2::new(400.0, 300.0), 1.0);
        ui.add(Panel::new("dlg", true, layout));
        ui.set_visible("dlg", true);
        ui
    }

    // Panel is at x 290..390, y 230..290.
    const ON_BTN: DVec2 = DVec2::new(380.0, 240.0);
    const ON_ROW: DVec2 = DVec2::new(300.0, 255.0);
    const ON_BAR: DVec2 = DVec2::new(386.0, 252.0);
    const OFF: DVec2 = DVec2::new(10.0, 10.0);

    fn offset(ui: &Ui) -> f64 {
        ui.panels[0].body.as_ref().unwrap().offset
    }

    fn low_layout(p: &mut Panel, m: &Metrics) {
        // A tall panel that asks for the whole bottom, and so has to be cut off at the floor.
        let h = (300.0f64).min(m.floor() - 10.0);
        p.rect = Rect::new(10.0, m.floor() - h, 100.0, h);
    }

    #[test]
    fn panels_stay_above_the_credits_whatever_their_height() {
        let mut ui = Ui::new();
        ui.resize(DVec2::new(500.0, 350.0), 1.0);
        ui.add(Panel::new("low", true, low_layout));
        ui.set_visible("low", true);
        assert!(ui.covering_credits().is_empty());
        for band in [0.0, 36.0, 120.0, 200.0] {
            ui.set_credit_band(band);
            assert!(ui.covering_credits().is_empty(), "band {band}");
            let r = ui.panels()[0].rect;
            assert!(r.y + r.h <= 350.0 - band.max(36.0) + 0.01);
        }
        // A panel that ignores the rule is reported.
        ui.panels[0].rect.h += 50.0;
        assert_eq!(ui.covering_credits(), vec!["low"]);
        ui.panels[0].in_band = true;
        assert!(ui.covering_credits().is_empty(), "icons in the strip are exempt");
    }

    #[test]
    fn the_row_of_icons_leaves_the_credits_the_rest_of_the_width() {
        let mut ui = Ui::new();
        ui.resize(DVec2::new(1000.0, 500.0), 1.0);
        let m = &ui.metrics;
        let (a, b) = (m.icon_rect(0), m.icon_rect(1));
        assert!(b.x + b.w < a.x, "icons do not touch");
        assert!(a.x + a.w <= 992.0 && a.y + a.h <= 492.0);
        assert!(m.icons_width(2) > m.icons_width(1));
        assert!(1000.0 - m.icons_width(2) <= b.x);
    }

    #[test]
    fn opening_a_second_dialog_closes_the_first_only_when_they_overlap() {
        fn left(p: &mut Panel, _: &Metrics) {
            p.rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        }
        fn right(p: &mut Panel, m: &Metrics) {
            p.rect = Rect::new(m.screen.x - 100.0, 0.0, 100.0, 100.0);
        }
        for (w, both) in [(400.0, true), (150.0, false)] {
            let mut ui = Ui::new();
            ui.resize(DVec2::new(w, 300.0), 1.0);
            ui.add(Panel::new("a", true, left));
            ui.add(Panel::new("b", true, right));
            ui.set_visible("a", true);
            ui.set_visible("b", true);
            ui.close_overlapping("b", &["a"]);
            assert_eq!(ui.is_visible("a"), both, "{w}");
            assert!(ui.is_visible("b"));
        }
    }

    #[test]
    fn hidden_panels_take_nothing() {
        let mut ui = ui();
        ui.set_visible("dlg", false);
        assert!(!ui.press(Button::Left, ON_BTN).consumed);
        assert!(!ui.wheel(1.0, ON_ROW).consumed);
        assert!(!ui.covers(ON_ROW));
    }

    #[test]
    fn press_and_wheel_over_a_panel_are_consumed_elsewhere_not() {
        let mut ui = ui();
        assert!(!ui.press(Button::Left, OFF).consumed);
        assert!(!ui.wheel(1.0, OFF).consumed);
        assert!(!ui.release(Button::Left, OFF).consumed);
        // The header strip is the panel's surface; the right button is taken as well.
        let surface = DVec2::new(300.0, 235.0);
        assert!(ui.press(Button::Right, surface).consumed);
        assert!(ui.release(Button::Right, surface).consumed);
    }

    #[test]
    fn click_needs_press_and_release_on_the_same_button() {
        let mut ui = ui();
        ui.press(Button::Left, ON_BTN);
        let r = ui.release(Button::Left, ON_BTN);
        assert_eq!(r.event, Some(UiEvent::Clicked { panel: "dlg", widget: BTN }));
        ui.press(Button::Left, ON_BTN);
        // Slid off before letting go: no click, but still the interface's.
        let r = ui.release(Button::Left, ON_ROW);
        assert!(r.consumed);
        assert_eq!(r.event, None);
        ui.press(Button::Left, ON_ROW);
        let r = ui.release(Button::Left, ON_BTN);
        assert_eq!(r.event, None);
    }

    #[test]
    fn hover_reports_only_changes() {
        let mut ui = ui();
        assert!(!ui.pointer_moved(OFF).redraw);
        assert!(ui.pointer_moved(ON_BTN).redraw);
        assert!(!ui.pointer_moved(ON_BTN + DVec2::new(-1.0, 1.0)).redraw);
        assert_eq!(ui.hovered(0), Some(BTN));
        assert!(ui.pointer_moved(OFF).redraw);
        assert_eq!(ui.hovered(0), None);
        // The pointer is never consumed outside a bar drag.
        assert!(!ui.pointer_moved(ON_BTN).consumed);
    }

    #[test]
    fn wheel_scrolls_within_bounds() {
        let mut ui = ui();
        assert!(!ui.wheel(1.0, ON_ROW).redraw, "already at the top");
        assert!(ui.wheel(-2.0, ON_ROW).redraw);
        assert_eq!(offset(&ui), 40.0);
        ui.wheel(-100.0, ON_ROW);
        assert_eq!(offset(&ui), 160.0);
        let r = ui.wheel(-1.0, ON_ROW);
        assert!(r.consumed && !r.redraw);
    }

    #[test]
    fn scrolled_rows_are_hit_where_they_are_drawn() {
        let mut ui = ui();
        ui.wheel(-2.0, ON_ROW); // offset 40: row 2 is at the top of the viewport
        ui.press(Button::Left, ON_ROW);
        let r = ui.release(Button::Left, ON_ROW);
        assert_eq!(r.event, Some(UiEvent::Clicked { panel: "dlg", widget: 102 }));
    }

    #[test]
    fn dragging_the_thumb_scrolls_and_stays_captured() {
        let mut ui = ui();
        // Viewport 40 px of 200: the thumb is 16 px (the minimum) at the top of a 40 px track.
        assert!(ui.press(Button::Left, ON_BAR).consumed);
        assert!(ui.thumb_held(0));
        let r = ui.pointer_moved(ON_BAR + DVec2::new(-200.0, 12.0));
        assert!(r.consumed && r.redraw);
        assert!(offset(&ui) > 0.0);
        ui.pointer_moved(ON_BAR + DVec2::new(0.0, 500.0));
        assert_eq!(offset(&ui), 160.0);
        ui.release(Button::Left, ON_BAR);
        assert!(!ui.pointer_moved(ON_BAR).consumed);
    }

    #[test]
    fn clicking_the_track_jumps() {
        let mut ui = ui();
        ui.press(Button::Left, DVec2::new(386.0, 286.0));
        assert_eq!(offset(&ui), 160.0);
    }

    #[test]
    fn no_bar_when_everything_fits() {
        let mut ui = ui();
        ui.panels[0].body.as_mut().unwrap().content_h = 30.0;
        let r = ui.press(Button::Left, ON_BAR);
        assert!(r.consumed);
        assert!(!ui.thumb_held(0));
    }

    #[test]
    fn resize_relayouts_and_keeps_the_scroll() {
        let mut ui = ui();
        ui.wheel(-3.0, ON_ROW);
        ui.resize(DVec2::new(500.0, 350.0), 1.0);
        assert_eq!(ui.panels[0].rect.x, 390.0);
        assert_eq!(offset(&ui), 60.0);
        assert!(ui.press(Button::Left, DVec2::new(480.0, 290.0)).consumed);
    }

    #[test]
    fn changes_are_reported_once() {
        let mut ui = ui();
        assert_eq!(ui.take_changes(), vec![(0, true)]);
        assert!(ui.take_changes().is_empty());
        ui.pointer_moved(ON_BTN);
        assert_eq!(ui.take_changes(), vec![(0, true)]);
        ui.set_visible("dlg", false);
        assert_eq!(ui.take_changes(), vec![(0, false)]);
    }

    #[test]
    fn hiding_during_a_press_releases_cleanly() {
        let mut ui = ui();
        ui.press(Button::Left, ON_BTN);
        ui.set_visible("dlg", false);
        let r = ui.release(Button::Left, ON_BTN);
        assert!(r.consumed);
        assert_eq!(r.event, None);
    }

    const SLIDER: WidgetId = 9;

    /// A 200 px bar with a 10 px thumb at the bottom left of a 300x100 screen.
    fn slider_layout(p: &mut Panel, m: &Metrics) {
        let (x, y) = (10.0 * m.scale, 10.0 * m.scale);
        p.rect = Rect::new(x, y, 220.0 * m.scale, 30.0 * m.scale);
        let pos = p.fixed.first().and_then(|w| match w.kind {
            Kind::Slider { pos, .. } => Some(pos),
            _ => None,
        });
        p.fixed = vec![Widget::slider(SLIDER, Rect::new(10.0 * m.scale, 5.0 * m.scale, 200.0 * m.scale, 12.0 * m.scale), pos.unwrap_or(0.25), 10.0 * m.scale)];
    }

    fn slider_ui(scale: f64) -> Ui {
        let mut ui = Ui::new();
        ui.resize(DVec2::new(300.0 * scale, 100.0 * scale), scale);
        ui.add(Panel::new("sl", true, slider_layout));
        ui.set_visible("sl", true);
        ui
    }

    /// Where the thumb of the slider is on the screen, as its left edge and middle height.
    fn thumb_at(ui: &Ui, scale: f64) -> DVec2 {
        let p = &ui.panels()[0];
        let Kind::Slider { pos, thumb_w } = p.fixed[0].kind else { panic!() };
        let t = slider_thumb(p.fixed[0].rect, thumb_w, pos);
        DVec2::new(p.rect.x + t.x, p.rect.y + t.y + 6.0 * scale)
    }

    fn slid(r: &Response) -> (f64, Drag) {
        match r.event {
            Some(UiEvent::Slid { pos, drag, .. }) => (pos, drag),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn grabbing_the_thumb_does_not_move_it_and_the_drag_keeps_the_grab_offset() {
        for scale in [1.0, 2.0] {
            let mut ui = slider_ui(scale);
            let thumb = thumb_at(&ui, scale);
            // 190 px of travel at scale 1: the thumb is at a quarter of it. Grab 3 design pixels in.
            let grab = thumb + DVec2::new(3.0 * scale, 0.0);
            let r = ui.press(Button::Left, grab);
            assert!(r.consumed);
            let (pos, drag) = slid(&r);
            assert_eq!(drag, Drag::Grab);
            assert!((pos - 0.25).abs() < 1e-9, "no jump on grab at scale {scale}: {pos}");
            assert_eq!(ui.slider_held(), Some(("sl", SLIDER)));
            // Moving by a tenth of the travel moves the position by a tenth, wherever the grab was.
            let r = ui.pointer_moved(grab + DVec2::new(19.0 * scale, 40.0 * scale));
            let (pos, drag) = slid(&r);
            assert_eq!(drag, Drag::Move);
            assert!(r.consumed && r.redraw);
            assert!((pos - 0.35).abs() < 1e-9, "{pos}");
            // Past either end it stops at the end.
            assert_eq!(slid(&ui.pointer_moved(DVec2::new(5000.0, 0.0))).0, 1.0);
            assert_eq!(slid(&ui.pointer_moved(DVec2::new(-5000.0, 0.0))).0, 0.0);
            let r = ui.release(Button::Left, DVec2::new(-5000.0, 0.0));
            assert_eq!(slid(&r), (0.0, Drag::End));
            assert_eq!(ui.slider_held(), None);
            assert!(!ui.pointer_moved(grab).consumed, "the pointer is the map's again");
        }
    }

    #[test]
    fn a_press_on_the_bar_jumps_the_thumb_under_the_pointer_and_drags_on() {
        for scale in [1.0, 2.0] {
            let mut ui = slider_ui(scale);
            let p = ui.panels()[0].rect;
            // Bar runs from panel x + 10 to + 210 design pixels; the thumb centre is the pointer.
            let target = DVec2::new(p.x + (10.0 + 5.0 + 0.5 * 190.0) * scale, p.y + 11.0 * scale);
            let (pos, drag) = slid(&ui.press(Button::Left, target));
            assert_eq!(drag, Drag::Jump);
            assert!((pos - 0.5).abs() < 1e-9, "{pos}");
            let (pos, _) = slid(&ui.pointer_moved(target + DVec2::new(19.0 * scale, 0.0)));
            assert!((pos - 0.6).abs() < 1e-9, "{pos}");
        }
    }

    #[test]
    fn the_wheel_over_a_slider_steps_and_elsewhere_in_the_panel_does_nothing() {
        let mut ui = slider_ui(1.0);
        let on = thumb_at(&ui, 1.0);
        let r = ui.wheel(2.0, on);
        assert!(r.consumed);
        assert!(matches!(r.event, Some(UiEvent::Stepped { widget: SLIDER, steps: 2, .. })));
        let r = ui.wheel(-0.2, on);
        assert!(matches!(r.event, Some(UiEvent::Stepped { steps: -1, .. })), "a small scroll is one notch");
        let off = DVec2::new(ui.panels()[0].rect.x + 3.0, ui.panels()[0].rect.y + 28.0);
        let r = ui.wheel(1.0, off);
        assert!(r.consumed && r.event.is_none());
    }

    #[test]
    fn closing_a_panel_during_a_slider_drag_lets_go_of_it() {
        let mut ui = slider_ui(1.0);
        let on = thumb_at(&ui, 1.0) + DVec2::new(2.0, 0.0);
        ui.press(Button::Left, on);
        assert!(ui.slider_held().is_some());
        ui.set_visible("sl", false);
        assert_eq!(ui.slider_held(), None);
        let r = ui.release(Button::Left, on);
        assert!(r.consumed && r.event.is_none());
    }
}
