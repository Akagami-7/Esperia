//! Slint-backed Count-In & Metronome toggle widget (`ui/count_metronome_button.slint`).
//!
//! All layout, squircle gradients, shadows, animations, and vector icons are
//! defined declaratively in Slint and rendered via Slint's software renderer.

use eframe::egui::{
    self, Color32, ColorImage, Response, Sense, TextureHandle, TextureOptions, Vec2,
};
use slint::ComponentHandle;
use slint::LogicalPosition;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

slint::include_modules!();

const WIDTH: u32 = 94;
const HEIGHT: u32 = 40;

thread_local! {
    static SLINT_WINDOW: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct SlintBridgePlatform {
    start: Instant,
}

impl Platform for SlintBridgePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
        SLINT_WINDOW.with(|slot| {
            *slot.borrow_mut() = Some(window.clone());
        });
        Ok(window)
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

fn ensure_slint_platform() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let _ = slint::platform::set_platform(Box::new(SlintBridgePlatform {
            start: Instant::now(),
        }));
    });
}

/// Slint-powered Count-In and Metronome control button pair.
pub struct CountMetronomeButton {
    pub countin_active: bool,
    pub metronome_active: bool,
    widget: Option<CountMetronomeWidget>,
    window: Option<Rc<MinimalSoftwareWindow>>,
    pixels: Vec<PremultipliedRgbaColor>,
    texture: Option<TextureHandle>,
    was_hovered: bool,
    was_pressed: bool,
}

impl Default for CountMetronomeButton {
    fn default() -> Self {
        Self {
            countin_active: false,
            metronome_active: false,
            widget: None,
            window: None,
            pixels: vec![PremultipliedRgbaColor::default(); (WIDTH * HEIGHT) as usize],
            texture: None,
            was_hovered: false,
            was_pressed: false,
        }
    }
}

impl CountMetronomeButton {
    fn ensure_initialized(&mut self) {
        if self.widget.is_some() {
            return;
        }
        ensure_slint_platform();
        if let Ok(widget) = CountMetronomeWidget::new() {
            widget.set_countin_active(self.countin_active);
            widget.set_metronome_active(self.metronome_active);
            self.window = SLINT_WINDOW.with(|slot| slot.borrow_mut().take());
            if let Some(ref win) = self.window {
                win.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
            }
            let _ = widget.show();
            self.widget = Some(widget);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, background: Color32) -> Response {
        self.ensure_initialized();

        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(WIDTH as f32, HEIGHT as f32),
            Sense::click_and_drag(),
        );

        let Some(ref widget) = self.widget else {
            return response;
        };
        let Some(ref window) = self.window else {
            return response;
        };

        // Match hover lift tint against the host control-bar background.
        let luma = 0.2126 * f32::from(background.r())
            + 0.7152 * f32::from(background.g())
            + 0.0722 * f32::from(background.b());
        let amount = if luma < 128.0 { 18.0 } else { -18.0 };
        let hover_r = (f32::from(background.r()) + amount).clamp(0.0, 255.0) as u8;
        let hover_g = (f32::from(background.g()) + amount).clamp(0.0, 255.0) as u8;
        let hover_b = (f32::from(background.b()) + amount).clamp(0.0, 255.0) as u8;

        widget.set_surface_color(slint::Color::from_rgb_u8(
            background.r(),
            background.g(),
            background.b(),
        ));
        widget.set_hover_color(slint::Color::from_rgb_u8(hover_r, hover_g, hover_b));

        // Forward pointer state from host rect into Slint's MinimalSoftwareWindow.
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        let interact_pos = ui.input(|i| i.pointer.interact_pos());
        let is_down = response.is_pointer_button_down_on();

        if let Some(pos) = hover_pos.filter(|p| rect.contains(*p)) {
            let local = LogicalPosition::new(pos.x - rect.left(), pos.y - rect.top());
            window.dispatch_event(WindowEvent::PointerMoved { position: local });
            self.was_hovered = true;
        } else if self.was_hovered && !is_down {
            window.dispatch_event(WindowEvent::PointerExited);
            self.was_hovered = false;
        }

        if is_down && !self.was_pressed {
            if let Some(pos) = interact_pos.or(hover_pos) {
                let local = LogicalPosition::new(pos.x - rect.left(), pos.y - rect.top());
                window.dispatch_event(WindowEvent::PointerPressed {
                    position: local,
                    button: PointerEventButton::Left,
                });
            }
            self.was_pressed = true;
        } else if !is_down && self.was_pressed {
            if let Some(pos) = interact_pos.or(hover_pos) {
                let local = LogicalPosition::new(pos.x - rect.left(), pos.y - rect.top());
                window.dispatch_event(WindowEvent::PointerReleased {
                    position: local,
                    button: PointerEventButton::Left,
                });
            }
            self.was_pressed = false;
        }

        // Advance Slint animations and render if dirty.
        slint::platform::update_timers_and_animations();
        if window.has_active_animations() {
            ui.ctx().request_repaint();
        }

        let redrawn = window.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels, WIDTH as usize);
        });

        if redrawn || self.texture.is_none() {
            let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
            for p in &self.pixels {
                // Convert premultiplied RGBA to straight RGBA for egui ColorImage.
                if p.alpha == 0 {
                    rgba.extend_from_slice(&[0, 0, 0, 0]);
                } else if p.alpha == 255 {
                    rgba.extend_from_slice(&[p.red, p.green, p.blue, 255]);
                } else {
                    let a = f32::from(p.alpha);
                    let r = ((f32::from(p.red) * 255.0) / a).min(255.0) as u8;
                    let g = ((f32::from(p.green) * 255.0) / a).min(255.0) as u8;
                    let b = ((f32::from(p.blue) * 255.0) / a).min(255.0) as u8;
                    rgba.extend_from_slice(&[r, g, b, p.alpha]);
                }
            }
            let image =
                ColorImage::from_rgba_unmultiplied([WIDTH as usize, HEIGHT as usize], &rgba);
            match &mut self.texture {
                Some(tex) => tex.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "count_metronome_slint",
                        image,
                        TextureOptions::LINEAR,
                    ));
                }
            }
        }

        if let Some(ref tex) = self.texture {
            ui.painter().image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // Sync toggled state back to public fields.
        let new_countin = widget.get_countin_active();
        let new_metronome = widget.get_metronome_active();
        if new_countin != self.countin_active || new_metronome != self.metronome_active {
            self.countin_active = new_countin;
            self.metronome_active = new_metronome;
            response.mark_changed();
        }

        response
    }
}
