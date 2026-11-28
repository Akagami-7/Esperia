use eframe::egui;
use esperia_ui::{
    Theme,
    primitives::{Icon, IconKind, Shadow, Surface, SurfaceKind, SurfaceStyle},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Section {
    #[default]
    Buttons,
    Toggles,
    Surfaces,
}

pub struct ComponentLabApp {
    theme: Theme,
    section: Section,
}

impl ComponentLabApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = Theme::default();
        theme.apply(&cc.egui_ctx);
        Self {
            theme,
            section: Section::Buttons,
        }
    }
}

impl eframe::App for ComponentLabApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        super::shell::show(ui, &self.theme, &mut self.section, |ui, theme, section| {
            lab_content::show(ui, theme, section);
        });
    }
}

mod lab_content {
    use super::*;

    pub(super) fn show(ui: &mut egui::Ui, theme: &Theme, selected: Section) {
        let p = &theme.palette;
        ui.label(
            egui::RichText::new("Component Lab")
                .size(theme.typography.workspace_title)
                .strong()
                .color(p.text_primary),
        );
        ui.add_space(theme.metrics.spacing.xs);
        ui.label(
            egui::RichText::new("A shared graphite surface system with selective violet accents.")
                .size(theme.typography.label)
                .color(p.text_secondary),
        );
        ui.add_space(theme.metrics.spacing.xl);
        match selected {
            Section::Buttons => button_placeholders(ui, theme),
            Section::Toggles => toggle_placeholders(ui, theme),
            Section::Surfaces => surface_samples(ui, theme),
        }
    }

