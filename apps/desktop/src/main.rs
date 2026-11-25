//! Esperia desktop application entry point.
//!
//! Responsibilities:
//! * Drive the egui UI.
//! * Own the CPAL stream lifetime (`_audio_stream`).
//! * Connect `esperia-audio-io` (device enumeration / stream) with
//!   `esperia-audio-engine` (DSP / transport) via `EngineHandle`.
//!
//! Neither `audio-engine` nor `audio-io` depends on each other — this crate
//! wires them together as the top-level coordinator.

use eframe::egui;
use esperia_audio_engine::{AudioEngine, EngineHandle};
use esperia_audio_io::{AudioDeviceInfo, AudioDeviceManager, StreamErrorChannel, cpal};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ─────────────────────────────────────────────────────────────────────────────
// Application state
// ─────────────────────────────────────────────────────────────────────────────

struct EsperiaApp {
    // ── Audio subsystem ──────────────────────────────────────────────────
    device_manager: Option<AudioDeviceManager>,
    output_devices: Vec<AudioDeviceInfo>,

    /// Currently selected device name (shown in combo-box).
    selected_device_name: String,

    /// Handle used by the UI thread to send commands and read telemetry.
    engine_handle: EngineHandle,

    /// Active CPAL stream.  Dropped when switching devices.
    _audio_stream: Option<cpal::Stream>,

    /// Set to `true` when a stream is successfully opened.
    stream_active: Arc<AtomicBool>,

    /// Resolved config of the currently open stream.
    active_sample_rate: u32,
    active_channels: u16,

    /// Buffer size hint (frames) used for DSP load calculation.
    /// We don't know the exact buffer size until CPAL reports it; use a
    /// reasonable default of 512 frames.
    buffer_frames_hint: Arc<AtomicU32>,

    /// Error reports from the CPAL stream error callback.
    stream_errors: StreamErrorChannel,

    // ── UI state ─────────────────────────────────────────────────────────
    frequency: f32,
    amplitude: f32,
    status_message: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Construction helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Try to open the audio stream on `device_name`.  Returns the stream, the
/// resolved (sample_rate, channels), and the status message.
///
/// `engine` is moved into the stream closure and must not be used afterwards.
fn open_stream(
    mgr: &AudioDeviceManager,
    device_name: &str,
    mut engine: AudioEngine,
    stream_active: Arc<AtomicBool>,
    buffer_frames_hint: Arc<AtomicU32>,
    error_channel: StreamErrorChannel,
) -> (Option<cpal::Stream>, u32, u16, String) {
    let dev_result = if device_name.is_empty() {
        mgr.default_output_device()
    } else {
        mgr.find_output_device_by_name(device_name)
            .or_else(|_| mgr.default_output_device())
    };

    let dev = match dev_result {
        Ok(d) => d,
        Err(e) => return (None, 48_000, 2, format!("No output device: {e}")),
    };

    let (sr, ch) = AudioDeviceManager::default_output_config_params(&dev);
    let budget_ns = AudioEngine::budget_ns(512, sr as f32); // initial budget estimate

    let bfh = buffer_frames_hint.clone();
    let stream_flag = stream_active.clone();

    let result = mgr.build_output_stream(&dev, sr, ch, error_channel, move |data: &mut [f32]| {
        // Update buffer size hint from actual callback length.
        let frames = (data.len() / ch as usize) as u32;
        bfh.store(frames, Ordering::Relaxed);
        engine.process(data, budget_ns);
    });

    match result {
        Ok(stream) => {
            stream_flag.store(true, Ordering::Release);
            let name = AudioDeviceManager::device_display_name(&dev);
            (
                Some(stream),
                sr,
                ch,
                format!("Stream active — {name} · {sr} Hz · {ch} ch"),
            )
        }
        Err(e) => {
            stream_flag.store(false, Ordering::Release);
            (None, sr, ch, format!("Stream error: {e}"))
        }
    }
}

impl EsperiaApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let _ = tracing_subscriber::fmt().try_init();
        tracing::info!("Initializing Esperia Desktop Application");

        let stream_active = Arc::new(AtomicBool::new(false));
        let buffer_frames_hint = Arc::new(AtomicU32::new(512));
        let stream_errors = StreamErrorChannel::new();

        // ── Initialise audio device manager ──────────────────────────────
        let mgr = match AudioDeviceManager::new() {
            Ok(m) => m,
            Err(e) => {
                let (_, handle) = AudioEngine::new(48_000.0, 2);
                return Self {
                    device_manager: None,
                    output_devices: Vec::new(),
                    selected_device_name: String::new(),
                    engine_handle: handle,
                    _audio_stream: None,
                    stream_active,
                    active_sample_rate: 48_000,
                    active_channels: 2,
                    buffer_frames_hint,
                    stream_errors,
                    frequency: 440.0,
                    amplitude: 0.1,
                    status_message: format!("Audio subsystem unavailable: {e}"),
                };
            }
        };

