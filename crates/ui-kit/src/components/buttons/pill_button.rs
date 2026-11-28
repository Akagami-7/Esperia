use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};

use crate::{
    Theme,
    primitives::{Shadow, shadow},
};

use super::button_state::ButtonState;

/// A pill-shaped latch container with two side-by-side icon slots
/// (designed for Count-in & Metronome in the Esperia transport bar).
pub struct PillButton<'a, FA, FB>
where
    FA: FnOnce(&mut Ui, Rect, Color32),
    FB: FnOnce(&mut Ui, Rect, Color32),
{
    icon_a: FA,
    icon_b: FB,
    theme: Option<&'a Theme>,
    active_a: bool,
    active_b: bool,
    slot_width: f32,
    height: f32,
    gap: f32,
}

impl<'a, FA, FB> PillButton<'a, FA, FB>
where
    FA: FnOnce(&mut Ui, Rect, Color32),
    FB: FnOnce(&mut Ui, Rect, Color32),
{
    pub fn new(icon_a: FA, icon_b: FB) -> Self {
        Self {
            icon_a,
            icon_b,
            theme: None,
            active_a: false,
            active_b: false,
            slot_width: 46.0,
            height: 40.0,
            gap: 2.0,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn active_a(mut self, yes: bool) -> Self {
        self.active_a = yes;
        self
    }

    pub fn active_b(mut self, yes: bool) -> Self {
        self.active_b = yes;
        self
    }

    pub fn slot_width(mut self, w: f32) -> Self {
        self.slot_width = w;
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
}

/// Result returned by [`PillButton::show`].
pub struct PillButtonResponse {
    pub clicked_a: bool,
    pub clicked_b: bool,
}

impl<FA, FB> PillButton<'_, FA, FB>
where
    FA: FnOnce(&mut Ui, Rect, Color32),
    FB: FnOnce(&mut Ui, Rect, Color32),
{
    pub fn show(self, ui: &mut Ui) -> PillButtonResponse {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;

        let total_width = self.slot_width * 2.0 + self.gap;
        let desired_size = Vec2::new(total_width, self.height);

        let (container_rect, container_response) =
            ui.allocate_exact_size(desired_size, Sense::hover());

        let rect_a =
            Rect::from_min_size(container_rect.min, Vec2::new(self.slot_width, self.height));
        let rect_b = Rect::from_min_size(
            pos2(
                container_rect.min.x + self.slot_width + self.gap,
                container_rect.min.y,
            ),
            Vec2::new(self.slot_width, self.height),
        );

        let resp_a = ui.interact(rect_a, ui.scope_id().with("pill_a"), Sense::click());
        let resp_b = ui.interact(rect_b, ui.scope_id().with("pill_b"), Sense::click());

        let any_hovered = container_response.hovered() || resp_a.hovered() || resp_b.hovered();

        if ui.is_rect_visible(container_rect) {
            if any_hovered {
                ui.painter().rect_filled(
                    container_rect,
                    CornerRadius::same(14),
                    Color32::from_rgba_unmultiplied(128, 128, 128, 36),
                );
            }

            let state_a = ButtonState::from_response(&resp_a, self.active_a, false);
            render_slot(ui, rect_a, theme, state_a);
            let col_a = slot_icon_color(p, state_a);
            (self.icon_a)(ui, rect_a, col_a);

            let state_b = ButtonState::from_response(&resp_b, self.active_b, false);
            render_slot(ui, rect_b, theme, state_b);
            let col_b = slot_icon_color(p, state_b);
            (self.icon_b)(ui, rect_b, col_b);
        }

        PillButtonResponse {
            clicked_a: resp_a.clicked(),
            clicked_b: resp_b.clicked(),
        }
    }
}

fn render_slot(ui: &mut Ui, rect: Rect, theme: &Theme, state: ButtonState) {
    let painter = ui.painter();
    let radius = theme.metrics.radii.window;
    let rounding = CornerRadius::same(14);

    let scale = state.press_scale();

    // HTML: translateY(1.5px) scale(0.94)
    let draw_rect = if (scale - 1.0).abs() > f32::EPSILON {
        Rect::from_center_size(rect.center() + Vec2::new(0.0, 1.5), rect.size() * scale)
    } else {
        rect
    };

    let active = matches!(state, ButtonState::Active);
    let pressed = matches!(state, ButtonState::Pressed);
    let hovered = matches!(state, ButtonState::Hovered);

    if active {
        // Purple external shadow.
        shadow(painter, draw_rect, radius, Shadow::control(theme));

        // Active purple background.
        painter.rect_filled(draw_rect, rounding, Color32::from_rgb(115, 37, 155));

        // Top gradient-like highlight.
        let highlight_rect =
            Rect::from_min_max(draw_rect.min, pos2(draw_rect.right(), draw_rect.center().y));

        painter.rect_filled(
            highlight_rect,
            CornerRadius {
                nw: 14,
                ne: 14,
                sw: 0,
                se: 0,
            },
            Color32::from_rgba_unmultiplied(255, 255, 255, 18),
        );

        // Inset top highlight.
        painter.line_segment(
            [
                pos2(draw_rect.left() + 9.0, draw_rect.top() + 1.0),
                pos2(draw_rect.right() - 9.0, draw_rect.top() + 1.0),
            ],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 82)),
        );

        // Bottom inset shadow.
        painter.line_segment(
            [
                pos2(draw_rect.left() + 9.0, draw_rect.bottom() - 1.5),
                pos2(draw_rect.right() - 9.0, draw_rect.bottom() - 1.5),
            ],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(32, 7, 46, 85)),
        );
    } else if pressed {
        // Tactile inward press.
        painter.rect_filled(
            draw_rect,
            rounding,
            Color32::from_rgba_unmultiplied(128, 128, 128, 28),
        );

        painter.rect_stroke(
            draw_rect,
            rounding,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 0, 0, 55)),
            StrokeKind::Inside,
        );
    } else if hovered {
        // Translucent individual hover.
        painter.rect_filled(
            draw_rect,
            rounding,
            Color32::from_rgba_unmultiplied(128, 128, 128, 26),
        );
    }
}

fn slot_icon_color(p: &crate::theme::Palette, state: ButtonState) -> Color32 {
    match state {
        ButtonState::Active => Color32::from_rgb(245, 232, 255),

        ButtonState::Pressed => p.text_primary,

        ButtonState::Hovered => Color32::from_rgb(81, 81, 81),

        ButtonState::Disabled => p.text_muted,

        _ => Color32::from_rgb(81, 81, 81),
    }
}
