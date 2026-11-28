use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Spacing {
    pub unit: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            unit: 4.0,
            xs: 4.0,
            sm: 8.0,
            md: 12.0,
            lg: 16.0,
            xl: 24.0,
            xxl: 32.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CornerRadii {
    pub control: f32,
    pub button: f32,
    pub panel: f32,
    pub popover: f32,
    pub window: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ShadowTokens {
    pub panel_alpha: u8,
    pub control_alpha: u8,
    pub floating_alpha: u8,
    pub control_offset_y: f32,
    pub floating_offset_y: f32,
    pub blur_radius: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct AnimationTimings {
    pub button_hover: Duration,
    pub button_press: Duration,
    pub button_release: Duration,
    pub toggle: Duration,
    pub knob_highlight: Duration,
    pub panel_appearance: Duration,
    pub popover_appearance: Duration,
    pub focus_highlight: Duration,
}

#[derive(Clone, Copy, Debug)]
pub struct ThemeMetrics {
    pub spacing: Spacing,
    pub radii: CornerRadii,
    pub shadows: ShadowTokens,
    pub animation: AnimationTimings,
    pub control_height: f32,
}

impl Default for ThemeMetrics {
    fn default() -> Self {
        Self {
            spacing: Spacing::default(),
            radii: CornerRadii {
                control: 5.0,
                button: 6.0,
                panel: 8.0,
                popover: 11.0,
                window: 12.0,
            },
            shadows: ShadowTokens {
                panel_alpha: 22,
                control_alpha: 34,
                floating_alpha: 48,
                control_offset_y: 1.0,
                floating_offset_y: 5.0,
                blur_radius: 8.0,
            },
            animation: AnimationTimings {
                button_hover: Duration::from_millis(120),
                button_press: Duration::from_millis(60),
                button_release: Duration::from_millis(160),
                toggle: Duration::from_millis(160),
                knob_highlight: Duration::from_millis(120),
                panel_appearance: Duration::from_millis(180),
                popover_appearance: Duration::from_millis(160),
                focus_highlight: Duration::from_millis(100),
            },
            control_height: 32.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_is_a_consistent_four_point_scale() {
        let s = Spacing::default();
        assert_eq!(
            [s.xs, s.sm, s.md, s.lg, s.xl, s.xxl],
            [4.0, 8.0, 12.0, 16.0, 24.0, 32.0]
        );
    }

    #[test]
    fn radii_stay_within_desktop_guidelines() {
        let r = ThemeMetrics::default().radii;
        assert!((4.0..=6.0).contains(&r.control));
        assert!((6.0..=8.0).contains(&r.button));
        assert_eq!(r.panel, 8.0);
        assert!((10.0..=12.0).contains(&r.popover));
        assert_eq!(r.window, 12.0);
    }

    #[test]
    fn animation_timings_match_the_spec() {
        let a = ThemeMetrics::default().animation;
        assert_eq!(a.button_hover.as_millis(), 120);
        assert_eq!(a.button_press.as_millis(), 60);
        assert_eq!(a.button_release.as_millis(), 160);
        assert_eq!(a.toggle.as_millis(), 160);
        assert_eq!(a.focus_highlight.as_millis(), 100);
    }
}
