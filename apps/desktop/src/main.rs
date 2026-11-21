use eframe::egui;
use resonance_audio_engine::io::{AudioDeviceInfo, AudioDeviceManager, cpal};
use resonance_audio_engine::{AudioEngine, EngineControl};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use vizia as _;

struct ResonanceApp {
    device_manager: Option<AudioDeviceManager>,
    output_devices: Vec<AudioDeviceInfo>,
    selected_device_name: String,
    engine_control: EngineControl,
    frequency: f32,
    amplitude: f32,
    stream_active: Arc<AtomicBool>,
    _audio_stream: Option<cpal::Stream>,
    status_message: String,
}

impl ResonanceApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Initialize logging
        let _ = tracing_subscriber::fmt().try_init();
        tracing::info!("Initializing Project Resonance Desktop Application");

        let (mut engine, control) = AudioEngine::new(48000.0);
        let stream_active = Arc::new(AtomicBool::new(false));

        let mut output_devices = Vec::new();
        let mut selected_name = String::new();
        let mut active_stream = None;
        let status;

        let device_manager = match AudioDeviceManager::new() {
            Ok(mgr) => {
                if let Ok(devs) = mgr.enumerate_output_devices() {
                    output_devices = devs;
                    if let Some(default_dev) = output_devices.iter().find(|d| d.is_default_output) {
                        selected_name = default_dev.name.clone();
                    } else if let Some(first) = output_devices.first() {
                        selected_name = first.name.clone();
                    }
                }

                // Attempt to start output stream on default device
                match mgr.default_output_device() {
                    Ok(dev) => {
                        let stream_flag = stream_active.clone();
                        match mgr.build_output_stream(&dev, 48000, 2, move |data: &mut [f32]| {
                            engine.process(data);
                        }) {
                            Ok(stream) => {
                                active_stream = Some(stream);
                                stream_flag.store(true, Ordering::Release);
                                status = "Audio output stream active (48 kHz Stereo)".to_string();
                            }
                            Err(e) => {
                                status = format!("Could not start audio stream: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        status = format!("No default audio device: {e}");
                    }
                }

                Some(mgr)
            }
            Err(e) => {
                status = format!("Failed to initialize audio subsystem: {e}");
                None
            }
        };

        Self {
            device_manager,
            output_devices,
            selected_device_name: selected_name,
            engine_control: control,
            frequency: 440.0,
            amplitude: 0.1,
            stream_active,
            _audio_stream: active_stream,
            status_message: status,
        }
    }
}

impl eframe::App for ResonanceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.engine_control.is_playing() {
            ui.ctx().request_repaint();
        }

        // Header Panel
        ui.horizontal(|ui| {
            ui.heading("Project Resonance");
            ui.label("•");
            ui.label("Developer Preview (Stage 2/3)");
            ui.separator();

            let is_stream_on = self.stream_active.load(Ordering::Acquire);
            if is_stream_on {
                ui.colored_label(
                    egui::Color32::from_rgb(80, 220, 100),
                    "AUDIO STREAM: ONLINE",
                );
            } else {
                ui.colored_label(
                    egui::Color32::from_rgb(240, 80, 80),
                    "AUDIO STREAM: OFFLINE",
                );
            }

            ui.separator();
            ui.label(format!(
                "Host: {}",
                self.device_manager
                    .as_ref()
                    .map_or("None".into(), |m| m.host_name())
            ));
        });

        ui.separator();
        ui.add_space(8.0);

        // Transport & Tone Generator Box
        ui.group(|ui| {
            ui.heading("Tone Generator & Real-Time Test");
            ui.label(
                "Generates a pure continuous sine wave directly inside the audio callback without blocking the UI.",
            );
            ui.add_space(8.0);

            let is_playing = self.engine_control.is_playing();

            ui.horizontal(|ui| {
                let play_button_text = if is_playing {
                    "⏸ Stop Playback"
                } else {
                    "▶ Play Tone (440 Hz)"
                };
                let btn_color = if is_playing {
                    egui::Color32::from_rgb(220, 80, 80)
                } else {
                    egui::Color32::from_rgb(60, 140, 240)
                };

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(play_button_text)
                                .color(egui::Color32::WHITE)
                                .size(16.0),
                        )
                        .fill(btn_color),
                    )
                    .clicked()
                {
                    self.engine_control.toggle_play();
                }

                if ui.button("Reset to 440 Hz").clicked() {
                    self.frequency = 440.0;
                    self.engine_control.set_frequency(440.0);
                }

                ui.separator();
                if is_playing {
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 220, 100),
                        format!("● PLAYING at {:.1} Hz", self.frequency),
                    );
                } else {
                    ui.colored_label(egui::Color32::GRAY, "○ STOPPED");
                }
            });

            ui.add_space(10.0);

            // Frequency Slider
            ui.horizontal(|ui| {
                ui.label("Frequency:");
                if ui
                    .add(
                        egui::Slider::new(&mut self.frequency, 20.0..=2000.0)
                            .suffix(" Hz")
                            .logarithmic(true),
                    )
                    .changed()
                {
                    self.engine_control.set_frequency(self.frequency);
                }
            });

            // Amplitude / Volume Slider
            ui.horizontal(|ui| {
                ui.label("Amplitude:");
                if ui
                    .add(
                        egui::Slider::new(&mut self.amplitude, 0.0..=1.0)
                            .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
                    )
                    .changed()
                {
                    self.engine_control.set_amplitude(self.amplitude);
                }
            });
        });

        ui.add_space(16.0);

        // Audio Hardware Devices Box
        ui.group(|ui| {
            ui.heading("Audio Hardware Devices (CPAL CoreAudio / WASAPI / ALSA)");
            ui.label("Discovered physical audio output endpoints on this workstation:");
            ui.add_space(6.0);

            if self.output_devices.is_empty() {
                ui.label("No audio output devices discovered.");
            } else {
                for dev in &self.output_devices {
                    ui.horizontal(|ui| {
                        let is_selected = dev.name == self.selected_device_name;
                        let prefix = if dev.is_default_output {
                            "★ [Default] "
                        } else {
                            "  "
                        };
                        ui.label(format!("{prefix}{}", dev.name));

                        if is_selected {
                            ui.colored_label(egui::Color32::from_rgb(80, 220, 100), "(Active)");
                        }

                        if dev.min_sample_rate > 0 {
                            ui.label(
                                egui::RichText::new(format!(
                                    "[{}-{} Hz, {}ch]",
                                    dev.min_sample_rate, dev.max_sample_rate, dev.max_channels
                                ))
                                .weak(),
                            );
                        }
                    });
                }
            }
        });

        ui.add_space(16.0);

        // GUI Framework Note (egui & vizia)
        ui.group(|ui| {
            ui.heading("GUI Architecture Note");
            ui.label(
                "Resonance integrates egui/eframe for immediate-mode workspace rendering, and retains vizia for audio widget and plugin UI interoperability.",
            );
        });

        ui.add_space(16.0);

        // Status Bar
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Status:");
            ui.label(egui::RichText::new(&self.status_message).monospace());
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 720.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Resonance — Audio Workstation"),
        ..Default::default()
    };

    eframe::run_native(
        "Resonance",
        options,
        Box::new(|cc| Ok(Box::new(ResonanceApp::new(cc)))),
    )
}
