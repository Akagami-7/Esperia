use std::f32::consts::{FRAC_PI_4, PI};

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, Widget, pos2};

use crate::{Theme, primitives::FocusRing};

/// Behaviour mode for [`RotaryKnob`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobMode {
    /// Unipolar 0.0..=1.0 sweep from 7 o'clock to 5 o'clock.
    Standard,
    /// Bipolar -1.0..=1.0 sweep with center detent at 12 o'clock (Pan / Balance).
    Bipolar,
    /// Quantized to `steps` discrete detents across 0.0..=1.0.
    Stepped(usize),
}

/// Precision rotary knob for DAW parameters (gain, pan, filter cutoff, send level).
pub struct RotaryKnob<'a> {
    value: &'a mut f32,
    default_value: f32,
    mode: KnobMode,
    diameter: f32,
    label: Option<String>,
    theme: Option<&'a Theme>,
    enabled: bool,
}

impl<'a> RotaryKnob<'a> {
    pub fn new(value: &'a mut f32) -> Self {
        Self {
            value,
            default_value: 0.5,
            mode: KnobMode::Standard,
            diameter: 40.0,
            label: None,
            theme: None,
            enabled: true,
        }
    }

    pub fn mode(mut self, mode: KnobMode) -> Self {
        self.mode = mode;
        if matches!(mode, KnobMode::Bipolar) {
            self.default_value = 0.0;
        }
        self
    }

    pub fn default_value(mut self, v: f32) -> Self {
        self.default_value = v;
        self
    }

    pub fn diameter(mut self, d: f32) -> Self {
        self.diameter = d;
        self
    }

    pub fn mini(mut self) -> Self {
        self.diameter = 28.0;
        self
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

/// Normalize value to 0.0..=1.0 according to knob mode.
pub fn normalize_value(value: f32, mode: KnobMode) -> f32 {
    match mode {
        KnobMode::Bipolar => ((value.clamp(-1.0, 1.0)) + 1.0) * 0.5,
        KnobMode::Standard | KnobMode::Stepped(_) => value.clamp(0.0, 1.0),
    }
}

/// Apply drag delta and quantization according to knob mode.
pub fn apply_drag(value: f32, delta_norm: f32, mode: KnobMode) -> f32 {
    let norm = (normalize_value(value, mode) + delta_norm).clamp(0.0, 1.0);
    match mode {
        KnobMode::Bipolar => (norm * 2.0 - 1.0).clamp(-1.0, 1.0),
        KnobMode::Standard => norm,
        KnobMode::Stepped(steps) => {
            let max_step = steps.saturating_sub(1).max(1) as f32;
            (norm * max_step).round() / max_step
        }
    }
}

impl Widget for RotaryKnob<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;
        let m = &theme.metrics;

        let font = egui::FontId::proportional(theme.typography.metadata);
        let label_h = if self.label.is_some() { 16.0 } else { 0.0 };
        let desired = Vec2::new(self.diameter.max(40.0), self.diameter + label_h);

        let sense = if self.enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, mut response) = ui.allocate_exact_size(desired, sense);

        if self.enabled {
            if response.double_clicked() {
                *self.value = self.default_value;
                response.mark_changed();
            } else if response.dragged() {
                let sensitivity = if ui.input(|i| i.modifiers.shift) {
                    1.0 / 500.0
                } else {
                    1.0 / 160.0
                };
                let delta = -response.drag_delta().y * sensitivity;
                let next = apply_drag(*self.value, delta, self.mode);
                if (next - *self.value).abs() > f32::EPSILON {
                    *self.value = next;
                    response.mark_changed();
                }
            }
        }

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let knob_center = pos2(rect.center().x, rect.min.y + self.diameter * 0.5);
            let outer_r = self.diameter * 0.5 - 3.0;
            let arc_r = outer_r + 2.5;

