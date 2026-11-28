use egui::{
    Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2,
};

use crate::{Theme, primitives::FocusRing};

/// A compact DAW switch toggle with smooth animated thumb travel and optional label.
pub struct Toggle<'a> {
    on: &'a mut bool,
    label: Option<String>,
    theme: Option<&'a Theme>,
    enabled: bool,
}

impl<'a> Toggle<'a> {
    pub fn new(on: &'a mut bool) -> Self {
        Self {
            on,
            label: None,
            theme: None,
            enabled: true,
        }
    }

    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
}

impl Widget for Toggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let track_size = Vec2::new(34.0, 18.0);
        let font = egui::FontId::proportional(theme.typography.control_label);
        let label_galley = self.label.as_ref().map(|l| {
            ui.painter()
                .layout_no_wrap(l.clone(), font.clone(), Color32::WHITE)
        });

        let total_width = if let Some(ref g) = label_galley {
            track_size.x + m.spacing.sm + g.size().x
        } else {
            track_size.x
        };
        let total_height = track_size.y.max(
            label_galley
                .as_ref()
                .map(|g| g.size().y)
                .unwrap_or(track_size.y),
        );

        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(total_width, total_height), sense);

        if self.enabled && response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }

        if ui.is_rect_visible(rect) {
            let track_rect = Rect::from_min_size(
                pos2(rect.min.x, rect.center().y - track_size.y * 0.5),
                track_size,
            );
            let anim_t = ui.ctx().animate_bool_with_time(
                response.id,
                *self.on,
                m.animation.toggle.as_secs_f32(),
            );

            let track_rounding = CornerRadius::same((track_size.y * 0.5) as u8);
            let track_fill = if !self.enabled {
                p.control.gamma_multiply(0.5)
            } else if *self.on {
                p.accent_active
            } else if response.hovered() {
                p.control_hover
            } else {
                p.control
            };
            let track_border = if !self.enabled {
                p.border.gamma_multiply(0.5)
            } else if *self.on {
                p.accent
            } else if response.hovered() {
                p.border_strong
            } else {
                p.border
            };

            let painter = ui.painter();
            painter.rect_filled(track_rect, track_rounding, track_fill);
            painter.rect_stroke(
                track_rect,
                track_rounding,
                Stroke::new(1.0, track_border),
                StrokeKind::Inside,
            );

            let thumb_r = track_size.y * 0.5 - 3.0;
            let left_x = track_rect.left() + 3.0 + thumb_r;
            let right_x = track_rect.right() - 3.0 - thumb_r;
            let thumb_x = egui::lerp(left_x..=right_x, anim_t);
            let thumb_center = pos2(thumb_x, track_rect.center().y);

            let thumb_color = if !self.enabled {
                p.text_muted
            } else if *self.on {
                p.text_primary
            } else {
                p.text_secondary
            };
            painter.circle_filled(thumb_center, thumb_r, thumb_color);

            FocusRing::paint(painter, track_rect, theme, response.has_focus());

            if let Some(g) = label_galley {
                let text_color = if !self.enabled {
                    p.text_muted
                } else if response.hovered() || *self.on {
                    p.text_primary
                } else {
                    p.text_secondary
                };
                painter.galley(
                    pos2(
                        track_rect.right() + m.spacing.sm,
                        rect.center().y - g.size().y * 0.5,
                    ),
                    g,
                    text_color,
                );
            }
        }

        response
    }
}
