use eframe::egui;
use esperia_ui::Theme;

use super::component_lab::Section;

/// Component Lab frame: fixed navigation and header with a scrollable content well.
pub(super) fn show(
    ui: &mut egui::Ui,
    theme: &Theme,
    selected: &mut Section,
    content: impl FnOnce(&mut egui::Ui, &Theme, Section),
) {
    let p = &theme.palette;
    egui::Panel::top("component_lab_header")
        .frame(
            egui::Frame::new()
                .fill(p.shell)
                .inner_margin(egui::Margin::symmetric(20, 12)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("ESPERIA")
                        .size(15.0)
                        .strong()
                        .color(p.text_primary),
                );
                ui.add_space(theme.metrics.spacing.md);
                ui.label(
                    egui::RichText::new("UI LABORATORY")
                        .size(10.0)
                        .strong()
                        .color(p.accent),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("DESIGN SYSTEM · FOUNDATION")
                            .size(10.0)
                            .color(p.text_muted),
                    );
                });
            });
        });

    egui::Panel::left("component_lab_navigation")
        .resizable(false)
        .exact_size(204.0)
        .frame(
            egui::Frame::new()
                .fill(p.shell)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("DESIGN SYSTEM")
                    .size(10.0)
                    .strong()
                    .color(p.text_muted),
            );
            ui.add_space(theme.metrics.spacing.sm);
            for (section, title, detail) in [
                (Section::Buttons, "Buttons", "Primary · Secondary · Icon"),
                (Section::Toggles, "Toggles", "Enabled · Disabled"),
                (Section::Surfaces, "Surfaces", "Elevation · borders · focus"),
            ] {
                let response = ui.selectable_label(*selected == section, title);
                if response.clicked() {
                    *selected = section;
                }
                ui.add_space(2.0);
                ui.label(egui::RichText::new(detail).size(10.0).color(p.text_muted));
                ui.add_space(theme.metrics.spacing.sm);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.label(
                    egui::RichText::new("PREVIEW ONLY · NO AUDIO ENGINE")
                        .size(9.0)
                        .color(p.text_muted),
                );
            });
        });

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(p.canvas)
                .inner_margin(egui::Margin::same(24)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| content(ui, theme, *selected));
        });
}
