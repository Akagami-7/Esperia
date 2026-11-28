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
        }
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
