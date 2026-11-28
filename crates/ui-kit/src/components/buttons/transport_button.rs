use egui::{
    Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2,
};

use crate::{
    Theme,
    primitives::{FocusRing, Shadow, shadow},
};

use super::button_state::ButtonState;

/// Transport-deck button variant (Play, Pause, Stop, Record).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportVariant {
    Play,
    Pause,
    Stop,
    Record,
    Generic,
}

/// Tactile transport-deck button with semantic playback/record color states.
pub struct TransportButton<'a, F>
where
    F: FnOnce(&mut Ui, Rect, Color32),
{
    icon_fn: F,
    variant: TransportVariant,
    theme: Option<&'a Theme>,
    active: bool,
    enabled: bool,
    size: f32,
}

impl<'a, F> TransportButton<'a, F>
where
    F: FnOnce(&mut Ui, Rect, Color32),
{
    pub fn new(icon_fn: F, variant: TransportVariant) -> Self {
        Self {
            icon_fn,
            variant,
            theme: None,
            active: false,
            enabled: true,
            size: 36.0,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
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

    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
}

impl<F> Widget for TransportButton<'_, F>
where
    F: FnOnce(&mut Ui, Rect, Color32),
{
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let desired = Vec2::splat(self.size);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(desired, sense);

        if ui.is_rect_visible(rect) {
            let state = ButtonState::from_response(&response, self.active, !self.enabled);
            let painter = ui.painter();
            let radius = m.radii.button + 2.0;
            let rounding = CornerRadius::same(radius as u8);

            let scale = state.press_scale();
            let draw_rect = if (scale - 1.0).abs() > f32::EPSILON {
                Rect::from_center_size(rect.center(), rect.size() * scale)
            } else {
                rect
            };

            if !matches!(state, ButtonState::Pressed | ButtonState::Disabled) {
                shadow(painter, draw_rect, radius, Shadow::control(theme));
            }

            let fill = match self.variant {
                TransportVariant::Record => match state {
                    ButtonState::Active => p.danger,
                    ButtonState::Pressed => p.danger.gamma_multiply(0.75),
                    ButtonState::Hovered => p.control_hover,
                    _ => p.control,
                },
                TransportVariant::Play => match state {
                    ButtonState::Active => p.success.gamma_multiply(0.28),
                    ButtonState::Pressed => p.control_pressed,
                    ButtonState::Hovered => p.control_hover,
                    _ => p.control,
                },
                _ => match state {
                    ButtonState::Active => p.accent_active,
                    ButtonState::Pressed => p.control_pressed,
                    ButtonState::Hovered => p.control_hover,
                    _ => p.control,
                },
            };
            painter.rect_filled(draw_rect, rounding, fill);

            if !matches!(state, ButtonState::Pressed | ButtonState::Disabled) {
                painter.line_segment(
                    [
                        pos2(draw_rect.left() + radius, draw_rect.top() + 0.5),
                        pos2(draw_rect.right() - radius, draw_rect.top() + 0.5),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(28)),
                );
            }

            let border = match (self.variant, state) {
                (TransportVariant::Record, ButtonState::Active) => p.danger,
                (TransportVariant::Play, ButtonState::Active) => p.success,
                (_, ButtonState::Active) => p.accent,
                (_, ButtonState::Hovered) => p.border_strong,
                _ => p.border,
            };
            painter.rect_stroke(
                draw_rect,
                rounding,
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );

            FocusRing::paint(
                painter,
                draw_rect,
                theme,
                matches!(state, ButtonState::Focused),
            );

            let icon_col = match (self.variant, state) {
                (TransportVariant::Record, ButtonState::Active) => p.text_primary,
                (TransportVariant::Record, _) => p.danger,
                (TransportVariant::Play, ButtonState::Active) => p.success,
                (_, ButtonState::Active | ButtonState::Hovered | ButtonState::Pressed) => {
                    p.text_primary
                }
                (_, ButtonState::Disabled) => p.text_muted,
                _ => p.text_secondary,
            };
            (self.icon_fn)(ui, draw_rect, icon_col);
        }

        response
    }
}
