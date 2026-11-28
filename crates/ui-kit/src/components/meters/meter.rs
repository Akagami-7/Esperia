use egui::{CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2};

use crate::Theme;

/// Stateful ballistic tracker for peak + RMS audio meters (fast attack, peak hold, smooth decay).
#[derive(Debug, Clone, Copy)]
pub struct MeterState {
    pub level: f32,
    pub peak_hold: f32,
    hold_frames: u32,
}

impl Default for MeterState {
    fn default() -> Self {
        Self {
            level: 0.0,
            peak_hold: 0.0,
            hold_frames: 0,
        }
    }
}

impl MeterState {
    /// Update meter ballistics with a normalized linear sample (`0.0..=1.2`).
    pub fn update(&mut self, target: f32, decay_per_frame: f32) {
        let clamped = target.clamp(0.0, 1.2);
        if clamped >= self.level {
            self.level = clamped;
        } else {
            self.level = (self.level - decay_per_frame).max(clamped);
        }

        if clamped >= self.peak_hold {
            self.peak_hold = clamped;
            self.hold_frames = 45;
        } else if self.hold_frames > 0 {
            self.hold_frames -= 1;
        } else {
            self.peak_hold = (self.peak_hold - decay_per_frame * 0.7).max(self.level);
        }
    }
}

/// Vertical peak meter with green/amber/red zones and peak-hold cap.
pub struct PeakMeter<'a> {
    state: &'a MeterState,
    size: Vec2,
    theme: Option<&'a Theme>,
}

impl<'a> PeakMeter<'a> {
    pub fn new(state: &'a MeterState) -> Self {
        Self {
            state,
            size: Vec2::new(10.0, 120.0),
            theme: None,
        }
    }

    pub fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }
}

impl Widget for PeakMeter<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::hover());

        if ui.is_rect_visible(rect) {
            paint_vertical_meter(
                ui.painter(),
                rect,
                self.state.level,
                Some(self.state.peak_hold),
                theme,
            );
        }

        response
    }
}

/// Vertical RMS meter without peak-hold cap.
pub struct RmsMeter<'a> {
    level: f32,
    size: Vec2,
    theme: Option<&'a Theme>,
}

impl<'a> RmsMeter<'a> {
    pub fn new(level: f32) -> Self {
        Self {
            level: level.clamp(0.0, 1.0),
            size: Vec2::new(8.0, 120.0),
            theme: None,
        }
    }

    pub fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }
}

impl Widget for RmsMeter<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::hover());

        if ui.is_rect_visible(rect) {
            paint_vertical_meter(ui.painter(), rect, self.level, None, theme);
        }

        response
    }
}

fn paint_vertical_meter(
    painter: &egui::Painter,
    rect: Rect,
    level: f32,
    peak_hold: Option<f32>,
    theme: &Theme,
) {
    let p = &theme.palette;
    let rounding = CornerRadius::same(2);
    painter.rect_filled(rect, rounding, p.canvas);
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );

    let inner = rect.shrink(1.5);
    let norm = level.clamp(0.0, 1.0);
    let fill_h = inner.height() * norm;

    if fill_h > 0.5 {
        // Draw segmented color bands: green (0..0.72), amber (0.72..0.9), red (0.9..1.0)
        let bands = [
            (0.0_f32, 0.72_f32, p.success),
            (0.72_f32, 0.90_f32, p.warning),
            (0.90_f32, 1.0_f32, p.danger),
        ];
        for (low, high, color) in bands {
            if norm > low {
                let top_norm = norm.min(high);
                let y_bottom = inner.bottom() - inner.height() * low;
                let y_top = inner.bottom() - inner.height() * top_norm;
                painter.rect_filled(
                    Rect::from_min_max(pos2(inner.left(), y_top), pos2(inner.right(), y_bottom)),
                    CornerRadius::ZERO,
                    color,
                );
            }
        }
    }

    if let Some(hold) = peak_hold {
        let hold_norm = hold.clamp(0.0, 1.0);
        if hold_norm > 0.01 {
            let y = inner.bottom() - inner.height() * hold_norm;
            let hold_color = if hold > 0.95 {
                p.danger
            } else {
                p.text_primary
            };
            painter.line_segment(
                [pos2(inner.left(), y), pos2(inner.right(), y)],
                Stroke::new(1.5, hold_color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MeterState;

    #[test]
    fn peak_hold_tracks_attack_and_decays_after_hold_window() {
        let mut state = MeterState::default();
        state.update(0.8, 0.05);
        assert!((state.level - 0.8).abs() < f32::EPSILON);
        assert!((state.peak_hold - 0.8).abs() < f32::EPSILON);

        for _ in 0..45 {
            state.update(0.0, 0.05);
        }
        assert!((state.peak_hold - 0.8).abs() < f32::EPSILON);

        state.update(0.0, 0.05);
        assert!(state.peak_hold < 0.8);
    }
}
