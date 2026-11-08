//! The interface, drawn with egui over the scene: windows, the row of icons, the readout and the
//! credits. The scene builds it afresh each frame from its own state; what the user does comes
//! back as actions the scene carries out, so the dialogs keep no state of their own.
//!
//! Input reaches egui through the scene's own mouse handling ([`Gui::pointer_moved`],
//! [`Gui::press`] and so on), which is the same for the window, the browser and simulated clicks.
//! A press over a window belongs to the interface until it is let go; anything else goes to the
//! globe or the map.
//!
//! The look follows KDE's Breeze: Noto Sans (Plasma's default font), Breeze's dark and light
//! colours with its blue accent, rounded corners and see-through windows over the imagery.

use crate::nav::Button;
use crate::wtime::Instant;
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2};
use glam::DVec2;
use std::collections::HashSet;
use std::sync::Arc;

pub type PanelId = &'static str;

/// The family of the bold font, for headings.
pub const BOLD: &str = "bold";

/// Breeze's accent, the blue of selections and links.
const ACCENT: Color32 = Color32::from_rgb(61, 174, 233);

/// What is to be drawn this frame, once the interface was built.
struct Paint {
    primitives: Vec<egui::ClippedPrimitive>,
    textures: egui::TexturesDelta,
    screen: egui_wgpu::ScreenDescriptor,
}

pub struct Gui {
    pub ctx: egui::Context,
    renderer: Option<(egui_wgpu::Renderer, wgpu::TextureFormat)>,
    events: Vec<egui::Event>,
    /// Where the pointer is, in pixels; `None` once it left the window.
    pointer: Option<DVec2>,
    /// Pixels per point: the window's scale factor.
    ppp: f32,
    size: [u32; 2],
    open: HashSet<PanelId>,
    /// A press began over the interface, so the release belongs to it too.
    held: bool,
    paint: Option<Paint>,
    /// Textures freed by the last frame, freed after it was drawn.
    free: Vec<egui::TextureId>,
    started: Instant,
    dark: bool,
    /// Something in the interface moves on its own, such as a window fading in.
    animating: bool,
    /// Addresses the user clicked, for the scene to open in the browser.
    pub links: Vec<String>,
    /// Screen areas (x0, y0, x1, y1) in pixels that windows covered in the last frame, for labels to keep clear of.
    covered: Vec<[f64; 4]>,
}

impl Default for Gui {
    fn default() -> Self {
        Self::new()
    }
}