        let output_devices = mgr.enumerate_output_devices().unwrap_or_default();
        let selected_device_name = output_devices
            .iter()
            .find(|d| d.is_default_output)
            .or_else(|| output_devices.first())
            .map(|d| d.name.clone())
            .unwrap_or_default();

        // ── Build engine & stream ─────────────────────────────────────────
        // We first query the device config so we can build the engine with the
        // correct sample_rate and channel count before passing it into the closure.
        let (init_sr, init_ch) = mgr
            .default_output_device()
            .map(|d| AudioDeviceManager::default_output_config_params(&d))
            .unwrap_or((48_000, 2));

        let (engine, handle) = AudioEngine::new(init_sr as f32, init_ch);

        let (stream, active_sr, active_ch, status) = open_stream(
            &mgr,
            &selected_device_name,
            engine,
            stream_active.clone(),
            buffer_frames_hint.clone(),
            stream_errors.clone(),
        );

        Self {
            device_manager: Some(mgr),
            output_devices,
            selected_device_name,
            engine_handle: handle,
            _audio_stream: stream,
            stream_active,
            active_sample_rate: active_sr,
            active_channels: active_ch,
            buffer_frames_hint,
            stream_errors,
            frequency: 440.0,
            amplitude: 0.1,
            status_message: status,
        }
    }

    /// Switch to a new output device: drop the old stream, reconfigure the
    /// engine, and open a new stream.
    fn switch_device(&mut self, new_device_name: String) {
        // Drop existing stream first so the device is released.
        self._audio_stream = None;
        self.stream_active.store(false, Ordering::Release);

        let Some(mgr) = &self.device_manager else {
            self.status_message = "No audio device manager".to_string();
            return;
        };

        // Query new device config.
        let (new_sr, new_ch) = mgr
            .find_output_device_by_name(&new_device_name)
            .map(|d| AudioDeviceManager::default_output_config_params(&d))
            .unwrap_or((48_000, 2));

        // Tell the existing engine to reconfigure.  The engine was moved into
        // the previous stream closure, so we have to build a new one.
        //
        // Note: `engine` is moved into the closure; `handle` stays on the UI thread.
        let (engine, handle) = AudioEngine::new(new_sr as f32, new_ch);
        // Transfer existing play/stop state.
        if self.engine_handle.is_playing() {
            handle.play();
        }

        self.engine_handle = handle;

        let (stream, active_sr, active_ch, status) = open_stream(
            mgr,
            &new_device_name,
            engine,
            self.stream_active.clone(),
            self.buffer_frames_hint.clone(),
            self.stream_errors.clone(),
        );

        self._audio_stream = stream;
        self.selected_device_name = new_device_name;
        self.active_sample_rate = active_sr;
        self.active_channels = active_ch;
        self.status_message = status;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// egui rendering
// ─────────────────────────────────────────────────────────────────────────────

impl eframe::App for EsperiaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Repaint while playing so meters update.
        if self.engine_handle.is_playing() {
            ui.ctx().request_repaint();
        }

        // Check for stream errors reported by CPAL.
        if let Some(err) = self.stream_errors.take() {
            self.stream_active.store(false, Ordering::Release);
            self.status_message = format!("Stream error: {err}");
        }

        // ── Header ───────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.heading("Esperia");
            ui.weak("Audio Workstation · Developer Preview");
            ui.separator();

            if self.stream_active.load(Ordering::Acquire) {
                ui.colored_label(egui::Color32::from_rgb(80, 220, 100), "● STREAM ONLINE");
            } else {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), "● STREAM OFFLINE");
            }

            ui.separator();
            if let Some(mgr) = &self.device_manager {
                ui.weak(format!("Host: {}", mgr.host_name()));
            }
        });

        ui.separator();
        ui.add_space(6.0);

        // ── Device selection ─────────────────────────────────────────────
        ui.group(|ui| {
            ui.heading("Output Device");

            let mut new_selection: Option<String> = None;

            ui.horizontal(|ui| {
                ui.label("Device:");
                egui::ComboBox::from_id_salt("output_device")
                    .selected_text(&self.selected_device_name)
                    .width(360.0)
                    .show_ui(ui, |ui| {
                        for dev in &self.output_devices {
                            let label = if dev.is_default_output {
                                format!("★ {}", dev.name)
                            } else {
                                format!("  {}", dev.name)
                            };
                            if ui
                                .selectable_label(dev.name == self.selected_device_name, &label)
                                .clicked()
                            {
                                new_selection = Some(dev.name.clone());
                            }
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.weak(format!(
                    "Active config: {} Hz · {} ch",
                    self.active_sample_rate, self.active_channels
                ));

                // DSP load
                let sr = self.active_sample_rate as f32;
                let frames = self.buffer_frames_hint.load(Ordering::Relaxed);
                let load = self.engine_handle.telemetry.dsp_load(frames, sr);
                let load_pct = (load * 100.0).min(999.9);
                let load_color = if load_pct < 50.0 {
                    egui::Color32::from_rgb(80, 220, 100)
                } else if load_pct < 85.0 {
                    egui::Color32::from_rgb(255, 200, 0)
                } else {
                    egui::Color32::from_rgb(220, 80, 80)
                };
                ui.separator();
                ui.colored_label(load_color, format!("DSP {load_pct:.1}%"));

                // Underrun counter
                let ur = self.engine_handle.telemetry.underruns();
                if ur > 0 {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 80, 80),
                        format!("⚠ {ur} underrun(s)"),
                    );
                }
            });

            if let Some(name) = new_selection
                && name != self.selected_device_name
            {
                self.switch_device(name);
            }
        });

        ui.add_space(10.0);

        // ── Tone Generator ────────────────────────────────────────────────
        ui.group(|ui| {
            ui.heading("Tone Generator");
            ui.label(
                "Pure sine wave rendered inside the real-time callback. Zero heap allocations.",
            );
            ui.add_space(6.0);

            let is_playing = self.engine_handle.is_playing();

            ui.horizontal(|ui| {
                let (label, color) = if is_playing {
                    ("⏸  Stop", egui::Color32::from_rgb(220, 80, 80))
                } else {
                    ("▶  Play Tone", egui::Color32::from_rgb(60, 140, 240))
                };

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(label)
                                .color(egui::Color32::WHITE)
                                .size(15.0),
                        )
                        .fill(color),
                    )
                    .clicked()
                {
                    self.engine_handle.toggle_play();
                }

                if ui.button("Reset 440 Hz").clicked() {
                    self.frequency = 440.0;
                    self.engine_handle.set_frequency(440.0);
                }

                ui.separator();
                if is_playing {
                    // Peak meter
                    let peak = self.engine_handle.telemetry.peak();
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 220, 100),
                        format!("● {:.1} Hz  peak {:.3}", self.frequency, peak),
                    );
                } else {
                    ui.colored_label(egui::Color32::GRAY, "○ stopped");
                }
            });

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("Frequency:");
                if ui
                    .add(
                        egui::Slider::new(&mut self.frequency, 20.0..=2_000.0)
                            .suffix(" Hz")
                            .logarithmic(true),
                    )
                    .changed()
                {
                    self.engine_handle.set_frequency(self.frequency);
                }
            });

            ui.horizontal(|ui| {
                ui.label("Amplitude:");
                if ui
                    .add(
                        egui::Slider::new(&mut self.amplitude, 0.0..=1.0)
                            .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
                    )
                    .changed()
                {
                    self.engine_handle.set_amplitude(self.amplitude);
                }
            });
        });

        ui.add_space(10.0);

        // ── All discovered output devices ─────────────────────────────────
        ui.group(|ui| {
            ui.heading("Discovered Output Endpoints");
            ui.add_space(4.0);
            if self.output_devices.is_empty() {
                ui.label("No output devices discovered.");
            } else {
                for dev in &self.output_devices {
                    ui.horizontal(|ui| {
                        let prefix = if dev.is_default_output { "★" } else { " " };
                        ui.label(format!("{prefix} {}", dev.name));
                        if dev.name == self.selected_device_name {
                            ui.colored_label(egui::Color32::from_rgb(80, 220, 100), "(active)");
                        }
                        if dev.min_sample_rate > 0 {
                            ui.weak(format!(
                                "[{}-{} Hz, {}ch]",
                                dev.min_sample_rate, dev.max_sample_rate, dev.max_channels
                            ));
                        }
                    });
                }
            }
        });

        ui.add_space(10.0);

        // ── Status bar ────────────────────────────────────────────────────
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Status:");
            ui.monospace(&self.status_message);
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// main
// ─────────────────────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Esperia",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1_100.0, 760.0])
                .with_min_inner_size([800.0, 500.0])
                .with_title("Esperia — Audio Workstation"),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(EsperiaApp::new(cc)))),
    )
}
