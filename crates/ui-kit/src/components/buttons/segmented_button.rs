use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};

use crate::Theme;

use super::button_state::ButtonState;

/// A horizontal row of mutually-exclusive labeled segments.
pub struct SegmentedButton<'a> {
    segments: &'a [&'a str],
    selected: usize,
    theme: Option<&'a Theme>,
    height: f32,
    min_segment_width: f32,
}

impl<'a> SegmentedButton<'a> {
    pub fn new(segments: &'a [&'a str], selected: usize) -> Self {
        Self {
            segments,
            selected,
            theme: None,
            height: 24.0,
            min_segment_width: 48.0,
        }
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    pub fn height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }

    pub fn show(self, ui: &mut Ui) -> usize {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let font = egui::FontId::proportional(theme.typography.control_label);
        let padding = m.spacing.md;
        let seg_widths: Vec<f32> = self
            .segments
            .iter()
            .map(|s| {
                let g = ui
                    .painter()
                    .layout_no_wrap((*s).to_string(), font.clone(), Color32::WHITE);
                (g.size().x + padding * 2.0).max(self.min_segment_width)
            })
            .collect();

        let total_width: f32 = seg_widths.iter().sum();
        let desired = Vec2::new(total_width, self.height);
        let (container_rect, _) = ui.allocate_exact_size(desired, Sense::hover());

        let r = m.radii.control as u8;
        let outer_rounding = CornerRadius::same(r);
        ui.painter()
            .rect_filled(container_rect, outer_rounding, p.shell);
        ui.painter().rect_stroke(
            container_rect,
            outer_rounding,
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );

        let mut new_selection = self.selected;
        let mut x = container_rect.min.x;

        for (i, (label, seg_w)) in self.segments.iter().zip(seg_widths.iter()).enumerate() {
            let seg_rect = Rect::from_min_size(
                pos2(x, container_rect.min.y),
                Vec2::new(*seg_w, self.height),
            );
            x += seg_w;

            let resp = ui.interact(seg_rect, ui.scope_id().with(("seg", i)), Sense::click());
            if resp.clicked() {
                new_selection = i;
            }

            if ui.is_rect_visible(seg_rect) {
                let is_selected = i == new_selection;
                let state = ButtonState::from_response(&resp, is_selected, false);
                render_segment(
                    ui,
                    seg_rect,
                    label,
                    i,
                    self.segments.len(),
                    theme,
                    &font,
                    state,
                );
            }
        }

        new_selection
    }
}

#[allow(clippy::too_many_arguments)]
fn render_segment(
    ui: &mut Ui,
    rect: Rect,
    label: &str,
    index: usize,
    total: usize,
    theme: &Theme,
    font: &egui::FontId,
    state: ButtonState,
) {
    let p = &theme.palette;
    let r = theme.metrics.radii.control as u8;
    let painter = ui.painter();
    let is_first = index == 0;
    let is_last = index + 1 == total;

    let rounding = CornerRadius {
        nw: if is_first { r } else { 0 },
        sw: if is_first { r } else { 0 },
        ne: if is_last { r } else { 0 },
        se: if is_last { r } else { 0 },
    };

    let fill = match state {
        ButtonState::Active => p.accent_active,
        ButtonState::Pressed => p.control_pressed,
        ButtonState::Hovered => p.control_hover,
        _ => Color32::TRANSPARENT,
    };
    painter.rect_filled(rect, rounding, fill);

    if state.is_accent() {
        painter.line_segment(
            [
                pos2(rect.left() + 2.0, rect.top() + 0.5),
                pos2(rect.right() - 2.0, rect.top() + 0.5),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(40)),
        );
    }

    if !is_last {
        painter.line_segment(
            [
                pos2(rect.max.x, rect.min.y + 4.0),
                pos2(rect.max.x, rect.max.y - 4.0),
            ],
            Stroke::new(1.0, p.border),
        );
    }

    let text_color = match state {
        ButtonState::Active | ButtonState::Hovered | ButtonState::Pressed => p.text_primary,
        ButtonState::Disabled => p.text_muted,
        _ => p.text_secondary,
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font.clone(),
        text_color,
    );
}