impl Gui {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let gui = Gui {
            ctx,
            renderer: None,
            events: Vec::new(),
            pointer: None,
            ppp: 1.0,
            size: [800, 600],
            open: HashSet::new(),
            held: false,
            paint: None,
            free: Vec::new(),
            started: Instant::now(),
            dark: true,
            animating: false,
            links: Vec::new(),
            covered: Vec::new(),
        };
        gui.apply_theme();
        gui
    }

    /// Dark or light, as the desktop or the browser prefers.
    pub fn set_dark(&mut self, dark: bool) {
        if self.dark != dark {
            self.dark = dark;
            self.apply_theme();
        }
    }

    fn apply_theme(&self) {
        self.ctx.set_visuals(visuals(self.dark));
        self.ctx.all_styles_mut(|style| {
            style.text_styles = [
                (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(15.5, FontFamily::Name(BOLD.into()))),
                (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
            ]
            .into();
            let s = &mut style.spacing;
            s.item_spacing = Vec2::new(8.0, 5.0);
            s.button_padding = Vec2::new(10.0, 4.0);
            s.window_margin = Margin::same(12);
            s.interact_size = Vec2::new(24.0, 24.0);
            s.slider_width = 180.0;
            s.slider_rail_height = 4.0;
            s.icon_width = 16.0;
            s.icon_width_inner = 9.0;
            s.icon_spacing = 6.0;
            style.interaction.tooltip_delay = 0.4;
        });
    }

    pub fn is_visible(&self, id: PanelId) -> bool {
        self.open.contains(id)
    }

    pub fn set_visible(&mut self, id: PanelId, visible: bool) {
        if visible {
            self.open.insert(id);
        } else {
            self.open.remove(id);
        }
    }

    pub fn toggle(&mut self, id: PanelId) {
        let open = !self.is_visible(id);
        self.set_visible(id, open);
    }

    /// Whether a press that began over the interface is still down, such as on a slider.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Whether a window or icon of the interface lies under a position, in pixels.
    pub fn covers(&self, p: DVec2) -> bool {
        let pos = egui::pos2((p.x / self.ppp as f64) as f32, (p.y / self.ppp as f64) as f32);
        self.ctx.layer_id_at(pos).is_some_and(|l| l.order != egui::Order::Background)
    }

    /// The areas windows covered in the last frame, in pixels, as (x0, y0, x1, y1).
    pub fn covered(&self) -> &[[f64; 4]] {
        &self.covered
    }

    /// Notes a window's area as covered, in points.
    pub fn cover(&mut self, r: egui::Rect) {
        let k = self.ppp as f64;
        self.covered.push([r.min.x as f64 * k, r.min.y as f64 * k, r.max.x as f64 * k, r.max.y as f64 * k]);
    }

    fn pos(&self, p: DVec2) -> egui::Pos2 {
        egui::pos2((p.x / self.ppp as f64) as f32, (p.y / self.ppp as f64) as f32)
    }

    /// The pointer moved; true if the interface takes it (it is over a window, or dragging in one).
    pub fn pointer_moved(&mut self, p: DVec2) -> bool {
        let inside = p.x >= 0.0 && p.y >= 0.0;
        if inside {
            self.pointer = Some(p);
            self.events.push(egui::Event::PointerMoved(self.pos(p)));
        } else {
            self.pointer = None;
            self.events.push(egui::Event::PointerGone);
        }
        self.held || (inside && self.covers(p))
    }

    /// A button went down; true if the interface takes it.
    pub fn press(&mut self, b: Button) -> bool {
        let Some(p) = self.pointer.filter(|p| self.covers(*p)) else { return false };
        self.held = true;
        self.events.push(egui::Event::PointerButton { pos: self.pos(p), button: button(b), pressed: true, modifiers: Default::default() });
        true
    }

    /// A button went up; true if it belongs to the interface, which had its press.
    pub fn release(&mut self, b: Button) -> bool {
        let pos = self.pointer.map_or(egui::Pos2::ZERO, |p| self.pos(p));
        // Always passed on, so that egui never thinks a button stays down.
        self.events.push(egui::Event::PointerButton { pos, button: button(b), pressed: false, modifiers: Default::default() });
        std::mem::take(&mut self.held)
    }

    /// Wheel notches over the interface scroll it; true if it took them.
    pub fn wheel(&mut self, lines: f64) -> bool {
        if !self.pointer.is_some_and(|p| self.covers(p)) {
            return false;
        }
        self.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, lines as f32),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Wheel,
        });
        true
    }

    /// The mouse was lost to another window: nothing stays held.
    pub fn mouse_lost(&mut self) {
        if self.held {
            self.release(Button::Left);
        }
        self.pointer = None;
        self.events.push(egui::Event::PointerGone);
    }

    /// The input for a frame `size` pixels large at `ppp` pixels per point.
    pub fn begin(&mut self, size: (u32, u32), ppp: f32) -> egui::RawInput {
        self.size = [size.0.max(1), size.1.max(1)];
        if (self.ppp - ppp).abs() > f32::EPSILON {
            // Positions already queued were in the old scale.
            self.ppp = ppp;
        }
        self.covered.clear();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(self.size[0] as f32 / ppp, self.size[1] as f32 / ppp))),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        raw
    }

    /// Takes in what the frame built: what to draw, and links that were clicked.
    pub fn finish(&mut self, out: egui::FullOutput) {
        for cmd in out.platform_output.commands {
            if let egui::OutputCommand::OpenUrl(u) = cmd {
                self.links.push(u.url);
            }
        }
        self.animating = out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.is_zero());
        let primitives = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: self.size, pixels_per_point: out.pixels_per_point };
        // Textures from an earlier frame that was never drawn are kept, with this frame's changes after them.
        let mut textures = self.paint.take().map(|p| p.textures).unwrap_or_default();
        textures.append(out.textures_delta);
        self.paint = Some(Paint { primitives, textures, screen });
    }

    /// Whether the interface wants another frame soon, such as for an animation.
    pub fn animating(&self) -> bool {
        self.animating
    }

    /// Uploads this frame's textures and vertices. Run before the frame's drawing is submitted.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) {
        if self.renderer.as_ref().is_none_or(|r| r.1 != format) {
            let options = egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, predictable_texture_filtering: false };
            self.renderer = Some((egui_wgpu::Renderer::new(device, format, options), format));
        }
        let (Some((renderer, _)), Some(paint)) = (&mut self.renderer, &mut self.paint) else { return };
        for id in self.free.drain(..) {
            renderer.free_texture(&id);
        }
        for (id, deltas) in &paint.textures.set {
            for delta in deltas {
                renderer.update_texture(device, queue, *id, delta);
            }
        }
        self.free.extend(paint.textures.free.drain());
        paint.textures.set.clear();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("interface upload") });
        let extra = renderer.update_buffers(device, queue, &mut encoder, &paint.primitives, &paint.screen);
        queue.submit(std::iter::once(encoder.finish()).chain(extra));
    }

    /// Draws the interface over what `target` holds.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let (Some((renderer, _)), Some(paint)) = (&self.renderer, &self.paint) else { return };
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        renderer.render(&mut pass, &paint.primitives, &paint.screen);
    }
}

