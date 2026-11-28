use egui::{
    Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2,
};

use crate::{Theme, primitives::FocusRing};

use super::button_state::ButtonState;

/// A square icon-only button that delegates icon drawing to the caller.
pub struct IconButton<'a, F>
where
    F: FnOnce(&mut Ui, Rect, Color32),
{
    icon_fn: F,
    theme: Option<&'a Theme>,
    size: f32,
    active: bool,
    enabled: bool,
    tooltip: Option<String>,
}

impl<'a, F> IconButton<'a, F>
where
    F: FnOnce(&mut Ui, Rect, Color32),
{
    pub fn new(icon_fn: F) -> Self {
        Self {
            icon_fn,
            theme: None,
            size: 32.0,
            active: false,
            enabled: true,
            tooltip: None,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }

    pub fn active(mut self, yes: bool) -> Self {
        self.active = yes;
        self
    }

    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
}

impl<F> Widget for IconButton<'_, F>
where
    F: FnOnce(&mut Ui, Rect, Color32),
{
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let desired_size = Vec2::splat(self.size);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, mut response) = ui.allocate_exact_size(desired_size, sense);

        if let Some(ref tip) = self.tooltip {
            response = response.on_hover_text(tip.clone());
        }

        if ui.is_rect_visible(rect) {
            let state = ButtonState::from_response(&response, self.active, !self.enabled);
            let painter = ui.painter();
            let radius = m.radii.control;
            let rounding = CornerRadius::same(radius as u8);

            let scale = state.press_scale();
            let draw_rect = if (scale - 1.0).abs() > f32::EPSILON {
                Rect::from_center_size(rect.center(), rect.size() * scale)
            } else {
                rect
            };

            let fill = match state {
                ButtonState::Active => p.accent_active,
                ButtonState::Pressed => p.control_pressed,
                ButtonState::Hovered => p.control_hover,
                _ => Color32::TRANSPARENT,
            };
            painter.rect_filled(draw_rect, rounding, fill);

            if matches!(state, ButtonState::Active) {
                painter.line_segment(
                    [
                        pos2(draw_rect.left() + radius, draw_rect.top() + 0.5),
                        pos2(draw_rect.right() - radius, draw_rect.top() + 0.5),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(48)),
                );
                painter.rect_stroke(
                    draw_rect,
                    rounding,
                    Stroke::new(1.0, p.accent),
                    StrokeKind::Inside,
                );
            } else if matches!(state, ButtonState::Hovered) {
                painter.rect_stroke(
                    draw_rect,
                    rounding,
                    Stroke::new(1.0, p.border),
                    StrokeKind::Inside,
                );
            }

            FocusRing::paint(
                painter,
                draw_rect,
                theme,
                matches!(state, ButtonState::Focused),
            );

            let icon_color = match state {
                ButtonState::Active => p.text_primary,
                ButtonState::Hovered | ButtonState::Pressed => p.text_primary,
                ButtonState::Disabled => p.text_muted,
                _ => p.text_secondary,
            };
            (self.icon_fn)(ui, draw_rect, icon_color);
        }

        response
    }
}
