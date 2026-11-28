use egui::{Color32, CornerRadius, Painter, Rect, Shape, Stroke, StrokeKind, pos2};

use super::{Shadow, shadow};
use crate::Theme;

#[derive(Clone, Copy, Debug)]
pub enum SurfaceKind {
    Matte,
    Raised,
    Floating,
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceStyle {
    pub fill: Color32,
    pub radius: f32,
    pub border: Color32,
    pub highlight: Option<Color32>,
    pub inner_shade: Option<Color32>,
    pub shadow: Option<Shadow>,
}

impl SurfaceStyle {
    pub fn from_theme(theme: &Theme, kind: SurfaceKind) -> Self {
        let p = &theme.palette;
        match kind {
            SurfaceKind::Matte => Self {
                fill: p.panel,
                radius: theme.metrics.radii.panel,
                border: p.border,
                highlight: None,
                inner_shade: None,
                shadow: None,
            },
            SurfaceKind::Raised => Self {
                fill: p.panel_raised,
                radius: theme.metrics.radii.panel,
                border: p.border,
                highlight: Some(Color32::from_white_alpha(12)),
                inner_shade: Some(Color32::from_black_alpha(28)),
                shadow: Some(Shadow::control(theme)),
            },
            SurfaceKind::Floating => Self {
                fill: p.glass,
                radius: theme.metrics.radii.popover,
                border: p.border_strong,
                highlight: Some(Color32::from_white_alpha(18)),
                inner_shade: Some(Color32::from_black_alpha(34)),
                shadow: Some(Shadow::floating(theme)),
            },
        }
    }
}

pub struct Surface;

impl Surface {
    pub fn paint(painter: &Painter, rect: Rect, style: SurfaceStyle) {
        if let Some(s) = style.shadow {
            shadow(painter, rect, style.radius, s);
        }
        let rounding = CornerRadius::same(style.radius.clamp(0.0, 255.0) as u8);
        painter.add(Shape::rect_filled(rect, rounding, style.fill));
        painter.rect_stroke(
            rect,
            rounding,
            Stroke::new(1.0, style.border),
            StrokeKind::Inside,
        );
        if let Some(highlight) = style.highlight {
            painter.line_segment(
                [
                    pos2(rect.left() + style.radius, rect.top() + 0.5),
                    pos2(rect.right() - style.radius, rect.top() + 0.5),
                ],
                Stroke::new(1.0, highlight),
            );
        }
        if let Some(shade) = style.inner_shade {
            painter.line_segment(
                [
                    pos2(rect.left() + style.radius, rect.bottom() - 0.5),
                    pos2(rect.right() - style.radius, rect.bottom() - 0.5),
                ],
                Stroke::new(1.0, shade),
            );
        }
    }
}

pub struct RaisedSurface;

impl RaisedSurface {
    pub fn paint(painter: &Painter, rect: Rect, theme: &Theme) {
        Surface::paint(
            painter,
            rect,
            SurfaceStyle::from_theme(theme, SurfaceKind::Raised),
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Border {
    pub color: Color32,
    pub width: f32,
    pub radius: f32,
}

impl Border {
    pub fn paint(self, painter: &Painter, rect: Rect) {
        painter.rect_stroke(
            rect,
            CornerRadius::same(self.radius.clamp(0.0, 255.0) as u8),
            Stroke::new(self.width, self.color),
            StrokeKind::Inside,
        );
    }
}

pub struct FocusRing;

impl FocusRing {
    pub fn paint(painter: &Painter, rect: Rect, theme: &Theme, focused: bool) {
        if focused {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(theme.metrics.radii.control as u8),
                Stroke::new(1.5, theme.palette.accent),
                StrokeKind::Outside,
            );
        }
    }
}

pub struct Divider;

impl Divider {
    pub fn horizontal(painter: &Painter, x: std::ops::RangeInclusive<f32>, y: f32, theme: &Theme) {
        painter.hline(x, y, Stroke::new(1.0, theme.palette.border));
    }
    pub fn vertical(painter: &Painter, x: f32, y: std::ops::RangeInclusive<f32>, theme: &Theme) {
        painter.vline(x, y, Stroke::new(1.0, theme.palette.border));
    }
}

#[derive(Clone, Copy, Debug)]
pub enum IconKind {
    Play,
    Stop,
    Record,
    ChevronRight,
    CountIn,
    Metronome,
}

/// Small resolution-independent geometric icons used where no project SVG is selected.
pub struct Icon;

impl Icon {
    pub fn paint(painter: &Painter, rect: Rect, kind: IconKind, color: Color32) {
        let c = rect.center();
        let r = rect.width().min(rect.height()) * 0.34;
        match kind {
            IconKind::Play => {
                painter.add(Shape::convex_polygon(
                    vec![
                        pos2(c.x - r * 0.65, c.y - r),
                        pos2(c.x - r * 0.65, c.y + r),
                        pos2(c.x + r, c.y),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            IconKind::Stop => {
                painter.rect_filled(
                    Rect::from_center_size(c, egui::vec2(r * 1.55, r * 1.55)),
                    CornerRadius::same(1),
                    color,
                );
            }
            IconKind::Record => {
                painter.circle_filled(c, r, color);
            }
            IconKind::ChevronRight => {
                let top = pos2(c.x - r * 0.5, c.y - r);
                let tip = pos2(c.x + r * 0.5, c.y);
                let bottom = pos2(c.x - r * 0.5, c.y + r);
                let stroke = Stroke::new(1.5, color);
                painter.line_segment([top, tip], stroke);
                painter.line_segment([tip, bottom], stroke);
            }
            IconKind::CountIn => {
                paint_count_in(painter, rect, color);
            }
            IconKind::Metronome => {
                paint_metronome(painter, rect, color);
            }
        }
    }
}

/// Map a point from the 46x40 SVG slot space into `rect` preserving aspect ratio.
fn map_slot_pt(rect: Rect, sx: f32, sy: f32) -> egui::Pos2 {
    let scale = (rect.width() / 46.0).min(rect.height() / 40.0);
    let ox = rect.center().x - 23.0 * scale;
    let oy = rect.center().y - 20.0 * scale;
    pos2(ox + sx * scale, oy + sy * scale)
}

fn slot_scale(rect: Rect) -> f32 {
    (rect.width() / 46.0).min(rect.height() / 40.0)
}

fn sample_cubic(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    steps: usize,
) -> Vec<[f32; 2]> {
    (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let u = 1.0 - t;
            let x = u * u * u * p0[0]
                + 3.0 * u * u * t * p1[0]
                + 3.0 * u * t * t * p2[0]
                + t * t * t * p3[0];
            let y = u * u * u * p0[1]
                + 3.0 * u * u * t * p1[1]
                + 3.0 * u * t * t * p2[1]
                + t * t * t * p3[1];
            [x, y]
        })
        .collect()
}

fn paint_dashed_cubic(painter: &Painter, rect: Rect, pts: &[[f32; 2]], stroke: Stroke) {
    // Draw alternating short segments to reproduce stroke-dasharray="2 2.5".
    for seg in pts.chunks(3) {
        if seg.len() >= 2 {
            painter.line_segment(
                [
                    map_slot_pt(rect, seg[0][0], seg[0][1]),
                    map_slot_pt(rect, seg[1][0], seg[1][1]),
                ],
                stroke,
            );
        }
    }
}

fn paint_count_in(painter: &Painter, rect: Rect, color: Color32) {
    let s = slot_scale(rect);

    // Dashed rising arc from Count-in_Metronome.svg (left slot coords: x-4, y-4)
    let arc_pts = sample_cubic([11.5, 19.5], [15.5, 13.5], [23.5, 10.0], [32.5, 9.5], 14);
    paint_dashed_cubic(
        painter,
        rect,
        &arc_pts,
        Stroke::new(1.6 * s, color.gamma_multiply(0.45)),
    );

    // 4-Beat Rising Pulse bars
    let bars = [
        (10.5_f32, 23.0_f32, 3.8_f32, 6.0_f32, 0.50_f32),
        (17.5, 19.5, 3.8, 9.5, 0.70),
        (24.5, 16.0, 3.8, 13.0, 0.88),
        (31.5, 12.0, 4.2, 17.0, 1.00),
    ];
    for (bx, by, bw, bh, alpha) in bars {
        let min = map_slot_pt(rect, bx, by);
        let max = map_slot_pt(rect, bx + bw, by + bh);
        let rx = ((bw * 0.5) * s).clamp(1.0, 255.0) as u8;
        painter.rect_filled(
            Rect::from_min_max(min, max),
            CornerRadius::same(rx),
            color.gamma_multiply(alpha),
        );
    }
}

fn paint_metronome(painter: &Painter, rect: Rect, color: Color32) {
    let s = slot_scale(rect);

    // Top click-spark dot (right slot coords: x-50, y-4)
    painter.circle_filled(map_slot_pt(rect, 23.0, 10.2), 2.1 * s, color);

    // Central solid beat pillar
    let p_min = map_slot_pt(rect, 20.9, 14.2);
    let p_max = map_slot_pt(rect, 25.1, 29.0);
    let rx = (2.1 * s).clamp(1.0, 255.0) as u8;
    painter.rect_filled(
        Rect::from_min_max(p_min, p_max),
        CornerRadius::same(rx),
        color,
    );

    // Inner acoustic click waves (left & right)
    let wave_stroke = Stroke::new(2.1 * s, color.gamma_multiply(0.85));
    for pts in [
        sample_cubic([16.2, 17.1], [14.4, 19.5], [14.4, 23.7], [16.2, 26.1], 10),
        sample_cubic([29.8, 17.1], [31.6, 19.5], [31.6, 23.7], [29.8, 26.1], 10),
    ] {
        let mapped: Vec<_> = pts.iter().map(|p| map_slot_pt(rect, p[0], p[1])).collect();
        painter.add(Shape::line(mapped, wave_stroke));
    }

    // Outer dashed echo ripples (left & right)
    let echo_stroke = Stroke::new(1.7 * s, color.gamma_multiply(0.45));
    for pts in [
        sample_cubic([12.0, 14.6], [9.4, 18.4], [9.4, 24.8], [12.0, 28.6], 12),
        sample_cubic([34.0, 14.6], [36.6, 18.4], [36.6, 24.8], [34.0, 28.6], 12),
    ] {
        paint_dashed_cubic(painter, rect, &pts, echo_stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_kinds_use_expected_material_tokens() {
        let theme = Theme::default();
        let matte = SurfaceStyle::from_theme(&theme, SurfaceKind::Matte);
        let raised = SurfaceStyle::from_theme(&theme, SurfaceKind::Raised);
        let floating = SurfaceStyle::from_theme(&theme, SurfaceKind::Floating);

        assert_eq!(matte.fill, theme.palette.panel);
        assert!(matte.shadow.is_none());
        assert!(matte.highlight.is_none());

        assert_eq!(raised.fill, theme.palette.panel_raised);
        assert!(raised.shadow.is_some());
        assert!(raised.inner_shade.is_some());

        assert_eq!(floating.fill, theme.palette.glass);
        assert_eq!(floating.radius, theme.metrics.radii.popover);
        assert!(floating.shadow.is_some());
    }
}