fn button(b: Button) -> egui::PointerButton {
    match b {
        Button::Left => egui::PointerButton::Primary,
        Button::Right => egui::PointerButton::Secondary,
    }
}

/// Noto Sans, Plasma's default font, with the symbols the interface and the labels use; egui's
/// own fonts stay as fallbacks for other scripts and emoji.
fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, data: &'static [u8]| {
        f.font_data.insert(name.to_string(), Arc::new(FontData::from_static(data)));
    };
    add(&mut f, "noto", include_bytes!("../web/fonts/NotoSans-Regular.ttf"));
    add(&mut f, "noto-bold", include_bytes!("../web/fonts/NotoSans-Bold.ttf"));
    add(&mut f, "symbols", include_bytes!("../web/fonts/NotoSansSymbols2-Subset.ttf"));
    let proportional = f.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "noto".into());
    proportional.insert(1, "symbols".into());
    let fallback = proportional.clone();
    let mut bold = vec!["noto-bold".to_string()];
    bold.extend(fallback.into_iter().filter(|n| n != "noto"));
    f.families.insert(FontFamily::Name(BOLD.into()), bold);
    f.families.entry(FontFamily::Monospace).or_default().push("symbols".into());
    f
}

/// Breeze's colours: Plasma's Breeze Dark or Breeze Light.
fn visuals(dark: bool) -> egui::Visuals {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let (window, view, button, text, weak, border) = if dark {
        (
            Color32::from_rgb(32, 35, 38),
            Color32::from_rgb(20, 22, 24),
            Color32::from_rgb(49, 54, 59),
            Color32::from_rgb(252, 252, 252),
            Color32::from_rgb(161, 169, 177),
            Color32::from_rgba_unmultiplied(255, 255, 255, 34),
        )
    } else {
        (
            Color32::from_rgb(239, 240, 241),
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(252, 252, 252),
            Color32::from_rgb(35, 38, 41),
            Color32::from_rgb(112, 125, 138),
            Color32::from_rgba_unmultiplied(0, 0, 0, 40),
        )
    };
    let radius = CornerRadius::same(5);
    v.override_text_color = Some(text);
    v.weak_text_color = Some(weak);
    v.window_fill = window;
    v.panel_fill = window;
    v.window_stroke = Stroke::new(1.0, border);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = radius;
    v.window_shadow = Shadow { offset: [0, 4], blur: 18, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 50 }) };
    v.popup_shadow = Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(80) };
    v.window_highlight_topmost = false;
    v.extreme_bg_color = view;
    v.faint_bg_color = if dark { Color32::from_rgb(38, 42, 46) } else { Color32::from_rgb(247, 247, 247) };
    v.hyperlink_color = if dark { Color32::from_rgb(29, 153, 243) } else { Color32::from_rgb(41, 128, 185) };
    v.selection.bg_fill = ACCENT.linear_multiply(if dark { 0.45 } else { 0.6 });
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.warn_fg_color = Color32::from_rgb(246, 116, 0);
    v.error_fg_color = Color32::from_rgb(218, 68, 83);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, text);
    w.noninteractive.corner_radius = radius;
    for (state, fill, stroke) in [
        (&mut w.inactive, button, border),
        (&mut w.hovered, button, ACCENT),
        (&mut w.active, ACCENT.linear_multiply(0.5), ACCENT),
        (&mut w.open, button, ACCENT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.5, text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    v
}

/// A window of the interface in the house style: a title bar with a close button, not
/// collapsible, placed at `pos` (points) until the user moves it.
pub fn window(title: &str, id: PanelId) -> egui::Window<'_> {
    egui::Window::new(egui::RichText::new(title).text_style(TextStyle::Heading))
        .id(egui::Id::unique(id))
        .collapsible(false)
        .resizable(false)
}