    fn section_title(ui: &mut egui::Ui, theme: &Theme, title: &str, note: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(theme.typography.panel_title)
                    .strong()
                    .color(theme.palette.text_primary),
            );
            ui.label(
                egui::RichText::new(note)
                    .size(theme.typography.metadata)
                    .color(theme.palette.text_muted),
            );
        });
        ui.add_space(theme.metrics.spacing.sm);
    }

    fn button_placeholders(ui: &mut egui::Ui, theme: &Theme) {
        section_title(
            ui,
            theme,
            "Button states",
            "Visual placeholders — interaction comes in the next stage",
        );
        ui.horizontal_wrapped(|ui| {
            preview_card(ui, theme, "DEFAULT", Preview::Default);
            preview_card(ui, theme, "HOVER", Preview::Hover);
            preview_card(ui, theme, "PRESSED", Preview::Pressed);
            preview_card(ui, theme, "SELECTED", Preview::Selected);
            preview_card(ui, theme, "DISABLED", Preview::Disabled);
            preview_card(ui, theme, "FOCUSED", Preview::Focused);
        });
        ui.add_space(theme.metrics.spacing.xl);
        section_title(
            ui,
            theme,
            "Surface levels",
            "Matte workspace · satin controls · floating overlays",
        );
        ui.horizontal_wrapped(|ui| {
            surface_card(ui, theme, "MATTE", SurfaceKind::Matte);
            surface_card(ui, theme, "RAISED", SurfaceKind::Raised);
            surface_card(ui, theme, "FLOATING", SurfaceKind::Floating);
        });
    }

    fn toggle_placeholders(ui: &mut egui::Ui, theme: &Theme) {
        section_title(
            ui,
            theme,
            "Toggle states",
            "Static previews · no control behavior is implied",
        );
        ui.horizontal_wrapped(|ui| {
            toggle_card(ui, theme, "SNAP · OFF", TogglePreview::Off);
            toggle_card(ui, theme, "SNAP · ON", TogglePreview::On);
            toggle_card(ui, theme, "SNAP · DISABLED", TogglePreview::Disabled);
        });
    }

    fn surface_samples(ui: &mut egui::Ui, theme: &Theme) {
        section_title(
            ui,
            theme,
            "Shared surfaces",
            "Single token source for fill, border, highlight and elevation",
        );
        ui.horizontal_wrapped(|ui| {
            surface_card(ui, theme, "MATTE", SurfaceKind::Matte);
            surface_card(ui, theme, "RAISED", SurfaceKind::Raised);
            surface_card(ui, theme, "FLOATING", SurfaceKind::Floating);
        });
        ui.add_space(theme.metrics.spacing.xl);
        section_title(
            ui,
            theme,
            "Semantic palette",
            "Color is reserved for meaning and focus",
        );
        for (name, color) in [
            ("Accent", theme.palette.accent),
            ("Playback / success", theme.palette.success),
            ("Warning", theme.palette.warning),
            ("Record / danger", theme.palette.danger),
            ("Secondary edit", theme.palette.cyan),
        ] {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(4), color);
                ui.label(egui::RichText::new(name).color(theme.palette.text_secondary));
            });
        }
    }

    #[derive(Clone, Copy)]
    enum Preview {
        Default,
        Hover,
        Pressed,
        Selected,
        Disabled,
        Focused,
    }

    fn preview_card(ui: &mut egui::Ui, theme: &Theme, label: &str, state: Preview) {
        let (outer, _) = ui.allocate_exact_size(egui::vec2(154.0, 104.0), egui::Sense::hover());
        Surface::paint(
            ui.painter(),
            outer,
            SurfaceStyle::from_theme(theme, SurfaceKind::Matte),
        );
        ui.painter().text(
            egui::pos2(outer.left() + 12.0, outer.top() + 13.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(9.0),
            theme.palette.text_muted,
        );

        let p = &theme.palette;
        let button = egui::Rect::from_min_size(
            egui::pos2(outer.left() + 12.0, outer.top() + 36.0),
            egui::vec2(130.0, 34.0),
        );
        let (fill, border, text) = match state {
            Preview::Default => (p.control, p.border, p.text_primary),
            Preview::Hover => (p.control_hover, p.border_strong, p.text_primary),
            Preview::Pressed => (p.control_pressed, p.accent, p.text_primary),
            Preview::Selected => (p.accent_active, p.accent_hover, p.text_primary),
            Preview::Disabled => (p.panel_raised, p.border, p.text_muted),
            Preview::Focused => (p.control, p.border_strong, p.text_primary),
        };

        // Buttons are composed from the shared raised surface so highlight,
        // border and shadow stay identical to every other control.
        let interactive = !matches!(state, Preview::Pressed | Preview::Disabled);
        Surface::paint(
            ui.painter(),
            button,
            SurfaceStyle {
                fill,
                radius: theme.metrics.radii.button,
                border,
                highlight: interactive.then_some(egui::Color32::from_white_alpha(18)),
                inner_shade: (!matches!(state, Preview::Disabled))
                    .then_some(egui::Color32::from_black_alpha(30)),
                shadow: interactive.then(|| Shadow::control(theme)),
            },
        );
        if matches!(state, Preview::Pressed) {
            // Recessed top edge reads as a pushed control, not a latched one.
            ui.painter().line_segment(
                [
                    egui::pos2(button.left() + 6.0, button.top() + 1.5),
                    egui::pos2(button.right() - 6.0, button.top() + 1.5),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_black_alpha(70)),
            );
        }
        if matches!(state, Preview::Focused) {
            esperia_ui::primitives::FocusRing::paint(ui.painter(), button, theme, true);
        }

        if !matches!(state, Preview::Disabled) {
            Icon::paint(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(button.left() + 18.0, button.center().y),
                    egui::vec2(15.0, 15.0),
                ),
                IconKind::Play,
                text,
            );
        }
        ui.painter().text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            "Play",
            egui::FontId::proportional(theme.typography.control_label),
            text,
        );
    }

    #[derive(Clone, Copy)]
    enum TogglePreview {
        Off,
        On,
        Disabled,
    }

    fn toggle_card(ui: &mut egui::Ui, theme: &Theme, label: &str, state: TogglePreview) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(188.0, 80.0), egui::Sense::hover());
        Surface::paint(
            ui.painter(),
            rect,
            SurfaceStyle::from_theme(theme, SurfaceKind::Matte),
        );
        ui.painter().text(
            egui::pos2(rect.left() + 12.0, rect.top() + 16.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(9.0),
            theme.palette.text_muted,
        );

        let (track_fill, knob_fill, knob_on) = match state {
            TogglePreview::Off => (theme.palette.control, theme.palette.text_secondary, false),
            TogglePreview::On => (
                theme.palette.accent_active,
                theme.palette.accent_hover,
                true,
            ),
            TogglePreview::Disabled => {
                (theme.palette.panel_raised, theme.palette.text_muted, false)
            }
        };
        let track = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 12.0, rect.top() + 34.0),
            egui::vec2(48.0, 26.0),
        );
        let radius = egui::CornerRadius::same(13);
        ui.painter().rect_filled(track, radius, track_fill);
        ui.painter().rect_stroke(
            track,
            radius,
            egui::Stroke::new(1.0, theme.palette.border),
            egui::StrokeKind::Inside,
        );
        // Position, not colour alone, communicates on/off.
        let knob_x = if knob_on {
            track.right() - 13.0
        } else {
            track.left() + 13.0
        };
        ui.painter()
            .circle_filled(egui::pos2(knob_x, track.center().y), 9.0, knob_fill);
    }

    fn surface_card(ui: &mut egui::Ui, theme: &Theme, label: &str, kind: SurfaceKind) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(208.0, 112.0), egui::Sense::hover());
        Surface::paint(ui.painter(), rect, SurfaceStyle::from_theme(theme, kind));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.0),
            theme.palette.text_secondary,
        );
    }
}
