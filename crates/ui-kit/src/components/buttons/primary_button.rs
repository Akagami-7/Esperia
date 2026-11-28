use egui::{
    Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2,
};

use crate::{
    Theme,
    primitives::{FocusRing, Shadow, shadow},
};

use super::button_state::ButtonState;

/// A primary call-to-action button with Esperia violet accent fill.
pub struct PrimaryButton<'a> {
    label: String,
    theme: Option<&'a Theme>,
    min_width: f32,
    height: Option<f32>,
    enabled: bool,
}

impl<'a> PrimaryButton<'a> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            theme: None,
            min_width: 80.0,
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

impl Widget for PrimaryButton<'_> {
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
            let state = ButtonState::from_response(&response, true, !self.enabled);
            let painter = ui.painter();
            let radius = m.radii.button;
            let rounding = CornerRadius::same(radius as u8);

            let scale = state.press_scale();
            let draw_rect = if (scale - 1.0).abs() > f32::EPSILON {
                Rect::from_center_size(rect.center(), rect.size() * scale)
            } else {
                rect
            };

            if !matches!(state, ButtonState::Disabled | ButtonState::Pressed) {
                shadow(painter, draw_rect, radius, Shadow::control(theme));
            }

            let top_fill = match state {
                ButtonState::Pressed => p.accent_active,
                ButtonState::Hovered => p.accent_hover,
                ButtonState::Disabled => p.control.gamma_multiply(0.5),
                _ => p.accent,
            };
            let bottom_fill = darken(top_fill, 0.14);

            painter.rect_filled(draw_rect, rounding, top_fill);
            let mid_y = draw_rect.center().y;
            painter.rect_filled(
                Rect::from_min_max(pos2(draw_rect.min.x, mid_y), draw_rect.max),
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: rounding.sw,
                    se: rounding.se,
                },
                bottom_fill,
            );

            // Top specular highlight
            if !matches!(state, ButtonState::Disabled | ButtonState::Pressed) {
                painter.line_segment(
                    [
                        pos2(draw_rect.left() + radius, draw_rect.top() + 0.5),
                        pos2(draw_rect.right() - radius, draw_rect.top() + 0.5),
                    ],
                    Stroke::new(1.0, Color32::from_white_alpha(64)),
                );
            }

            let border_color = match state {
                ButtonState::Disabled => p.border.gamma_multiply(0.5),
                ButtonState::Hovered => p.accent_hover,
                _ => Color32::from_black_alpha(70),
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

            let text_color = if matches!(state, ButtonState::Disabled) {
                p.text_muted
            } else {
                p.text_primary
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

fn darken(c: Color32, amount: f32) -> Color32 {
    let f = 1.0 - amount.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
        c.a(),
    )
}

#[cfg(test)]
mod tests {
    use super::darken;
    use egui::Color32;

    #[test]
    fn darken_clamps_to_black() {
        assert_eq!(
            darken(Color32::WHITE, 1.0),
            Color32::from_rgba_unmultiplied(0, 0, 0, 255)
        );
        let c = Color32::from_rgb(100, 150, 200);
        assert_eq!(darken(c, 0.0), c);
    }
}