            // Sweep: 135 deg (bottom-left) -> 405 deg (bottom-right), 270 deg total.
            let start_angle = PI * 0.75;
            let sweep = PI * 1.5;
            let norm = normalize_value(*self.value, self.mode);

            // Background track arc
            paint_arc(
                painter,
                knob_center,
                arc_r,
                start_angle,
                start_angle + sweep,
                Stroke::new(2.5, p.control),
            );

            // Active value arc
            let active_color = if !self.enabled {
                p.text_muted
            } else if response.dragged() || response.hovered() {
                p.accent_hover
            } else {
                p.accent
            };

            match self.mode {
                KnobMode::Bipolar => {
                    let mid_angle = start_angle + sweep * 0.5;
                    let val_angle = start_angle + sweep * norm;
                    let (a0, a1) = if val_angle >= mid_angle {
                        (mid_angle, val_angle)
                    } else {
                        (val_angle, mid_angle)
                    };
                    if (a1 - a0).abs() > 0.01 {
                        paint_arc(
                            painter,
                            knob_center,
                            arc_r,
                            a0,
                            a1,
                            Stroke::new(2.5, active_color),
                        );
                    }
                }
                KnobMode::Standard | KnobMode::Stepped(_) => {
                    if norm > 0.005 {
                        paint_arc(
                            painter,
                            knob_center,
                            arc_r,
                            start_angle,
                            start_angle + sweep * norm,
                            Stroke::new(2.5, active_color),
                        );
                    }
                }
            }

            // Housing body + specular rim
            let body_fill = if response.dragged() {
                p.panel_raised
            } else if response.hovered() {
                p.control_hover
            } else {
                p.panel_raised
            };
            painter.circle_filled(knob_center, outer_r, body_fill);
            painter.circle_stroke(knob_center, outer_r, Stroke::new(1.0, p.border_strong));

            // Subtle upper-left specular arc
            paint_arc(
                painter,
                knob_center,
                outer_r - 1.5,
                PI + FRAC_PI_4 * 0.5,
                PI * 1.75,
                Stroke::new(1.0, Color32::from_white_alpha(28)),
            );

            // Indicator line from center toward edge
            let angle = start_angle + sweep * norm;
            let dir = Vec2::new(angle.cos(), angle.sin());
            let inner = knob_center + dir * (outer_r * 0.28);
            let outer = knob_center + dir * (outer_r - 2.5);
            let ind_color = if self.enabled {
                p.text_primary
            } else {
                p.text_muted
            };
            painter.line_segment([inner, outer], Stroke::new(2.0, ind_color));

            let knob_rect = Rect::from_center_size(knob_center, Vec2::splat(self.diameter));
            FocusRing::paint(painter, knob_rect, theme, response.has_focus());

            if let Some(ref text) = self.label {
                painter.text(
                    pos2(rect.center().x, rect.bottom() - m.spacing.xs),
                    egui::Align2::CENTER_BOTTOM,
                    text,
                    font,
                    if self.enabled {
                        p.text_secondary
                    } else {
                        p.text_muted
                    },
                );
            }
        }

        response
    }
}

fn paint_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: Stroke,
) {
    let steps = (((end_angle - start_angle).abs() / (PI * 2.0)) * 48.0)
        .ceil()
        .max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let a = egui::lerp(start_angle..=end_angle, t);
            pos2(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    painter.add(Shape::line(pts, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bipolar_normalizes_center_to_half() {
        assert!((normalize_value(0.0, KnobMode::Bipolar) - 0.5).abs() < f32::EPSILON);
        assert!((normalize_value(-1.0, KnobMode::Bipolar) - 0.0).abs() < f32::EPSILON);
        assert!((normalize_value(1.0, KnobMode::Bipolar) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn stepped_knob_quantizes_to_detents() {
        let v = apply_drag(0.0, 0.31, KnobMode::Stepped(5));
        assert!((v - 0.25).abs() < f32::EPSILON);
    }
}
