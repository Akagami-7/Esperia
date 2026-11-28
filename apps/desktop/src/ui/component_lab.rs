use eframe::egui;
use esperia_ui::{
    Theme,
    components::{
        IconButton, KnobMode, MeterState, PeakMeter, PillButton, PrimaryButton, RmsMeter,
        RotaryKnob, SecondaryButton, SegmentedButton, Toggle, ToggleButton, TransportButton,
        TransportVariant, VolumeFader,
    },
    primitives::{Icon, IconKind, Surface, SurfaceKind, SurfaceStyle},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Section {
    #[default]
    Buttons,
    Toggles,
    KnobsAndFaders,
    Meters,
    Surfaces,
}

pub struct ComponentLabApp {
    theme: Theme,
    section: Section,
    state: LabState,
}

struct LabState {
    count_in: bool,
    metronome: bool,
    playing: bool,
    recording: bool,
    loop_on: bool,
    snap_on: bool,
    auto_quantize: bool,
    segmented_mode: usize,
    gain_knob: f32,
    pan_knob: f32,
    stepped_knob: f32,
    mini_knob: f32,
    fader_db: f32,
    horiz_db: f32,
    meter_drive: f32,
    peak_state: MeterState,
}

impl Default for LabState {
    fn default() -> Self {
        Self {
            count_in: false,
            metronome: true,
            playing: false,
            recording: false,
            loop_on: true,
            snap_on: true,
            auto_quantize: false,
            segmented_mode: 0,
            gain_knob: 0.72,
            pan_knob: 0.0,
            stepped_knob: 0.5,
            mini_knob: 0.35,
            fader_db: 0.0,
            horiz_db: -6.0,
            meter_drive: 0.68,
            peak_state: MeterState::default(),
        }
    }
}

impl ComponentLabApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = Theme::default();
        theme.apply(&cc.egui_ctx);
        Self {
            theme,
            section: Section::Buttons,
            state: LabState::default(),
        }
    }
}

impl eframe::App for ComponentLabApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.state.peak_state.update(self.state.meter_drive, 0.02);
        let section = self.section;
        let lab_state = &mut self.state;
        super::shell::show(ui, &self.theme, &mut self.section, |ui, theme| {
            lab_content::show(ui, theme, section, lab_state);
        });
    }
}

mod lab_content {
    use super::*;

