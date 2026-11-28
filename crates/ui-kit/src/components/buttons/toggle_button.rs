use egui::{Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2};

use crate::{Theme, primitives::FocusRing};

use super::button_state::ButtonState;

/// A labeled toggle button that latches on/off.
pub struct ToggleButton<'a> {
    label: String,
    active: bool,
    theme: Option<&'a Theme>,
    min_width: f32,
    height: f32,
    enabled: bool,
}

impl<'a> ToggleButton<'a> {
    pub fn new(label: impl Into<String>, active: bool) -> Self {
        Self {
            label: label.into(),
            active,
            theme: None,
            min_width: 56.0,
            height: 24.0,
            enabled: true,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }

    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
}

impl Widget for ToggleButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let font = egui::FontId::proportional(theme.typography.control_label);
        let galley = ui
            .painter()
            .layout_no_wrap(self.label.clone(), font.clone(), Color32::WHITE);
        let desired = Vec2::new(
            (galley.size().x + m.spacing.md * 2.0).max(self.min_width),
            self.height,
        );

        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(desired, sense);

        if ui.is_rect_visible(rect) {
            let state = ButtonState::from_response(&response, self.active, !self.enabled);
            let painter = ui.painter();
            let radius = m.radii.control;
            let rounding = CornerRadius::same(radius as u8);

            let fill = match state {
                ButtonState::Active => p.accent_active,
                ButtonState::Pressed => p.control_pressed,
                ButtonState::Hovered => p.control_hover,
                ButtonState::Disabled => p.control.gamma_multiply(0.5),
                _ => p.control,
            };
            painter.rect_filled(rect, rounding, fill);

            if state.is_accent() {
                painter.line_segment(
                    [
                        pos2(rect.left() + radius, rect.top() + 0.5),
                        pos2(rect.right() - radius, rect.top() + 0.5),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(48)),
                );
            }

            let border_color = match state {
                ButtonState::Active => p.accent,
                ButtonState::Hovered => p.border_strong,
                ButtonState::Disabled => p.border.gamma_multiply(0.5),
                _ => p.border,
            };
            painter.rect_stroke(
                rect,
                rounding,
                Stroke::new(1.0, border_color),
                StrokeKind::Inside,
            );

            FocusRing::paint(painter, rect, theme, matches!(state, ButtonState::Focused));

            let text_color = match state {
                ButtonState::Active => p.text_primary,
                ButtonState::Hovered | ButtonState::Pressed => p.text_primary,
                ButtonState::Disabled => p.text_muted,
                _ => p.text_secondary,
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &self.label,
                font,
                text_color,
            );
        }

        response
    }
}
