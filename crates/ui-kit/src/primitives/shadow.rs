use egui::{Color32, CornerRadius, Painter, Rect, Shape, vec2};

/// Draws a restrained two-layer ambient shadow beneath a surface.
#[derive(Clone, Copy, Debug)]
pub struct Shadow {
    pub color: Color32,
    pub offset_y: f32,
    pub spread: f32,
}

impl Shadow {
    /// Level 1: a hairline lift for elevated panels. Matte workspace surfaces stay flat.
    pub fn panel(theme: &crate::Theme) -> Self {
        Self {
            color: Color32::from_black_alpha(theme.metrics.shadows.panel_alpha),
            offset_y: 1.0,
            spread: theme.metrics.shadows.blur_radius * 0.15,
        }
    }

    pub fn control(theme: &crate::Theme) -> Self {
        Self {
            color: Color32::from_black_alpha(theme.metrics.shadows.control_alpha),
            offset_y: theme.metrics.shadows.control_offset_y,
            spread: theme.metrics.shadows.blur_radius * 0.25,
        }
    }

    pub fn floating(theme: &crate::Theme) -> Self {
        Self {
            color: Color32::from_black_alpha(theme.metrics.shadows.floating_alpha),
            offset_y: theme.metrics.shadows.floating_offset_y,
            spread: theme.metrics.shadows.blur_radius * 0.75,
        }
    }
}

pub fn shadow(painter: &Painter, rect: Rect, radius: f32, shadow: Shadow) {
    let rounding = CornerRadius::same(radius.clamp(0.0, 255.0) as u8);
    // Concentric, offset fills approximate a soft ambient shadow. Weights are
    // fractions of the token alpha; together they fade from the surface edge.
    let layers = [
        (shadow.spread + 7.0, 0.14),
        (shadow.spread + 4.0, 0.26),
        (shadow.spread + 2.0, 0.44),
        (shadow.spread, 0.70),
    ];
    for (spread, opacity) in layers {
        painter.add(Shape::rect_filled(
            rect.expand(spread)
                .translate(vec2(0.0, shadow.offset_y + 1.0)),
            rounding,
            shadow.color.gamma_multiply(opacity),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;

    #[test]
    fn shadow_spread_follows_the_blur_token() {
        let theme = Theme::default();
        let blur = theme.metrics.shadows.blur_radius;
        assert!((Shadow::panel(&theme).spread - blur * 0.15).abs() < f32::EPSILON);
        assert!((Shadow::control(&theme).spread - blur * 0.25).abs() < f32::EPSILON);
        assert!((Shadow::floating(&theme).spread - blur * 0.75).abs() < f32::EPSILON);
        assert!(Shadow::panel(&theme).color.a() < Shadow::control(&theme).color.a());
        assert!(Shadow::control(&theme).color.a() < Shadow::floating(&theme).color.a());
    }
}