    pub(super) fn show(ui: &mut egui::Ui, theme: &Theme, selected: Section, state: &mut LabState) {
        let p = &theme.palette;
        ui.label(
            egui::RichText::new("Component Lab")
                .size(theme.typography.workspace_title)
                .strong()
                .color(p.text_primary),
        );
        ui.add_space(theme.metrics.spacing.xs);
        ui.label(
            egui::RichText::new(
                "Interactive Esperia UI Kit · graphite surfaces & selective violet accents.",
            )
            .size(theme.typography.label)
            .color(p.text_secondary),
        );
        ui.add_space(theme.metrics.spacing.xl);
        match selected {
            Section::Buttons => buttons_section(ui, theme, state),
            Section::Toggles => toggles_section(ui, theme, state),
            Section::KnobsAndFaders => knobs_and_faders_section(ui, theme, state),
            Section::Meters => meters_section(ui, theme, state),
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

    fn buttons_section(ui: &mut egui::Ui, theme: &Theme, state: &mut LabState) {
        section_title(
            ui,
            theme,
            "Standard buttons",
            "Primary, secondary, and disabled states",
        );
        ui.horizontal_wrapped(|ui| {
            let _ = ui.add(PrimaryButton::new("Export Mix").theme(theme));
            let _ = ui.add(SecondaryButton::new("Cancel").theme(theme));
            let _ = ui.add(PrimaryButton::new("Disabled").theme(theme).enabled(false));
            let _ = ui.add(SecondaryButton::new("Disabled").theme(theme).enabled(false));
        });

        ui.add_space(theme.metrics.spacing.xl);
        section_title(
            ui,
            theme,
            "Transport & Count-in / Metronome Pill",
            "Tactile squircle controls and two-slot latch pill",
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(
                    TransportButton::new(
                        |ui, rect, col| Icon::paint(ui.painter(), rect, IconKind::Play, col),
                        TransportVariant::Play,
                    )
                    .theme(theme)
                    .active(state.playing),
                )
                .clicked()
            {
                state.playing = !state.playing;
            }
            if ui
                .add(
                    TransportButton::new(
                        |ui, rect, col| Icon::paint(ui.painter(), rect, IconKind::Stop, col),
                        TransportVariant::Stop,
                    )
                    .theme(theme),
                )
                .clicked()
            {
                state.playing = false;
                state.recording = false;
            }
            if ui
                .add(
                    TransportButton::new(
                        |ui, rect, col| Icon::paint(ui.painter(), rect, IconKind::Record, col),
                        TransportVariant::Record,
                    )
                    .theme(theme)
                    .active(state.recording),
                )
                .clicked()
            {
                state.recording = !state.recording;
            }

            ui.add_space(theme.metrics.spacing.md);

            let pill_resp = PillButton::new(
                |ui, rect, col| {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "123",
                        egui::FontId::proportional(11.0),
                        col,
                    );
                },
                |ui, rect, col| {
                    let c = rect.center();
                    let stroke = egui::Stroke::new(1.5, col);
                    ui.painter().line_segment(
                        [egui::pos2(c.x - 6.0, c.y + 7.0), egui::pos2(c.x, c.y - 7.0)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [egui::pos2(c.x, c.y - 7.0), egui::pos2(c.x + 6.0, c.y + 7.0)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [
                            egui::pos2(c.x - 4.0, c.y + 7.0),
                            egui::pos2(c.x + 4.0, c.y + 7.0),
                        ],
                        stroke,
                    );
                },
            )
            .theme(theme)
            .active_a(state.count_in)
            .active_b(state.metronome)
            .show(ui);

            if pill_resp.clicked_a {
                state.count_in = !state.count_in;
            }
            if pill_resp.clicked_b {
                state.metronome = !state.metronome;
            }

            ui.add_space(theme.metrics.spacing.md);

            if ui
                .add(
                    IconButton::new(|ui, rect, col| {
                        Icon::paint(ui.painter(), rect, IconKind::ChevronRight, col);
                    })
                    .theme(theme)
                    .active(state.loop_on)
                    .tooltip("Inspector"),
                )
                .clicked()
            {
                state.loop_on = !state.loop_on;
            }
        });
    }

    fn toggles_section(ui: &mut egui::Ui, theme: &Theme, state: &mut LabState) {
        section_title(
            ui,
            theme,
            "Switch toggles",
            "Animated thumb travel with semantic accent",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                Toggle::new(&mut state.snap_on)
                    .label("Snap to Grid")
                    .theme(theme),
            );
            ui.add_space(theme.metrics.spacing.lg);
            ui.add(
                Toggle::new(&mut state.auto_quantize)
                    .label("Auto Quantize")
                    .theme(theme),
            );
            ui.add_space(theme.metrics.spacing.lg);
            let mut disabled_val = false;
            ui.add(
                Toggle::new(&mut disabled_val)
                    .label("Disabled")
                    .theme(theme)
                    .enabled(false),
            );
        });

        ui.add_space(theme.metrics.spacing.xl);
        section_title(
            ui,
            theme,
            "Latch & Segmented controls",
            "Mutually exclusive and toggle buttons",
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(ToggleButton::new("Loop", state.loop_on).theme(theme))
                .clicked()
            {
                state.loop_on = !state.loop_on;
            }
            ui.add_space(theme.metrics.spacing.md);
            state.segmented_mode =
                SegmentedButton::new(&["Notes", "Chords", "Scales"], state.segmented_mode)
                    .theme(theme)
                    .show(ui);
        });
    }

    fn knobs_and_faders_section(ui: &mut egui::Ui, theme: &Theme, state: &mut LabState) {
        section_title(
            ui,
            theme,
            "Rotary knobs",
            "Vertical drag (Shift for fine control), double-click to reset",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                RotaryKnob::new(&mut state.gain_knob)
                    .label("Drive")
                    .theme(theme),
            );
            ui.add_space(theme.metrics.spacing.lg);
            ui.add(
                RotaryKnob::new(&mut state.pan_knob)
                    .mode(KnobMode::Bipolar)
                    .label("Pan")
                    .theme(theme),
            );
            ui.add_space(theme.metrics.spacing.lg);
            ui.add(
                RotaryKnob::new(&mut state.stepped_knob)
                    .mode(KnobMode::Stepped(5))
                    .label("Steps")
                    .theme(theme),
            );
            ui.add_space(theme.metrics.spacing.lg);
            ui.add(
                RotaryKnob::new(&mut state.mini_knob)
                    .mini()
                    .label("Send")
                    .theme(theme),
            );
        });

        ui.add_space(theme.metrics.spacing.xl);
        section_title(
            ui,
            theme,
            "Volume faders",
            "Musical dB taper (-60 dB .. +6 dB, double-click for 0.0 dB)",
        );
        ui.horizontal(|ui| {
            ui.add(VolumeFader::vertical(&mut state.fader_db).theme(theme));
            ui.add_space(theme.metrics.spacing.lg);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:.1} dB", state.fader_db))
                        .monospace()
                        .color(theme.palette.text_primary),
                );
                ui.add_space(theme.metrics.spacing.md);
                ui.add(VolumeFader::horizontal(&mut state.horiz_db).theme(theme));
                ui.label(
                    egui::RichText::new(format!("Aux: {:.1} dB", state.horiz_db))
                        .monospace()
                        .size(theme.typography.metadata)
                        .color(theme.palette.text_secondary),
                );
            });
        });
    }

    fn meters_section(ui: &mut egui::Ui, theme: &Theme, state: &mut LabState) {
        section_title(
            ui,
            theme,
            "Peak & RMS meters",
            "Green/amber/red zones with peak-hold ballistics",
        );
        ui.horizontal(|ui| {
            ui.add(PeakMeter::new(&state.peak_state).theme(theme));
            ui.add(PeakMeter::new(&state.peak_state).theme(theme));
            ui.add_space(theme.metrics.spacing.sm);
            ui.add(RmsMeter::new(state.meter_drive * 0.78).theme(theme));
            ui.add_space(theme.metrics.spacing.lg);
            ui.vertical(|ui| {
                ui.label("Input Drive");
                ui.add(egui::Slider::new(&mut state.meter_drive, 0.0..=1.0));
                ui.label(
                    egui::RichText::new(format!(
                        "Peak hold: {:.0}%",
                        state.peak_state.peak_hold * 100.0
                    ))
                    .monospace()
                    .size(theme.typography.metadata)
                    .color(theme.palette.text_secondary),
                );
            });
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
