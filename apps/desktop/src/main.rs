use eframe::egui;
use esperia_audio_engine::{AudioEngine, EngineControl};
use esperia_audio_io::{AudioDeviceInfo, AudioDeviceManager, cpal};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct EsperiaApp {
    device_manager: Option<AudioDeviceManager>,
    output_devices: Vec<AudioDeviceInfo>,
    selected_device_name: String,
    engine_control: EngineControl,
    frequency: f32,
    amplitude: f32,
    stream_active: Arc<AtomicBool>,
    _audio_stream: Option<cpal::Stream>,
    status_message: String,
    active_sample_rate: u32,
    active_channels: u16,
}

impl EsperiaApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Initialize logging
        let _ = tracing_subscriber::fmt().try_init();
        tracing::info!("Initializing Project Esperia Desktop Application");

        let stream_active = Arc::new(AtomicBool::new(false));

        // --- Try to bring up the audio subsystem ---
        let mgr = match AudioDeviceManager::new() {
            Ok(m) => m,
            Err(e) => {
                let (_eng, ctrl) = AudioEngine::new(48000.0, 2);
                return Self {
                    device_manager: None,
                    output_devices: Vec::new(),
                    selected_device_name: String::new(),
                    engine_control: ctrl,
                    frequency: 440.0,
                    amplitude: 0.1,
                    stream_active,
                    _audio_stream: None,
                    status_message: format!("Failed to initialize audio subsystem: {e}"),
                    active_sample_rate: 48000,
                    active_channels: 2,
                };
            }
        };

        // Enumerate output devices for the UI list
        let output_devices = mgr.enumerate_output_devices().unwrap_or_default();
        let selected_device_name = output_devices
            .iter()
            .find(|d| d.is_default_output)
            .or_else(|| output_devices.first())
            .map(|d| d.name.clone())
            .unwrap_or_default();

        // Query actual hardware config from the default output device
        let (sample_rate, channels, dev_result) = match mgr.default_output_device() {
            Ok(dev) => {
                let (sr, ch) = AudioDeviceManager::default_output_config_params(&dev);
                (sr, ch, Some(dev))
            }
            Err(_) => (48000, 2, None),
        };

        // Build the engine using the real device config
        let (mut engine, control) = AudioEngine::new(sample_rate as f32, channels);

        // Build the audio output stream
        let (active_stream, status_message) = match dev_result {
            Some(dev) => {
                let stream_flag = stream_active.clone();
                match mgr.build_output_stream(
                    &dev,
                    sample_rate,
                    channels,
                    move |data: &mut [f32]| {
                        engine.process(data);
                    },
                ) {
                    Ok(stream) => {
                        stream_flag.store(true, Ordering::Release);
                        let status = format!(
                            "Audio output stream active ({} Hz, {} ch)",
                            sample_rate, channels
                        );
                        (Some(stream), status)
                    }
                    Err(e) => (None, format!("Could not start audio stream: {e}")),
                }
            }
            None => (None, "No default audio device found".to_string()),
        };

        Self {
            device_manager: Some(mgr),
            output_devices,
            selected_device_name,
            engine_control: control,
            frequency: 440.0,
            amplitude: 0.1,
            stream_active,
            _audio_stream: active_stream,
            status_message,
            active_sample_rate: sample_rate,
            active_channels: channels,
        }
    }
}

impl eframe::App for EsperiaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.engine_control.is_playing() {
            ui.ctx().request_repaint();
        }

        // Header Panel
        ui.horizontal(|ui| {
            ui.heading("Project Esperia");
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
            ui.label(format!(
                "Active stream: {} Hz, {} ch — Discovered output endpoints:",
                self.active_sample_rate, self.active_channels
            ));
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
            .with_title("Esperia — Audio Workstation"),
        ..Default::default()
    };

    eframe::run_native(
        "Esperia",
        options,
        Box::new(|cc| Ok(Box::new(EsperiaApp::new(cc)))),
    )
}
