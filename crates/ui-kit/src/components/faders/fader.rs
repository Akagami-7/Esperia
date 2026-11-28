use egui::{CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, pos2};

use crate::{
    Theme,
    primitives::{FocusRing, Shadow, shadow},
};

/// Orientation for [`VolumeFader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaderOrientation {
    Vertical,
    Horizontal,
}

/// Convert dB (`-60.0..=+6.0`) to normalized fader travel (`0.0..=1.0`).
///
/// Uses a musical taper so `-12 dB`..`+6 dB` gets finer resolution than the
/// bottom tail (`-60 dB`..`-24 dB`).
pub fn db_to_normalized(db: f32) -> f32 {
    if db <= -60.0 {
        return 0.0;
    }
    if db >= 6.0 {
        return 1.0;
    }
    let linear = (db + 60.0) / 66.0;
    linear.powf(1.6)
}

/// Convert normalized fader travel (`0.0..=1.0`) back to dB (`-60.0..=+6.0`).
pub fn normalized_to_db(norm: f32) -> f32 {
    let clamped = norm.clamp(0.0, 1.0);
    if clamped <= 0.0 {
        return -60.0;
    }
    clamped.powf(1.0 / 1.6) * 66.0 - 60.0
}

/// Channel-strip volume fader operating in decibels (`-60.0..=+6.0`, unity = `0.0 dB`).
pub struct VolumeFader<'a> {
    db: &'a mut f32,
    orientation: FaderOrientation,
    length: f32,
    thickness: f32,
    theme: Option<&'a Theme>,
    enabled: bool,
}

impl<'a> VolumeFader<'a> {
    pub fn vertical(db: &'a mut f32) -> Self {
        Self {
            db,
            orientation: FaderOrientation::Vertical,
            length: 140.0,
            thickness: 28.0,
            theme: None,
            enabled: true,
        }
    }

    pub fn horizontal(db: &'a mut f32) -> Self {
        Self {
            db,
            orientation: FaderOrientation::Horizontal,
            length: 140.0,
            thickness: 24.0,
            theme: None,
            enabled: true,
        }
    }

    pub fn length(mut self, len: f32) -> Self {
        self.length = len;
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

impl Widget for VolumeFader<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let default_theme = Theme::default();
        let theme = self.theme.unwrap_or(&default_theme);
        let p = &theme.palette;

        let desired = match self.orientation {
            FaderOrientation::Vertical => Vec2::new(self.thickness, self.length),
            FaderOrientation::Horizontal => Vec2::new(self.length, self.thickness),
        };
        let sense = if self.enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, mut response) = ui.allocate_exact_size(desired, sense);

        let cap_len = 18.0;
        let pad = cap_len * 0.5 + 2.0;

        if self.enabled {
            if response.double_clicked() {
                *self.db = 0.0;
                response.mark_changed();
            } else if response.dragged() {
                let norm = db_to_normalized(*self.db);
                let travel = (self.length - pad * 2.0).max(1.0);
                let delta = match self.orientation {
                    FaderOrientation::Vertical => -response.drag_delta().y / travel,
                    FaderOrientation::Horizontal => response.drag_delta().x / travel,
                };
                let fine = if ui.input(|i| i.modifiers.shift) {
                    0.25
                } else {
                    1.0
                };
                let next_db = normalized_to_db(norm + delta * fine);
                if (next_db - *self.db).abs() > 0.01 {
                    *self.db = next_db;
                    response.mark_changed();
                }
            }
        }

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let norm = db_to_normalized(*self.db);
            let unity_norm = db_to_normalized(0.0);