/// A heading inside a window.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text).family(FontFamily::Name(BOLD.into())).size(13.5));
}

/// Grey text, for explanations.
pub fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak().size(12.0));
}

/// The accent colour, for marks that should stand out.
pub fn accent() -> Color32 {
    ACCENT
}

/// The colour of a warning.
pub fn warn_colour(ui: &egui::Ui) -> Color32 {
    ui.visuals().warn_fg_color
}

/// What the user did to a [`track`] slider this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slide {
    /// Pressed on the thumb: it keeps its place until moved.
    Grab,
    /// Pressed on the bar beside the thumb: the value jumps there.
    Jump(f64),
    /// Dragged to this position, 0 to 1.
    Move(f64),
    /// Let go.
    End,
    /// Wheel notches over the slider, up counting as forward.
    Step(i32),
}

const THUMB: f32 = 16.0;

/// A slider over positions 0 to 1, `width` points wide, that tells apart a grab of the thumb from
/// a press on the bar, and steps with the wheel.
pub fn track(ui: &mut egui::Ui, pos: f64, width: f32) -> Vec<Slide> {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 20.0), egui::Sense::drag());
    let id = response.id;
    let travel = (rect.width() - THUMB).max(1.0);
    let thumb_x = rect.left() + THUMB / 2.0 + pos.clamp(0.0, 1.0) as f32 * travel;
    let at = |x: f32, grab: f32| (((x - grab - rect.left() - THUMB / 2.0) / travel).clamp(0.0, 1.0)) as f64;
    let held: Option<f32> = ui.data(|d| d.get_temp(id));
    let down = response.is_pointer_button_down_on();
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut out = Vec::new();
    match (held, down, pointer) {
        (None, true, Some(p)) => {
            let grab = if (p.x - thumb_x).abs() <= THUMB / 2.0 + 2.0 {
                out.push(Slide::Grab);
                p.x - thumb_x
            } else {
                out.push(Slide::Jump(at(p.x, 0.0)));
                0.0
            };
            ui.data_mut(|d| d.insert_temp(id, grab));
        }
        (Some(grab), true, Some(p)) => {
            let to = at(p.x, grab);
            if (to - pos).abs() > 1e-9 {
                out.push(Slide::Move(to));
            }
        }
        (Some(_), false, _) => {
            out.push(Slide::End);
            ui.data_mut(|d| d.remove::<f32>(id));
        }
        _ => {}
    }
    if response.hovered() && !down {
        // Notches of the wheel as they came, before egui smooths them into scrolling.
        let dy: f32 = ui.input(|i| i.raw.events.iter().map(|e| if let egui::Event::MouseWheel { delta, .. } = e { delta.y } else { 0.0 }).sum());
        if dy != 0.0 {
            out.push(Slide::Step(dy.signum() as i32));
        }
    }
    let v = ui.visuals();
    let rail = egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width() - THUMB, 4.0));
    let p = ui.painter();
    p.rect_filled(rail, 2.0, v.widgets.inactive.bg_fill.gamma_multiply(1.6));
    p.rect_filled(egui::Rect::from_min_max(rail.min, egui::pos2(thumb_x, rail.max.y)), 2.0, ACCENT);
    let lit = response.hovered() || down;
    p.circle(egui::pos2(thumb_x, rect.center().y), THUMB / 2.0 - 1.0, v.widgets.inactive.bg_fill, Stroke::new(if lit { 2.0 } else { 1.0 }, if lit { ACCENT } else { v.widgets.inactive.bg_stroke.color }));
    out
}

