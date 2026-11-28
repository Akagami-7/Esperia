use egui::{
    Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2,
};

use crate::{
    Theme,
    primitives::{FocusRing, Shadow, shadow},
};

use super::button_state::ButtonState;

/// A secondary satin/outline button for standard workspace actions.
pub struct SecondaryButton<'a> {
    label: String,
    theme: Option<&'a Theme>,
    min_width: f32,
    height: Option<f32>,
    enabled: bool,
}

impl<'a> SecondaryButton<'a> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            theme: None,
            min_width: 72.0,
            height: None,
            enabled: true,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = Some(h);
        self
    }

    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
}

impl Widget for SecondaryButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let font = egui::FontId::proportional(theme.typography.control_label);
        let galley = ui
            .painter()
            .layout_no_wrap(self.label.clone(), font.clone(), Color32::WHITE);
        let desired_size = Vec2::new(
            (galley.size().x + m.spacing.lg * 2.0).max(self.min_width),
            self.height.unwrap_or(m.control_height),
        );

        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(desired_size, sense);

        if ui.is_rect_visible(rect) {
            let state = ButtonState::from_response(&response, false, !self.enabled);
            let painter = ui.painter();
            let radius = m.radii.button;
            let rounding = CornerRadius::same(radius as u8);

            let scale = state.press_scale();
            let draw_rect = if (scale - 1.0).abs() > f32::EPSILON {
                Rect::from_center_size(rect.center(), rect.size() * scale)
            } else {
                rect
            };

            if matches!(state, ButtonState::Hovered | ButtonState::Focused) {
                shadow(painter, draw_rect, radius, Shadow::control(theme));
            }

            let fill = match state {
                ButtonState::Pressed => p.control_pressed,
                ButtonState::Hovered => p.control_hover,
                ButtonState::Disabled => Color32::TRANSPARENT,
                _ => p.control,
            };
            painter.rect_filled(draw_rect, rounding, fill);

            if matches!(state, ButtonState::Default | ButtonState::Hovered) {
                painter.line_segment(
                    [
                        pos2(draw_rect.left() + radius, draw_rect.top() + 0.5),
                        pos2(draw_rect.right() - radius, draw_rect.top() + 0.5),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(16)),
                );
            }

            let border_color = match state {
                ButtonState::Focused => p.accent,
                ButtonState::Hovered => p.border_strong,
                ButtonState::Disabled => p.border.gamma_multiply(0.5),
                _ => p.border,
            };
            painter.rect_stroke(
                draw_rect,
                rounding,
                Stroke::new(1.0, border_color),
                StrokeKind::Inside,
            );

            FocusRing::paint(
                painter,
                draw_rect,
                theme,
                matches!(state, ButtonState::Focused),
            );

            let text_color = match state {
                ButtonState::Disabled => p.text_muted,
                ButtonState::Hovered | ButtonState::Pressed => p.text_primary,
                _ => p.text_secondary,
            };
            painter.text(
                draw_rect.center(),
                egui::Align2::CENTER_CENTER,
                &self.label,
                font,
                text_color,
            );
        }

        response
    }
}