            match self.orientation {
                FaderOrientation::Vertical => {
                    let slot = Rect::from_center_size(
                        rect.center(),
                        Vec2::new(4.0, (self.length - pad * 2.0).max(4.0)),
                    );
                    painter.rect_filled(slot, CornerRadius::same(2), p.canvas);
                    painter.rect_stroke(
                        slot,
                        CornerRadius::same(2),
                        Stroke::new(1.0, p.border),
                        StrokeKind::Inside,
                    );

                    // Unity 0 dB tick
                    let unity_y = egui::lerp(slot.bottom()..=slot.top(), unity_norm);
                    painter.line_segment(
                        [
                            pos2(rect.left() + 3.0, unity_y),
                            pos2(slot.left() - 2.0, unity_y),
                        ],
                        Stroke::new(1.0, p.border_strong),
                    );
                    painter.line_segment(
                        [
                            pos2(slot.right() + 2.0, unity_y),
                            pos2(rect.right() - 3.0, unity_y),
                        ],
                        Stroke::new(1.0, p.border_strong),
                    );

                    let cap_y = egui::lerp(slot.bottom()..=slot.top(), norm);
                    let fill_rect = Rect::from_min_max(pos2(slot.left(), cap_y), slot.max);
                    painter.rect_filled(fill_rect, CornerRadius::same(2), p.accent_active);

                    let cap = Rect::from_center_size(
                        pos2(rect.center().x, cap_y),
                        Vec2::new(self.thickness - 6.0, cap_len),
                    );
                    paint_fader_cap(
                        painter,
                        cap,
                        theme,
                        response.hovered() || response.dragged(),
                        true,
                    );
                }
                FaderOrientation::Horizontal => {
                    let slot = Rect::from_center_size(
                        rect.center(),
                        Vec2::new((self.length - pad * 2.0).max(4.0), 4.0),
                    );
                    painter.rect_filled(slot, CornerRadius::same(2), p.canvas);
                    painter.rect_stroke(
                        slot,
                        CornerRadius::same(2),
                        Stroke::new(1.0, p.border),
                        StrokeKind::Inside,
                    );

                    let cap_x = egui::lerp(slot.left()..=slot.right(), norm);
                    let fill_rect = Rect::from_min_max(slot.min, pos2(cap_x, slot.bottom()));
                    painter.rect_filled(fill_rect, CornerRadius::same(2), p.accent_active);

                    let cap = Rect::from_center_size(
                        pos2(cap_x, rect.center().y),
                        Vec2::new(cap_len, self.thickness - 6.0),
                    );
                    paint_fader_cap(
                        painter,
                        cap,
                        theme,
                        response.hovered() || response.dragged(),
                        false,
                    );
                }
            }

            FocusRing::paint(painter, rect, theme, response.has_focus());
        }

        response
    }
}

fn paint_fader_cap(
    painter: &egui::Painter,
    cap: Rect,
    theme: &Theme,
    highlighted: bool,
    vertical: bool,
) {
    let p = &theme.palette;
    let rounding = CornerRadius::same(3);
    shadow(painter, cap, 3.0, Shadow::control(theme));
    painter.rect_filled(
        cap,
        rounding,
        if highlighted {
            p.control_hover
        } else {
            p.panel_raised
        },
    );
    painter.rect_stroke(
        cap,
        rounding,
        Stroke::new(
            1.0,
            if highlighted {
                p.accent
            } else {
                p.border_strong
            },
        ),
        StrokeKind::Inside,
    );
    // Center hairline indicator
    if vertical {
        painter.line_segment(
            [
                pos2(cap.left() + 3.0, cap.center().y),
                pos2(cap.right() - 3.0, cap.center().y),
            ],
            Stroke::new(1.5, p.text_primary),
        );
    } else {
        painter.line_segment(
            [
                pos2(cap.center().x, cap.top() + 3.0),
                pos2(cap.center().x, cap.bottom() - 3.0),
            ],
            Stroke::new(1.5, p.text_primary),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_roundtrip_preserves_unity_and_bounds() {
        assert!((db_to_normalized(-60.0) - 0.0).abs() < f32::EPSILON);
        assert!((db_to_normalized(6.0) - 1.0).abs() < f32::EPSILON);
        let unity = db_to_normalized(0.0);
        assert!((normalized_to_db(unity) - 0.0).abs() < 0.001);
    }
}