/// Dark diagonal stripes over `r`, the hatching of a fossil source. The stripes lie on lines of
/// equal x + y, so neighbouring rectangles join up into one pattern.
pub fn hatch(p: &egui::Painter, r: egui::Rect, colour: [u8; 3]) {
    const STEP: f32 = 6.0;
    let dark = Color32::from_rgb((colour[0] as f32 * 0.4) as u8, (colour[1] as f32 * 0.4) as u8, (colour[2] as f32 * 0.4) as u8);
    let stroke = Stroke::new(1.6, dark);
    let first = ((r.min.x + r.min.y) / STEP).ceil() * STEP;
    let mut c = first;
    while c <= r.max.x + r.max.y {
        // The line x + y = c, cut to the rectangle.
        let xa = r.min.x.max(c - r.max.y);
        let xb = r.max.x.min(c - r.min.y);
        if xb > xa {
            p.line_segment([egui::pos2(xa, c - xa), egui::pos2(xb, c - xb)], stroke);
        }
        c += STEP;
    }
}

/// A patch of colour with straight alpha for a legend, hatched for a fossil source.
pub fn swatch(ui: &mut egui::Ui, colour: [u8; 4], hatched: bool) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
    paint_swatch(ui, r, colour, hatched);
}

/// A legend entry: a swatch and its name in small text, kept together when the line wraps.
pub fn legend_entry(ui: &mut egui::Ui, colour: [u8; 4], hatched: bool, name: &str) {
    let font = ui.style().text_styles[&TextStyle::Small].clone();
    let galley = ui.painter().layout_no_wrap(name.to_string(), font, ui.visuals().text_color());
    let size = Vec2::new(14.0 + 4.0 + galley.size().x, 14.0_f32.max(galley.size().y));
    let (r, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let patch = egui::Rect::from_min_size(egui::pos2(r.min.x, r.center().y - 7.0), Vec2::splat(14.0));
    paint_swatch(ui, patch, colour, hatched);
    let text_at = egui::pos2(patch.max.x + 4.0, r.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_at, galley, ui.visuals().text_color());
}

fn paint_swatch(ui: &egui::Ui, r: egui::Rect, colour: [u8; 4], hatched: bool) {
    let p = ui.painter().with_clip_rect(r);
    p.rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(colour[0], colour[1], colour[2], colour[3]));
    if hatched {
        hatch(&p, r, [colour[0], colour[1], colour[2]]);
    }
}
