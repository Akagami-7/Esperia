mod metrics;
mod palette;
mod typography;

pub use metrics::{AnimationTimings, CornerRadii, ShadowTokens, Spacing, ThemeMetrics};
pub use palette::Palette;
pub use typography::Typography;

/// Application-wide design tokens. Construct once at app startup and share
/// with UI surfaces and controls instead of embedding colors in components.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub palette: Palette,
    pub metrics: ThemeMetrics,
    pub typography: Typography,
}

impl Theme {
    /// Applies the shared color and density tokens to egui once during app setup.
    pub fn apply(&self, ctx: &egui::Context) {
        let p = &self.palette;
        let radius = egui::CornerRadius::same(self.metrics.radii.control as u8);
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = p.panel;
        visuals.window_stroke = egui::Stroke::new(1.0, p.border_strong);
        visuals.panel_fill = p.shell;
        visuals.extreme_bg_color = p.canvas;
        visuals.faint_bg_color = p.panel_raised;
        visuals.selection.bg_fill = p.accent_active;
        visuals.selection.stroke = egui::Stroke::new(1.0, p.accent_hover);
        visuals.hyperlink_color = p.cyan;

        // Non-interactive text is primary; interactive text is secondary until
        // hovered so controls read as actionable without a global colour override.
        visuals.widgets.noninteractive.bg_fill = p.panel;
        visuals.widgets.noninteractive.weak_bg_fill = p.panel;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
        visuals.widgets.noninteractive.fg_stroke.color = p.text_primary;

        visuals.widgets.inactive.bg_fill = p.control;
        visuals.widgets.inactive.weak_bg_fill = p.control;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, p.border);
        visuals.widgets.inactive.fg_stroke.color = p.text_secondary;
        visuals.widgets.inactive.corner_radius = radius;

        visuals.widgets.hovered.bg_fill = p.control_hover;
        visuals.widgets.hovered.weak_bg_fill = p.control_hover;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, p.border_strong);
        visuals.widgets.hovered.fg_stroke.color = p.text_primary;
        visuals.widgets.hovered.corner_radius = radius;

        visuals.widgets.active.bg_fill = p.accent_active;
        visuals.widgets.active.weak_bg_fill = p.accent_active;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, p.accent);
        visuals.widgets.active.fg_stroke.color = p.text_primary;
        visuals.widgets.active.corner_radius = radius;

        visuals.widgets.open.bg_fill = p.panel_raised;
        visuals.widgets.open.weak_bg_fill = p.panel_raised;
        visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, p.border_strong);
        visuals.widgets.open.fg_stroke.color = p.text_primary;
        visuals.widgets.open.corner_radius = radius;

        ctx.set_visuals(visuals);

        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.spacing.item_spacing =
                egui::vec2(self.metrics.spacing.sm, self.metrics.spacing.sm);
            style.spacing.button_padding =
                egui::vec2(self.metrics.spacing.md, self.metrics.spacing.sm);
            style.spacing.interact_size.y = self.metrics.control_height;
            style.text_styles.insert(
                egui::TextStyle::Heading,
                egui::FontId::new(
                    self.typography.workspace_title,
                    egui::FontFamily::Proportional,
                ),
            );
            style.text_styles.insert(
                egui::TextStyle::Body,
                egui::FontId::new(self.typography.label, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(
                    self.typography.control_label,
                    egui::FontFamily::Proportional,
                ),
            );
            style.text_styles.insert(
                egui::TextStyle::Small,
                egui::FontId::new(self.typography.metadata, egui::FontFamily::Proportional),
            );
            style.text_styles.insert(
                egui::TextStyle::Monospace,
                egui::FontId::new(self.typography.numeric, egui::FontFamily::Monospace),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphite_palette_uses_semantic_tokens() {
        let theme = Theme::default();
        assert_eq!(theme.palette.canvas, egui::Color32::from_rgb(17, 20, 26));
        assert_eq!(theme.metrics.spacing.unit, 4.0);
        assert_eq!(theme.metrics.animation.toggle.as_millis(), 160);
    }
}
