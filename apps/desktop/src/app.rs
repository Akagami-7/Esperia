//! Esperia desktop application entry point.
//!
//! Responsibilities:
//! * Drive the egui UI.
//! * Own the CPAL stream lifetime (`_audio_stream`).
//! * Connect `esperia-audio-io` (device enumeration / stream) with
//!   `esperia-audio-engine` (DSP / transport) via `EngineHandle`.
//! * Display a multitrack timeline with waveform thumbnails, clip move/delete,
//!   track mute/solo, project save/load, and transport controls.
//!
//! Neither `audio-engine` nor `audio-io` depends on each other — this crate
//! wires them together as the top-level coordinator.

#[cfg(target_os = "macos")]
#[path = "macos_menu.rs"]
mod macos_menu;
#[path = "piano_roll.rs"]
mod piano_roll;
#[path = "volume_glider.rs"]
mod volume_glider;

use eframe::egui;
use esperia_audio_engine::{AudioEngine, ClipSchedule, EngineHandle, MonitorRing, ScheduledClip};
use esperia_audio_io::{AudioDeviceInfo, AudioDeviceManager, StreamErrorChannel, cpal};
use esperia_media::{AssetManager, DecodedAudio, LoadStatus};
use esperia_midi::{
    InstrumentKind, MidiClip, MidiInputConnection, MidiInputManager, MidiInputRing, MidiMessage,
    MidiPortInfo, MidiRecorder, MidiSchedule, MidiTrack, PPQN,
};
use esperia_mixer::{BusConfig, ChannelConfig, MixerGraph, MixerMeters, RouteTarget};
use esperia_project_model::{
    audio_source::SourceId,
    clip::ClipId,
    commands::{CommandOutcome, ProjectCommand, ProjectSession},
    persistence::{load_project, save_project},
    project::Project,
    track::TrackId,
};
use esperia_recording::{
    InputChannelSelection, LatencyConfig, PunchRegion, Recorder, RecordingStatus,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use volume_glider::VolumeGlider;

// ─────────────────────────────────────────────────────────────────────────────
// Timeline constants
// ─────────────────────────────────────────────────────────────────────────────

const TRACK_HEADER_WIDTH: f32 = 210.0;
const TRACK_HEIGHT: f32 = 76.0;
const RULER_HEIGHT: f32 = 24.0;
const MIN_ZOOM: f32 = 0.0005; // pixels per frame
const MAX_ZOOM: f32 = 2.0;
const DEFAULT_ZOOM: f32 = 0.005; // at 48kHz: ~240 px/s

// ─────────────────────────────────────────────────────────────────────────────
// Pending import & drag state
// ─────────────────────────────────────────────────────────────────────────────

struct PendingImport {
    temp_id: SourceId,
    name: String,
    path: PathBuf,
    target_track_id: Option<TrackId>,
    start_frame: u64,
}

#[derive(Clone)]
struct DragState {
    clip_id: ClipId,
    track_id: TrackId,
    original_start_frame: u64,
    drag_start_px: f32,
    current_offset_frames: i64,
}

#[derive(Clone)]
struct MidiClipDragState {
    track_id: u64,
    clip_id: u64,
    original_start_tick: u64,
    drag_start_px: f32,
    current_offset_ticks: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Waveform thumbnail cache (peak arrays per source)
// ─────────────────────────────────────────────────────────────────────────────

struct ThumbnailCache {
    peaks: HashMap<(SourceId, usize), Vec<f32>>,
}

impl ThumbnailCache {
    fn new() -> Self {
        Self {
            peaks: HashMap::new(),
        }
    }

    fn get_or_compute(
        &mut self,
        source_id: SourceId,
        audio: &DecodedAudio,
        step_frames: usize,
    ) -> &[f32] {
        let step = step_frames.max(64);
        self.peaks
            .entry((source_id, step))
            .or_insert_with(|| audio.peak_thumbnail(step))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Application state
// ─────────────────────────────────────────────────────────────────────────────

struct EsperiaApp {
    #[cfg(target_os = "macos")]
    native_menu: Option<macos_menu::NativeMenu>,

    // ── Audio subsystem ──────────────────────────────────────────────────
    device_manager: Option<AudioDeviceManager>,
    output_devices: Vec<AudioDeviceInfo>,
    selected_device_name: String,
    engine_handle: EngineHandle,
    _audio_stream: Option<cpal::Stream>,
    stream_active: Arc<AtomicBool>,
    active_sample_rate: u32,
    active_channels: u16,
    buffer_frames_hint: Arc<AtomicU32>,
    stream_errors: StreamErrorChannel,

    // ── Audio input & recording subsystem ────────────────────────────────
    input_devices: Vec<AudioDeviceInfo>,
    selected_input_device_name: String,
    _input_stream: Option<cpal::Stream>,
    input_stream_active: Arc<AtomicBool>,
    input_sample_rate: u32,
    input_channels: u16,
    input_buffer_frames_hint: Arc<AtomicU32>,
    input_stream_errors: StreamErrorChannel,
    selected_input_channel: u16,
    recorder: Recorder,
    input_monitoring: bool,
    punch_enabled: bool,

    // ── Project ──────────────────────────────────────────────────────────
    project_session: ProjectSession,
    project_bundle_dir: Option<PathBuf>,
    /// Tracks absolute paths of imported/recorded sources before saving into bundle.
    source_abs_paths: HashMap<SourceId, PathBuf>,

    // ── Asset management ─────────────────────────────────────────────────
    asset_manager: AssetManager,
    thumbnail_cache: ThumbnailCache,
    pending_imports: Vec<PendingImport>,
    next_temp_source_seq: u64,

    // ── Mixer ─────────────────────────────────────────────────────────────
    mixer_meters: Option<MixerMeters>,
    show_mixer_panel: bool,
    master_volume: VolumeGlider,

    // ── MIDI & Piano Roll ────────────────────────────────────────────────
    midi_ports: Vec<MidiPortInfo>,
    selected_midi_port: Option<usize>,
    _midi_connection: Option<MidiInputConnection>,
    midi_input_ring: Arc<MidiInputRing>,
    midi_recorder: MidiRecorder,
    show_piano_roll: bool,
    piano_roll_height: f32,
    active_midi_track_id: Option<u64>,
    active_midi_clip_id: Option<u64>,
    piano_roll_state: piano_roll::PianoRollState,

    // ── UI state ─────────────────────────────────────────────────────────
    frequency: f32,
    amplitude: f32,
    status_message: String,
    last_command_queue_rejections: u64,
    last_schedule_update_rejections: u64,
    show_device_panel: bool,

    // ── Timeline state ───────────────────────────────────────────────────
    timeline_zoom: f32,
    selected_clip: Option<ClipId>,
    drag_state: Option<DragState>,
    midi_drag_state: Option<MidiClipDragState>,
    scrubbing_playhead: bool,
    loop_enabled: bool,
    loop_start_frame: u64,
    loop_end_frame: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Construction helpers
// ─────────────────────────────────────────────────────────────────────────────

fn open_stream(
    mgr: &AudioDeviceManager,
    device_name: &str,
    mut engine: AudioEngine,
    stream_active: Arc<AtomicBool>,
    buffer_frames_hint: Arc<AtomicU32>,
    rec_telemetry: Arc<esperia_recording::RecordingTelemetry>,
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
    let budget_ns = AudioEngine::budget_ns(512, sr as f32);

    let bfh = buffer_frames_hint.clone();
    let stream_flag = stream_active.clone();

    let result = mgr.build_output_stream_with_timing(
        &dev,
        sr,
        ch,
        error_channel,
        move |data: &mut [f32], latency_ns: Option<u64>| {
            let frames = (data.len() / ch as usize) as u32;
            bfh.store(frames, Ordering::Relaxed);
            if let Some(ns) = latency_ns {
                rec_telemetry
                    .last_output_latency_ns
                    .store(ns, Ordering::Relaxed);
            }
            engine.process(data, budget_ns);
        },
    );

    match result {
        Ok(stream) => {
            stream_flag.store(true, Ordering::Release);
            let name = AudioDeviceManager::device_display_name(&dev);
            (
                Some(stream),
                sr,
                ch,
                format!("Stream open: {name} @ {sr} Hz, {ch}ch"),
            )
        }
        Err(e) => (None, sr, ch, format!("Stream error: {e}")),
    }
}

fn open_input_stream(
    mgr: &AudioDeviceManager,
    device_name: &str,
    recorder: &Recorder,
    engine_handle: &EngineHandle,
    stream_active: Arc<AtomicBool>,
    buffer_frames_hint: Arc<AtomicU32>,
    error_channel: StreamErrorChannel,
) -> (Option<cpal::Stream>, u32, u16) {
    let dev_result = if device_name.is_empty() {
        mgr.default_input_device()
    } else {
        mgr.find_input_device_by_name(device_name)
            .or_else(|_| mgr.default_input_device())
    };

    let Ok(dev) = dev_result else {
        return (None, 48_000, 1);
    };

    let (sr, ch) = AudioDeviceManager::default_input_config_params(&dev);
    let Ok(mut capture) = recorder.build_input_capture(ch) else {
        return (None, sr, ch);
    };

    let monitor_ring: Arc<MonitorRing> = engine_handle.monitor_ring();
    let telemetry = engine_handle.telemetry.clone();
    let rec_telemetry = recorder.telemetry();
    let bfh = buffer_frames_hint;
    let stream_flag = stream_active;

    let result = mgr.build_input_stream(
        &dev,
        sr,
        ch,
        error_channel,
        move |data: &[f32], latency_ns: Option<u64>| {
            let frames = (data.len() / ch.max(1) as usize) as u32;
            bfh.store(frames, Ordering::Relaxed);
            let timeline_frame = telemetry.playhead_frame();
            capture.on_input_samples(data, timeline_frame, latency_ns);
            if recorder_mon_enabled(&rec_telemetry, &capture) {
                monitor_ring.push_samples(capture.last_captured_slice());
            }
        },
    );

    match result {
        Ok(stream) => {
            stream_flag.store(true, Ordering::Release);
            (Some(stream), sr, ch)
        }
        Err(_) => (None, sr, ch),
    }
}

#[inline]
fn recorder_mon_enabled(
    _tel: &esperia_recording::RecordingTelemetry,
    capture: &esperia_recording::InputCapture,
) -> bool {
    !capture.last_captured_slice().is_empty()
}

impl EsperiaApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        esperia_ui::Theme::default().apply(&cc.egui_ctx);
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .try_init();

        let (engine, engine_handle) = AudioEngine::new(48_000.0, 2);

        let stream_active = Arc::new(AtomicBool::new(false));
        let buffer_frames_hint = Arc::new(AtomicU32::new(512));
        let stream_errors = StreamErrorChannel::new();

        let (mgr, output_devices, selected_device_name, input_devices, selected_input_device_name) =
            match AudioDeviceManager::new() {
                Ok(m) => {
                    let out_devs = m.enumerate_output_devices().unwrap_or_default();
                    let default_out = out_devs
                        .iter()
                        .find(|d| d.is_default_output)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    let in_devs = m.enumerate_input_devices().unwrap_or_default();
                    let default_in = in_devs
                        .iter()
                        .find(|d| d.is_default_input)
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    (Some(m), out_devs, default_out, in_devs, default_in)
                }
                Err(e) => {
                    tracing::warn!("AudioDeviceManager init failed: {e}");
                    (None, vec![], String::new(), vec![], String::new())
                }
            };

        let recording_staging_dir = std::env::temp_dir().join("esperia_recordings");
        let recorder = Recorder::new(recording_staging_dir.clone());

        let (stream, active_sr, active_ch, status) = if let Some(ref m) = mgr {
            open_stream(
                m,
                &selected_device_name,
                engine,
                Arc::clone(&stream_active),
                Arc::clone(&buffer_frames_hint),
                recorder.telemetry(),
                stream_errors.clone(),
            )
        } else {
            (None, 48_000, 2, "No audio device manager".into())
        };

        let input_stream_active = Arc::new(AtomicBool::new(false));
        let input_buffer_frames_hint = Arc::new(AtomicU32::new(512));
        let input_stream_errors = StreamErrorChannel::new();

        let (input_stream, input_sr, input_ch) = if let Some(ref m) = mgr {
            open_input_stream(
                m,
                &selected_input_device_name,
                &recorder,
                &engine_handle,
                Arc::clone(&input_stream_active),
                Arc::clone(&input_buffer_frames_hint),
                input_stream_errors.clone(),
            )
        } else {
            (None, 48_000, 1)
        };

        let mut project = Project::new("Untitled".to_string(), active_sr, 120.0)
            .expect("default project parameters are valid");
        let default_tid = project.allocate_id();
        let default_cid = project.allocate_id();
        let mut default_midi = MidiTrack::new(default_tid, "MIDI 1");
        default_midi
            .clips
            .push(MidiClip::new(default_cid, "Pattern 1", 0, PPQN * 4));
        project.midi_tracks.push(default_midi);

        let project_session = ProjectSession::new(project);
        let asset_manager = AssetManager::new(Some(active_sr));
        let midi_ports = MidiInputManager::enumerate_input_ports().unwrap_or_default();
        let midi_input_ring = Arc::new(MidiInputRing::new());

        #[cfg(target_os = "macos")]
        let native_menu = match macos_menu::NativeMenu::install() {
            Ok(menu) => Some(menu),
            Err(error) => {
                tracing::error!(%error, "failed to install macOS menu bar");
                None
            }
        };

        let mut app = Self {
            #[cfg(target_os = "macos")]
            native_menu,
            device_manager: mgr,
            output_devices,
            selected_device_name,
            engine_handle,
            _audio_stream: stream,
            stream_active,
            active_sample_rate: active_sr,
            active_channels: active_ch,
            buffer_frames_hint,
            stream_errors,
            input_devices,
            selected_input_device_name,
            _input_stream: input_stream,
            input_stream_active,
            input_sample_rate: input_sr,
            input_channels: input_ch,
            input_buffer_frames_hint,
            input_stream_errors,
            selected_input_channel: 0,
            recorder,
            input_monitoring: false,
            punch_enabled: false,
            project_session,
            project_bundle_dir: None,
            source_abs_paths: HashMap::new(),
            asset_manager,
            thumbnail_cache: ThumbnailCache::new(),
            pending_imports: Vec::new(),
            next_temp_source_seq: 1,
            frequency: 440.0,
            amplitude: 0.1,
            status_message: status,
            last_command_queue_rejections: 0,
            last_schedule_update_rejections: 0,
            show_device_panel: false,
            mixer_meters: None,
            show_mixer_panel: false,
            master_volume: VolumeGlider::new(),
            midi_ports,
            selected_midi_port: None,
            _midi_connection: None,
            midi_input_ring,
            midi_recorder: MidiRecorder::new(),
            show_piano_roll: true,
            piano_roll_height: 320.0,
            active_midi_track_id: Some(default_tid),
            active_midi_clip_id: Some(default_cid),
            piano_roll_state: piano_roll::PianoRollState::default(),
            timeline_zoom: DEFAULT_ZOOM,
            selected_clip: None,
            drag_state: None,
            midi_drag_state: None,
            scrubbing_playhead: false,
            loop_enabled: false,
            loop_start_frame: 0,
            loop_end_frame: (active_sr as u64) * 4,
        };
        app.rebuild_midi_schedule();
        app
    }

    // ── Device switching ──────────────────────────────────────────────────────

    fn switch_device(&mut self, device_name: &str) {
        let Some(mgr) = &self.device_manager else {
            return;
        };
        let (engine, new_handle) =
            AudioEngine::new(self.active_sample_rate as f32, self.active_channels);
        self.stream_active.store(false, Ordering::Release);

        let (stream, sr, ch, status) = open_stream(
            mgr,
            device_name,
            engine,
            Arc::clone(&self.stream_active),
            Arc::clone(&self.buffer_frames_hint),
            self.recorder.telemetry(),
            self.stream_errors.clone(),
        );

        self._audio_stream = stream;
        self.engine_handle = new_handle;
        self.active_sample_rate = sr;
        self.active_channels = ch;
        self.selected_device_name = device_name.to_owned();
        self.status_message = status;
        self.engine_handle.reconfigure(sr as f32, ch);
        self.engine_handle
            .set_input_monitoring(self.input_monitoring);
        self.rebuild_schedule();
        self.switch_input_device(&self.selected_input_device_name.clone());
    }

    fn switch_input_device(&mut self, device_name: &str) {
        let Some(mgr) = &self.device_manager else {
            return;
        };
        self.input_stream_active.store(false, Ordering::Release);
        let (stream, sr, ch) = open_input_stream(
            mgr,
            device_name,
            &self.recorder,
            &self.engine_handle,
            Arc::clone(&self.input_stream_active),
            Arc::clone(&self.input_buffer_frames_hint),
            self.input_stream_errors.clone(),
        );
        self._input_stream = stream;
        self.input_sample_rate = sr;
        self.input_channels = ch;
        self.selected_input_device_name = device_name.to_owned();
    }

    // ── Recording control ─────────────────────────────────────────────────────

    fn toggle_recording(&mut self) {
        if matches!(self.recorder.status(), RecordingStatus::Recording { .. }) {
            // Stop recording and playback.
            self.engine_handle.pause();
            let latency = LatencyConfig {
                input_latency_ns: 0,
                output_latency_ns: 0,
                input_buffer_frames: self.input_buffer_frames_hint.load(Ordering::Relaxed),
                output_buffer_frames: self.buffer_frames_hint.load(Ordering::Relaxed),
                input_sample_rate: self.input_sample_rate,
                output_sample_rate: self.active_sample_rate,
                project_sample_rate: self.active_sample_rate,
                manual_offset_frames: 0,
            };
            match self.recorder.stop_recording(latency) {
                Ok(take) => {
                    self.ingest_recorded_take(&take);
                }
                Err(e) => {
                    self.status_message = format!("Recording error: {e}");
                }
            }
            return;
        }

        // Ensure an armed track exists (auto-create one if project has none, or
        // use the armed track).
        let armed_track_id = self
            .project_session
            .project()
            .tracks
            .iter()
            .find(|t| t.record_armed)
            .map(|t| t.id)
            .or_else(|| self.recorder.armed_track());

        let track_id = match armed_track_id {
            Some(tid) => tid,
            None => {
                // Auto-create and arm a new audio track for convenience.
                let idx = self.project_session.project().tracks.len() + 1;
                match self.project_session.execute(ProjectCommand::AddTrack {
                    name: format!("Audio {idx}"),
                }) {
                    Ok(CommandOutcome::TrackAdded(tid)) => {
                        let _ = self
                            .project_session
                            .execute(ProjectCommand::SetTrackRecordArmed {
                                track_id: tid,
                                armed: true,
                            });
                        tid
                    }
                    _ => return,
                }
            }
        };

        if let Err(e) = self.recorder.arm_track(Some(track_id)) {
            self.status_message = format!("Arm error: {e}");
            return;
        }

        let start_frame = self.engine_handle.playhead_frame();
        if let Err(e) = self.recorder.start_recording(
            start_frame,
            self.input_sample_rate,
            self.active_sample_rate,
            self.input_channels.max(1),
        ) {
            self.status_message = format!("Start recording failed: {e}");
            return;
        }

        // Start playback simultaneously so existing tracks play (overdubbing).
        self.engine_handle.play();
        self.status_message = "Recording…".into();
    }

    fn ingest_recorded_take(&mut self, take: &esperia_recording::TakeInfo) {
        if take.duration_frames == 0 {
            self.status_message = "Recording stopped (0 frames captured)".into();
            return;
        }

        let file_name = take
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("take.wav");
        let relative_path = PathBuf::from("Audio").join(file_name);
        let clip_name = format!("Take {}", take.take_number);

        let decoded = match esperia_media::decode::decode_audio(
            &take.file_path,
            Some(self.active_sample_rate),
        ) {
            Ok(d) => Arc::new(d),
            Err(e) => {
                self.status_message = format!("Failed to decode recorded take: {e}");
                return;
            }
        };

        let source_id = match self.project_session.execute(ProjectCommand::ImportAudio {
            name: clip_name.clone(),
            relative_path,
            sample_rate: decoded.sample_rate,
            channels: decoded.channels,
            duration_frames: decoded.duration_frames.max(1),
        }) {
            Ok(CommandOutcome::AudioImported(id)) => id,
            Ok(_) => return,
            Err(e) => {
                self.status_message = format!("ImportAudio error: {e}");
                return;
            }
        };

        self.source_abs_paths
            .insert(source_id, take.file_path.clone());
        self.asset_manager.register(source_id, Arc::clone(&decoded));

        if let Err(e) = self.project_session.execute(ProjectCommand::CreateClip {
            track_id: take.track_id,
            source_id,
            name: clip_name,
            timeline_start_frame: take.timeline_start_frame,
            source_offset_frame: 0,
            duration_frames: decoded.duration_frames.max(1),
        }) {
            self.status_message = format!("CreateClip error: {e}");
            return;
        }

        self.rebuild_schedule();
        self.status_message = format!(
            "Recorded Take {} ({:.2}s, comp {:+} frames)",
            take.take_number,
            decoded.duration_frames as f64 / decoded.sample_rate.max(1) as f64,
            take.applied_latency_compensation_frames
        );
    }

    // ── Schedule rebuild ──────────────────────────────────────────────────────

    fn rebuild_schedule(&mut self) {
        let project = self.project_session.project();
        let any_solo = project.tracks.iter().any(|t| t.soloed);
        let mut clips: Vec<ScheduledClip> = Vec::new();

        for track in &project.tracks {
            let track_audible = if any_solo {
                track.soloed && !track.muted
            } else {
                !track.muted
            };
            if !track_audible {
                continue;
            }
            let track_gain = db_to_linear(track.gain_db);

            for clip in &track.clips {
                let Some(audio) = self.asset_manager.get(&clip.source_id) else {
                    continue;
                };
                let clip_gain = db_to_linear(clip.gain_db) * track_gain;
                let max_available = audio
                    .duration_frames
                    .saturating_sub(clip.source_offset_frame);
                let duration_frames = clip.duration_frames.min(max_available);
                if duration_frames == 0 {
                    continue;
                }

                // Clone samples into an Arc<Vec<f32>> for the audio engine.
                let samples = Arc::new(audio.samples.clone());

                clips.push(ScheduledClip {
                    clip_id: clip.id.0,
                    track_id: track.id.0,
                    timeline_start_frame: clip.timeline_start_frame,
                    source_offset_frame: clip.source_offset_frame,
                    duration_frames,
                    gain: clip_gain,
                    channels: audio.channels,
                    samples,
                });
            }
        }

        let schedule = Arc::new(ClipSchedule { clips });
        if let Err(error) = self.engine_handle.set_clip_schedule(schedule) {
            self.status_message = format!("Clip schedule update rejected: {error}");
        }
    }

    // ── Mixer graph rebuild ────────────────────────────────────────────────────

    /// Build a fresh `MixerGraph` from current project tracks and push it to
    /// the audio thread lock-free.  Called after every track/project change.
    fn rebuild_mixer(&mut self) {
        let project = self.project_session.project();
        let any_solo = project.tracks.iter().any(|t| t.soloed);

        let channel_cfgs: Vec<ChannelConfig> = project
            .tracks
            .iter()
            .map(|t| {
                let audible = if any_solo {
                    t.soloed && !t.muted
                } else {
                    !t.muted
                };
                ChannelConfig {
                    id: t.id.0,
                    name: t.name.clone(),
                    gain_db: t.gain_db,
                    pan: 0.0, // Track pan not yet in project model; default centre
                    muted: !audible,
                    soloed: t.soloed,
                    output: RouteTarget::Master,
                    sends: vec![],
                }
            })
            .collect();

        // Buffer size hint from current stream; fall back to 1024 frames
        let buf_frames = self.buffer_frames_hint.load(Ordering::Relaxed).max(64) as usize;
        // Allocate with 4× headroom so the preallocated buffer covers large
        // device buffer sizes without real-time reallocation.
        let max_frames = buf_frames * 4;

        let buses: Vec<BusConfig> = vec![];

        let master_gain_db = if self.master_volume.value() <= 0.001 {
            -96.0
        } else {
            20.0 * self.master_volume.value().log10()
        };

        match MixerGraph::new(&channel_cfgs, &buses, master_gain_db, max_frames) {
            Ok(graph) => {
                let meters = self.engine_handle.set_mixer_graph(graph);
                self.mixer_meters = Some(meters);
            }
            Err(e) => {
                tracing::warn!("rebuild_mixer: MixerGraph::new failed: {e:?}");
            }
        }
    }

    // ── MIDI scheduling, input & recording ────────────────────────────────────

    fn rebuild_midi_schedule(&mut self) {
        let project = self.project_session.project();
        let sched = MidiSchedule::from_tracks(
            &project.midi_tracks,
            &project.tempo_map,
            self.active_sample_rate,
        );
        if let Err(error) = self.engine_handle.set_midi_schedule(Arc::new(sched)) {
            self.status_message = format!("MIDI schedule update rejected: {error}");
        }
    }

    fn connect_midi_port(&mut self, port_index: usize) {
        match MidiInputManager::open_input_port(port_index, Arc::clone(&self.midi_input_ring)) {
            Ok(conn) => {
                self._midi_connection = Some(conn);
                self.selected_midi_port = Some(port_index);
                let name = self
                    .midi_ports
                    .get(port_index)
                    .map_or("MIDI Port", |p| p.name.as_str());
                self.status_message = format!("MIDI connected: {name}");
            }
            Err(e) => {
                self.status_message = format!("MIDI connect failed: {e}");
            }
        }
    }

    fn poll_midi_input(&mut self) {
        let playhead = self.engine_handle.playhead_frame();
        let tempo_map = self.project_session.project().tempo_map.clone();
        while let Some(ev) = self.midi_input_ring.pop() {
            self.engine_handle.send_live_midi(ev.message);
            if self.midi_recorder.is_recording() {
                self.midi_recorder.record_message(
                    ev.message,
                    playhead,
                    &tempo_map,
                    self.active_sample_rate,
                );
            }
        }
    }

    fn add_midi_track(&mut self) {
        let proj = self.project_session.project_mut();
        let tid = proj.allocate_id();
        let cid = proj.allocate_id();
        let idx = proj.midi_tracks.len() + 1;
        let mut track = MidiTrack::new(tid, format!("MIDI {idx}"));
        track
            .clips
            .push(MidiClip::new(cid, format!("Pattern {idx}"), 0, PPQN * 4));
        proj.midi_tracks.push(track);

        self.active_midi_track_id = Some(tid);
        self.active_midi_clip_id = Some(cid);
        self.show_piano_roll = true;
        self.rebuild_midi_schedule();
        self.status_message = format!("Added MIDI {idx}");
    }

    fn create_midi_region(&mut self, track_id: u64, start_tick: u64) {
        let proj = self.project_session.project_mut();
        let cid = proj.allocate_id();
        if let Some(track) = proj.midi_tracks.iter_mut().find(|t| t.id == track_id) {
            let pat_idx = track.clips.len() + 1;
            let clip = MidiClip::new(cid, format!("Pattern {pat_idx}"), start_tick, PPQN * 4);
            track.clips.push(clip);
            self.active_midi_track_id = Some(track_id);
            self.active_midi_clip_id = Some(cid);
            self.show_piano_roll = true;
            self.piano_roll_state.scroll_to_tick(start_tick);
            self.status_message = format!("Created MIDI region Pattern {pat_idx}");
        }
        self.rebuild_midi_schedule();
    }

    fn toggle_midi_recording(&mut self) {
        if self.midi_recorder.is_recording() {
            let stop_frame = self.engine_handle.playhead_frame();
            let tempo_map = self.project_session.project().tempo_map.clone();
            self.engine_handle.pause();
            if let Some((tid, clip)) =
                self.midi_recorder
                    .stop_recording(stop_frame, &tempo_map, self.active_sample_rate)
            {
                let cid = clip.id;
                let note_count = clip.notes.len();
                let proj = self.project_session.project_mut();
                if let Some(track) = proj.midi_tracks.iter_mut().find(|t| t.id == tid) {
                    track.clips.push(clip);
                }
                self.active_midi_track_id = Some(tid);
                self.active_midi_clip_id = Some(cid);
                self.show_piano_roll = true;
                self.rebuild_midi_schedule();
                self.status_message = format!("MIDI recorded ({note_count} notes)");
            }
            return;
        }

        if self.active_midi_track_id.is_none()
            && self.project_session.project().midi_tracks.is_empty()
        {
            self.add_midi_track();
        }
        let target_tid = self.active_midi_track_id.or_else(|| {
            self.project_session
                .project()
                .midi_tracks
                .first()
                .map(|t| t.id)
        });

        if let Some(tid) = target_tid {
            self.midi_recorder.arm_track(Some(tid));
            let start_frame = self.engine_handle.playhead_frame();
            let tempo_map = self.project_session.project().tempo_map.clone();
            if self
                .midi_recorder
                .start_recording(start_frame, &tempo_map, self.active_sample_rate)
            {
                self.engine_handle.play();
                self.status_message = "MIDI Recording…".into();
            }
        }
    }

    // ── Import audio ──────────────────────────────────────────────────────────

    fn import_audio_file(&mut self, path: PathBuf, target_track_id: Option<TrackId>) {
        let temp_id = SourceId((1u64 << 63) | self.next_temp_source_seq);
        self.next_temp_source_seq += 1;

        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("audio")
            .to_owned();

        if let Err(e) =
            self.asset_manager
                .load_async(temp_id, path.clone(), Some(self.active_sample_rate))
        {
            self.status_message = format!("Failed to queue import: {e}");
            return;
        }

        let start_frame = self.engine_handle.playhead_frame();
        self.pending_imports.push(PendingImport {
            temp_id,
            name,
            path,
            target_track_id,
            start_frame,
        });
        self.status_message = "Decoding audio…".into();
    }

    fn poll_pending_imports(&mut self) {
        if self.pending_imports.is_empty() {
            return;
        }

        let mut remaining = Vec::new();
        let mut changed = false;

        let pending = std::mem::take(&mut self.pending_imports);
        for item in pending {
            match self.asset_manager.status(&item.temp_id) {
                Some(LoadStatus::Ready) => {
                    if let Some(audio) = self.asset_manager.get(&item.temp_id) {
                        self.finalize_import(&item, audio);
                        self.asset_manager.evict(&item.temp_id);
                        changed = true;
                    }
                }
                Some(LoadStatus::Failed(err)) => {
                    self.status_message = format!("Import failed ({}): {err}", item.name);
                    self.asset_manager.evict(&item.temp_id);
                }
                Some(LoadStatus::Pending | LoadStatus::Loading) => {
                    remaining.push(item);
                }
                None => {}
            }
        }

        self.pending_imports = remaining;
        if changed {
            self.rebuild_schedule();
        }
    }

    fn finalize_import(&mut self, item: &PendingImport, audio: Arc<DecodedAudio>) {
        let file_name = item
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.wav");
        let relative_path = PathBuf::from("Audio").join(file_name);

        let import_res = self.project_session.execute(ProjectCommand::ImportAudio {
            name: item.name.clone(),
            relative_path,
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            duration_frames: audio.duration_frames.max(1),
        });

        let source_id = match import_res {
            Ok(CommandOutcome::AudioImported(id)) => id,
            Ok(_) => return,
            Err(e) => {
                self.status_message = format!("Project import error: {e}");
                return;
            }
        };

        self.source_abs_paths.insert(source_id, item.path.clone());
        self.asset_manager.register(source_id, Arc::clone(&audio));

        let track_id = match item.target_track_id {
            Some(tid) if self.project_session.project().find_track(tid).is_some() => tid,
            _ => match self.project_session.execute(ProjectCommand::AddTrack {
                name: item.name.clone(),
            }) {
                Ok(CommandOutcome::TrackAdded(tid)) => tid,
                _ => return,
            },
        };

        if let Err(e) = self.project_session.execute(ProjectCommand::CreateClip {
            track_id,
            source_id,
            name: item.name.clone(),
            timeline_start_frame: item.start_frame,
            source_offset_frame: 0,
            duration_frames: audio.duration_frames.max(1),
        }) {
            self.status_message = format!("CreateClip error: {e}");
            return;
        }

        self.status_message = format!(
            "Imported '{}' ({:.2}s)",
            item.name,
            audio.duration_frames as f64 / audio.sample_rate.max(1) as f64
        );
    }

    // ── Save / Load Project ───────────────────────────────────────────────────

    fn save_project_dialog(&mut self) {
        let target_dir = if let Some(dir) = self.project_bundle_dir.clone() {
            Some(dir)
        } else {
            rfd::FileDialog::new()
                .set_title("Save Esperia Project Bundle Folder")
                .pick_folder()
        };

        if let Some(dir) = target_dir {
            // Copy all imported/recorded source files into `<bundle>/Audio/`
            // so reopening the saved project reproduces every recording & clip.
            for source in &self.project_session.project().sources {
                let dest = dir.join(&source.relative_path);
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Some(src_abs) = self.source_abs_paths.get(&source.id)
                    && src_abs.exists()
                    && src_abs != &dest
                {
                    let _ = std::fs::copy(src_abs, &dest);
                }
            }

            match save_project(&dir, self.project_session.project()) {
                Ok(()) => {
                    self.project_session.mark_saved();
                    self.project_bundle_dir = Some(dir.clone());
                    self.recorder.set_output_dir(dir.join("Audio"));
                    self.status_message = format!("Saved project to {}", dir.display());
                }
                Err(e) => {
                    self.status_message = format!("Save failed: {e}");
                }
            }
        }
    }

    fn load_project_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("Open Esperia Project Bundle Folder")
            .pick_folder()
        {
            match load_project(&dir) {
                Ok(report) => {
                    self.project_session = ProjectSession::new(report.project);
                    self.project_bundle_dir = Some(dir.clone());
                    self.recorder.set_output_dir(dir.join("Audio"));
                    self.source_abs_paths.clear();
                    self.selected_clip = None;

                    // Queue background decode for any sources whose files exist in bundle.
                    for source in &self.project_session.project().sources {
                        let abs = dir.join(&source.relative_path);
                        if abs.exists() {
                            self.source_abs_paths.insert(source.id, abs.clone());
                            let _ = self.asset_manager.load_async(
                                source.id,
                                abs,
                                Some(self.active_sample_rate),
                            );
                        }
                    }

                    self.rebuild_schedule();
                    self.status_message = format!("Loaded project from {}", dir.display());
                }
                Err(e) => {
                    self.status_message = format!("Load failed: {e}");
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// eframe::App impl
// ─────────────────────────────────────────────────────────────────────────────

impl eframe::App for EsperiaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.engine_handle.reclaim_schedules();
        let rejected = self
            .engine_handle
            .telemetry
            .command_queue_rejections
            .load(Ordering::Acquire);
        if rejected > self.last_command_queue_rejections {
            self.last_command_queue_rejections = rejected;
            self.status_message =
                format!("Audio command queue is busy; {rejected} command(s) rejected");
        }
        let rejected = self
            .engine_handle
            .telemetry
            .schedule_update_rejections
            .load(Ordering::Acquire);
        if rejected > self.last_schedule_update_rejections {
            self.last_schedule_update_rejections = rejected;
            self.status_message =
                format!("Audio schedule update busy; {rejected} update(s) rejected");
        }

        let ctx = ui.ctx().clone();
        ctx.request_repaint();

        self.poll_pending_imports();
        self.poll_midi_input();

        #[cfg(target_os = "macos")]
        if let Some(menu) = &self.native_menu {
            let actions: Vec<_> = menu.poll_actions().collect();
            for action in actions {
                match action {
                    macos_menu::Action::ImportAudio => {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "wave"])
                            .pick_file()
                        {
                            self.import_audio_file(path, None);
                        }
                    }
                    macos_menu::Action::SaveProject => self.save_project_dialog(),
                    macos_menu::Action::OpenProject => self.load_project_dialog(),
                    macos_menu::Action::Quit => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    macos_menu::Action::Undo => {
                        if self.project_session.can_undo() {
                            let _ = self.project_session.undo();
                            self.rebuild_schedule();
                        }
                    }
                    macos_menu::Action::Redo => {
                        if self.project_session.can_redo() {
                            let _ = self.project_session.redo();
                            self.rebuild_schedule();
                        }
                    }
                    macos_menu::Action::AddTrack => {
                        let idx = self.project_session.project().tracks.len() + 1;
                        let _ = self.project_session.execute(ProjectCommand::AddTrack {
                            name: format!("Track {idx}"),
                        });
                    }
                    macos_menu::Action::ToggleDevicePanel => {
                        self.show_device_panel = !self.show_device_panel;
                    }
                    macos_menu::Action::ToggleMixer => {
                        self.show_mixer_panel = !self.show_mixer_panel;
                    }
                    macos_menu::Action::TogglePianoRoll => {
                        self.show_piano_roll = !self.show_piano_roll;
                    }
                }
            }
        }

        if let Some(err) = self.stream_errors.take() {
            self.stream_active.store(false, Ordering::Release);
            self.status_message = format!("CPAL output error: {err}");
        }
        if let Some(err) = self.input_stream_errors.take() {
            self.input_stream_active.store(false, Ordering::Release);
            self.status_message = format!("CPAL input error: {err}");
        }

        // ── Menu bar ─────────────────────────────────────────────────────────
        #[cfg(not(target_os = "macos"))]
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Import Audio…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "wave"])
                            .pick_file()
                        {
                            self.import_audio_file(path, None);
                        }
                    }
                    if ui.button("Save Project…").clicked() {
                        ui.close();
                        self.save_project_dialog();
                    }
                    if ui.button("Open Project…").clicked() {
                        ui.close();
                        self.load_project_dialog();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.project_session.can_undo(), egui::Button::new("Undo"))
                        .clicked()
                    {
                        let _ = self.project_session.undo();
                        self.rebuild_schedule();
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.project_session.can_redo(), egui::Button::new("Redo"))
                        .clicked()
                    {
                        let _ = self.project_session.redo();
                        self.rebuild_schedule();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Add Track").clicked() {
                        let idx = self.project_session.project().tracks.len() + 1;
                        let _ = self.project_session.execute(ProjectCommand::AddTrack {
                            name: format!("Track {idx}"),
                        });
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Toggle Device & Tone Panel").clicked() {
                        self.show_device_panel = !self.show_device_panel;
                        ui.close();
                    }
                    if ui.button("🎚 Toggle Mixer").clicked() {
                        self.show_mixer_panel = !self.show_mixer_panel;
                        ui.close();
                    }
                    if ui.button("🎹 Toggle Piano Roll").clicked() {
                        self.show_piano_roll = !self.show_piano_roll;
                        ui.close();
                    }
                });
            });
        });

        // ── Status bar ────────────────────────────────────────────────────────
        egui::Panel::bottom("status_bar")
            .exact_size(22.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.weak(&self.status_message);
                });
            });

        // ── Transport bar (Logic Pro brushed-slate Control Bar) ───────────────
        egui::Panel::top("transport")
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(76, 76, 82))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 42, 45)))
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                self.draw_transport(ui);
            });

        // ── Device panel (optional sidebar) ───────────────────────────────────
        if self.show_device_panel {
            egui::Panel::right("device_panel")
                .min_size(250.0)
                .show(ui, |ui| {
                    self.draw_device_panel(ui);
                });
        }

        // ── Mixer panel (collapsible bottom strip) ────────────────────────────
        if self.show_mixer_panel {
            egui::Panel::bottom("mixer_panel")
                .resizable(true)
                .default_size(180.0)
                .min_size(140.0)
                .show(ui, |ui| {
                    self.draw_mixer_panel(ui);
                });
        }

        // ── Piano Roll panel (collapsible & resizable bottom editor) ──────────
        if self.show_piano_roll {
            let max_h = (ui.available_height() - 120.0).max(220.0);
            self.piano_roll_height = self.piano_roll_height.clamp(180.0, max_h);
            egui::Panel::bottom("piano_roll_panel")
                .resizable(false)
                .exact_size(self.piano_roll_height)
                .show(ui, |ui| {
                    self.draw_piano_roll_panel(ui, max_h);
                });
        }

        // ── Main timeline area ────────────────────────────────────────────────
        egui::CentralPanel::default().show(ui, |ui| {
            self.draw_timeline(ui);
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UI drawing helpers
// ─────────────────────────────────────────────────────────────────────────────

impl EsperiaApp {
    fn draw_transport(&mut self, ui: &mut egui::Ui) {
        let ph = self.engine_handle.playhead_frame();
        let sr_u32 = self.active_sample_rate.max(1);
        let tempo_map = &self.project_session.project().tempo_map;
        let ph_tick = tempo_map.sample_to_tick(ph, sr_u32);
        let (bar, beat, _) = esperia_midi::TempoMap::tick_to_bar_beat_tick(
            ph_tick,
            esperia_midi::TimeSignature::default(),
        );
        let mut tempo_bpm = self.project_session.project().timeline.tempo_bpm;

        ui.horizontal(|ui| {
            // ── Left: Panel & Track creation shortcuts ────────────────────────
            let pr_btn = egui::Button::new("🎹 Piano Roll").fill(if self.show_piano_roll {
                egui::Color32::from_rgb(35, 115, 210)
            } else {
                egui::Color32::from_gray(58)
            });
            if ui.add(pr_btn).clicked() {
                self.show_piano_roll = !self.show_piano_roll;
            }

            let mix_btn = egui::Button::new("🎚 Mixer").fill(if self.show_mixer_panel {
                egui::Color32::from_rgb(35, 115, 210)
            } else {
                egui::Color32::from_gray(58)
            });
            if ui.add(mix_btn).clicked() {
                self.show_mixer_panel = !self.show_mixer_panel;
                if self.show_mixer_panel {
                    self.rebuild_mixer();
                }
            }

            let dev_btn = egui::Button::new("⚙ Devices").fill(if self.show_device_panel {
                egui::Color32::from_rgb(35, 115, 210)
            } else {
                egui::Color32::from_gray(58)
            });
            if ui.add(dev_btn).clicked() {
                self.show_device_panel = !self.show_device_panel;
            }

            ui.separator();

            // ── Transport controls ────────────────────────────────────────────
            if ui.button("⏮").on_hover_text("Return to Bar 1").clicked() {
                self.engine_handle.seek(0);
            }

            let transport_state = self.engine_handle.telemetry.transport_state();
            let play_label = if transport_state == 1 { "⏸" } else { "▶" };
            let play_btn = egui::Button::new(play_label).fill(if transport_state == 1 {
                egui::Color32::from_rgb(45, 150, 75)
            } else {
                egui::Color32::from_gray(62)
            });
            if ui.add(play_btn).clicked() {
                self.engine_handle.toggle_play();
            }

            if ui.button("⏹").on_hover_text("Stop").clicked() {
                if matches!(self.recorder.status(), RecordingStatus::Recording { .. }) {
                    self.toggle_recording();
                } else if self.midi_recorder.is_recording() {
                    self.toggle_midi_recording();
                } else {
                    self.engine_handle.stop();
                }
            }

            let is_recording = matches!(self.recorder.status(), RecordingStatus::Recording { .. })
                || self.midi_recorder.is_recording();
            let rec_btn = egui::Button::new("⏺").fill(if is_recording {
                egui::Color32::from_rgb(205, 45, 45)
            } else {
                egui::Color32::from_gray(62)
            });
            if ui
                .add(rec_btn)
                .on_hover_text("Record MIDI (when MIDI track selected) or Audio")
                .clicked()
            {
                if self.active_midi_track_id.is_some()
                    && !self.project_session.project().midi_tracks.is_empty()
                {
                    self.toggle_midi_recording();
                } else {
                    self.toggle_recording();
                }
            }

            let loop_btn = egui::Button::new("⟳").fill(if self.loop_enabled {
                egui::Color32::from_rgb(205, 155, 35)
            } else {
                egui::Color32::from_gray(62)
            });
            if ui.add(loop_btn).on_hover_text("Cycle / Loop").clicked() {
                self.loop_enabled = !self.loop_enabled;
                self.engine_handle.enable_loop(self.loop_enabled);
                if self.loop_enabled {
                    self.engine_handle
                        .set_loop_region(self.loop_start_frame, self.loop_end_frame);
                }
            }

            ui.separator();

            // ── Logic Pro Style Center LCD Display ────────────────────────────
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(28, 30, 34))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(68)))
                .corner_radius(5.0)
                .inner_margin(egui::Margin::symmetric(10, 3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{bar}.{beat}"))
                                    .monospace()
                                    .size(16.0)
                                    .color(egui::Color32::from_gray(240)),
                            );
                            ui.label(
                                egui::RichText::new("BAR   BEAT")
                                    .size(8.5)
                                    .color(egui::Color32::from_gray(145)),
                            );
                        });
                        ui.separator();
                        ui.vertical(|ui| {
                            if ui
                                .add(
                                    egui::DragValue::new(&mut tempo_bpm)
                                        .range(40.0..=240.0)
                                        .speed(0.5)
                                        .max_decimals(0),
                                )
                                .changed()
                            {
                                let proj = self.project_session.project_mut();
                                proj.timeline.tempo_bpm = tempo_bpm;
                                proj.tempo_map = esperia_midi::TempoMap::constant(tempo_bpm);
                                self.rebuild_midi_schedule();
                            }
                            ui.label(
                                egui::RichText::new("TEMPO")
                                    .size(8.5)
                                    .color(egui::Color32::from_gray(145)),
                            );
                        });
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("4/4")
                                    .monospace()
                                    .size(13.0)
                                    .color(egui::Color32::from_gray(225)),
                            );
                            ui.label(
                                egui::RichText::new("SIG")
                                    .size(8.5)
                                    .color(egui::Color32::from_gray(145)),
                            );
                        });
                    });
                });

            ui.separator();

            // ── Right: Track / Import / Save / DSP ────────────────────────────
            if ui.button("+ Audio").clicked() {
                let idx = self.project_session.project().tracks.len() + 1;
                let _ = self.project_session.execute(ProjectCommand::AddTrack {
                    name: format!("Track {idx}"),
                });
            }
            if ui.button("+ MIDI").clicked() {
                self.add_midi_track();
            }
            if ui.button("📂 Import").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "wave"])
                    .pick_file()
            {
                self.import_audio_file(path, None);
            }
            if ui.button("💾 Save").clicked() {
                self.save_project_dialog();
            }

            if self.master_volume.ui(ui).changed() {
                self.rebuild_mixer();
            }

            let mon_btn = egui::Button::new("🎧").fill(if self.input_monitoring {
                egui::Color32::from_rgb(55, 130, 190)
            } else {
                egui::Color32::from_gray(58)
            });
            if ui.add(mon_btn).on_hover_text("Input Monitoring").clicked() {
                self.input_monitoring = !self.input_monitoring;
                self.recorder.set_monitoring_enabled(self.input_monitoring);
                self.engine_handle
                    .set_input_monitoring(self.input_monitoring);
            }

            let bf = self.buffer_frames_hint.load(Ordering::Relaxed);
            let load = self
                .engine_handle
                .telemetry
                .dsp_load(bf, self.active_sample_rate as f32);
            let load_color = if load > 0.8 {
                egui::Color32::RED
            } else if load > 0.5 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::from_rgb(80, 220, 100)
            };
            ui.colored_label(load_color, format!("DSP {:.0}%", load * 100.0));
        });
    }

    fn draw_piano_roll_panel(&mut self, ui: &mut egui::Ui, max_h: f32) {
        // ── Draggable top splitter bar (always works to expand/shrink) ────────
        let (split_rect, split_resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(100.0), 6.0),
            egui::Sense::drag(),
        );
        if split_resp.hovered() || split_resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if split_resp.dragged() {
            self.piano_roll_height =
                (self.piano_roll_height - split_resp.drag_delta().y).clamp(180.0, max_h);
        }
        ui.painter().rect_filled(
            split_rect,
            0.0,
            if split_resp.hovered() || split_resp.dragged() {
                egui::Color32::from_rgb(70, 145, 225)
            } else {
                egui::Color32::from_gray(55)
            },
        );

        if self.project_session.project().midi_tracks.is_empty() {
            ui.horizontal(|ui| {
                ui.label("No MIDI tracks yet.");
                if ui.button("🎹 Create MIDI Track").clicked() {
                    self.add_midi_track();
                }
            });
            return;
        }

        // Track / clip / instrument / size selector bar
        let track_options: Vec<(u64, String, InstrumentKind)> = self
            .project_session
            .project()
            .midi_tracks
            .iter()
            .map(|t| (t.id, t.name.clone(), t.instrument))
            .collect();

        if self.active_midi_track_id.is_none() {
            self.active_midi_track_id = track_options.first().map(|(id, _, _)| *id);
        }

        let mut schedule_dirty = false;
        ui.horizontal(|ui| {
            let active_tid = self.active_midi_track_id.unwrap_or(0);
            let active_name = track_options
                .iter()
                .find(|(id, _, _)| *id == active_tid)
                .map_or("Select Track", |(_, n, _)| n.as_str());
            egui::ComboBox::from_id_salt("pr_track_sel")
                .selected_text(active_name)
                .show_ui(ui, |ui| {
                    for (id, name, _) in &track_options {
                        if ui.selectable_label(*id == active_tid, name).clicked() {
                            self.active_midi_track_id = Some(*id);
                            self.active_midi_clip_id = self
                                .project_session
                                .project()
                                .midi_tracks
                                .iter()
                                .find(|t| t.id == *id)
                                .and_then(|t| t.clips.first().map(|c| c.id));
                        }
                    }
                });

            if let Some((_, _, current_inst)) =
                track_options.iter().find(|(id, _, _)| *id == active_tid)
            {
                let mut selected_inst = *current_inst;
                egui::ComboBox::from_id_salt("pr_inst_sel")
                    .selected_text(selected_inst.display_name())
                    .show_ui(ui, |ui| {
                        for inst in InstrumentKind::ALL {
                            if ui
                                .selectable_value(&mut selected_inst, inst, inst.display_name())
                                .changed()
                            {
                                let proj = self.project_session.project_mut();
                                if let Some(track) =
                                    proj.midi_tracks.iter_mut().find(|t| t.id == active_tid)
                                {
                                    track.instrument = selected_inst;
                                    track.program = match selected_inst {
                                        InstrumentKind::ElectricPiano => 0,
                                        InstrumentKind::SineSynth => 1,
                                        InstrumentKind::TriangleSynth => 2,
                                        InstrumentKind::SawSynth => 3,
                                    };
                                    self.engine_handle
                                        .send_live_midi(MidiMessage::ProgramChange {
                                            channel: track.channel,
                                            program: track.program,
                                        });
                                    schedule_dirty = true;
                                }
                            }
                        }
                    });
            }

            ui.separator();
            let pr_on = !self.piano_roll_state.show_velocity_lane;
            if ui
                .add(egui::Button::new("Piano Roll").fill(if pr_on {
                    egui::Color32::from_rgb(35, 115, 215)
                } else {
                    egui::Color32::from_gray(64)
                }))
                .clicked()
            {
                self.piano_roll_state.show_velocity_lane = false;
            }
            if ui
                .add(egui::Button::new("Piano Roll + Velocity").fill(if !pr_on {
                    egui::Color32::from_rgb(35, 115, 215)
                } else {
                    egui::Color32::from_gray(64)
                }))
                .clicked()
            {
                self.piano_roll_state.show_velocity_lane = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let is_expanded = self.piano_roll_height >= max_h - 40.0;
                if ui
                    .button(if is_expanded {
                        "⤡ Restore"
                    } else {
                        "⤢ Expand"
                    })
                    .clicked()
                {
                    self.piano_roll_height = if is_expanded { 300.0 } else { max_h };
                }
                ui.add(
                    egui::Slider::new(&mut self.piano_roll_height, 180.0..=max_h).show_value(false),
                );
                ui.weak("↕");
                ui.separator();
                ui.add(
                    egui::Slider::new(&mut self.piano_roll_state.px_per_tick, 0.015..=0.25)
                        .show_value(false),
                );
                ui.weak("↔");
            });
        });

        let active_tid = self.active_midi_track_id.unwrap_or(0);
        if self.active_midi_clip_id.is_none() {
            self.active_midi_clip_id = self
                .project_session
                .project()
                .midi_tracks
                .iter()
                .find(|t| t.id == active_tid)
                .and_then(|t| t.clips.first().map(|c| c.id));
        }

        let mut new_pattern_start: Option<u64> = None;
        let mut duplicate_clip: Option<MidiClip> = None;
        let mut seek_frame: Option<u64> = None;

        if let Some(active_cid) = self.active_midi_clip_id {
            let engine_handle = self.engine_handle.clone();
            let playhead_frame = engine_handle.playhead_frame();
            let sr = self.active_sample_rate;
            let pr_state = &mut self.piano_roll_state;
            let proj = self.project_session.project_mut();
            let tempo_map = proj.tempo_map.clone();
            let playhead_tick = tempo_map.sample_to_tick(playhead_frame, sr);
            if let Some(track) = proj.midi_tracks.iter_mut().find(|t| t.id == active_tid) {
                let tname = track.name.clone();
                let ch = track.channel;
                if let Some(clip) = track.find_clip_mut(active_cid) {
                    let outcome =
                        pr_state.draw(ui, clip, &tname, ch, playhead_tick, &engine_handle);
                    if outcome.clip_modified {
                        schedule_dirty = true;
                    }
                    if let Some(st) = outcome.seek_tick {
                        seek_frame = Some(tempo_map.tick_to_sample(st, sr));
                    }
                    if outcome.create_new_pattern {
                        new_pattern_start = Some(clip.start_tick.saturating_add(clip.length_ticks));
                    }
                    if outcome.duplicate_pattern {
                        let mut dup = clip.clone();
                        dup.start_tick = clip.start_tick.saturating_add(clip.length_ticks);
                        dup.name = format!("{} Copy", clip.name);
                        duplicate_clip = Some(dup);
                    }
                }
            }
        }

        if let Some(st) = new_pattern_start {
            self.create_midi_region(active_tid, st);
        }
        if let Some(mut dup) = duplicate_clip {
            let proj = self.project_session.project_mut();
            dup.id = proj.allocate_id();
            let new_cid = dup.id;
            if let Some(track) = proj.midi_tracks.iter_mut().find(|t| t.id == active_tid) {
                track.clips.push(dup);
                self.active_midi_clip_id = Some(new_cid);
                schedule_dirty = true;
            }
        }
        if let Some(f) = seek_frame {
            self.engine_handle.seek(f);
        }
        if schedule_dirty {
            self.rebuild_midi_schedule();
        }
    }

    // ── Mixer panel ───────────────────────────────────────────────────────────

    /// Bottom mixer strip: one channel strip per track + master.
    fn draw_mixer_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Mixer");
        ui.add_space(4.0);
        ui.separator();

        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                // Collect track info we need (name, id, gain_db, muted, soloed)
                let tracks: Vec<(u64, String, f32, bool, bool)> = self
                    .project_session
                    .project()
                    .tracks
                    .iter()
                    .map(|t| (t.id.0, t.name.clone(), t.gain_db, t.muted, t.soloed))
                    .collect();

                let any_solo = tracks.iter().any(|(_, _, _, _, s)| *s);

                for (tid, name, gain_db, muted, soloed) in &tracks {
                    // Read live meter for this channel (lock-free)
                    let reading = self
                        .mixer_meters
                        .as_ref()
                        .and_then(|m| m.channels.get(tid))
                        .map(|a| a.read());

                    let peak = reading
                        .as_ref()
                        .map(|r| r.peak_left.max(r.peak_right))
                        .unwrap_or(0.0);
                    let rms = reading
                        .as_ref()
                        .map(|r| r.rms_left.max(r.rms_right))
                        .unwrap_or(0.0);
                    let clipped = reading.as_ref().map(|r| r.clipped).unwrap_or(false);

                    ui.group(|ui| {
                        ui.set_min_width(72.0);
                        ui.vertical(|ui| {
                            // Track name
                            ui.label(egui::RichText::new(name.as_str()).small());

                            // Peak / RMS meter bar (vertical, 80 px tall)
                            let (rect, _resp) = ui
                                .allocate_exact_size(egui::vec2(16.0, 80.0), egui::Sense::hover());
                            let painter = ui.painter();
                            // Background
                            painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                            // RMS bar (green/yellow/red)
                            let rms_h = rms.min(1.0) * rect.height();
                            let rms_rect = egui::Rect::from_min_size(
                                egui::pos2(rect.left(), rect.bottom() - rms_h),
                                egui::vec2(rect.width(), rms_h),
                            );
                            let rms_color = if rms > 0.9 {
                                egui::Color32::RED
                            } else if rms > 0.7 {
                                egui::Color32::YELLOW
                            } else {
                                egui::Color32::from_rgb(80, 200, 100)
                            };
                            painter.rect_filled(rms_rect, 0.0, rms_color);
                            // Peak tick
                            if peak > 0.01 {
                                let pk_y = rect.bottom() - (peak.min(1.0) * rect.height());
                                painter.hline(
                                    rect.left()..=rect.right(),
                                    pk_y,
                                    egui::Stroke::new(1.5, egui::Color32::WHITE),
                                );
                            }
                            // Clip indicator (red bar at top)
                            if clipped {
                                painter.rect_filled(
                                    egui::Rect::from_min_size(
                                        rect.min,
                                        egui::vec2(rect.width(), 6.0),
                                    ),
                                    0.0,
                                    egui::Color32::RED,
                                );
                            }

                            // Gain (dB) drag
                            let mut g = *gain_db;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut g)
                                        .speed(0.5)
                                        .range(-60.0..=12.0)
                                        .suffix(" dB"),
                                )
                                .changed()
                            {
                                let _ =
                                    self.project_session
                                        .execute(ProjectCommand::ChangeTrackGain {
                                            track_id: esperia_project_model::track::TrackId(*tid),
                                            gain_db: g,
                                        });
                                self.rebuild_schedule();
                                self.rebuild_mixer();
                            }

                            // Mute / Solo buttons
                            ui.horizontal(|ui| {
                                let mute_col = if *muted {
                                    egui::Color32::from_rgb(200, 150, 30)
                                } else {
                                    egui::Color32::from_gray(60)
                                };
                                if ui
                                    .add(
                                        egui::Button::new("M")
                                            .fill(mute_col)
                                            .min_size(egui::vec2(20.0, 18.0)),
                                    )
                                    .clicked()
                                {
                                    let _ = self.project_session.execute(
                                        ProjectCommand::SetTrackMute {
                                            track_id: esperia_project_model::track::TrackId(*tid),
                                            muted: !muted,
                                        },
                                    );
                                    self.rebuild_schedule();
                                    self.rebuild_mixer();
                                }
                                let solo_col = if *soloed {
                                    egui::Color32::from_rgb(220, 200, 30)
                                } else {
                                    egui::Color32::from_gray(60)
                                };
                                if ui
                                    .add(
                                        egui::Button::new("S")
                                            .fill(solo_col)
                                            .min_size(egui::vec2(20.0, 18.0)),
                                    )
                                    .clicked()
                                {
                                    let _ = self.project_session.execute(
                                        ProjectCommand::SetTrackSolo {
                                            track_id: esperia_project_model::track::TrackId(*tid),
                                            soloed: !soloed,
                                        },
                                    );
                                    self.rebuild_schedule();
                                    self.rebuild_mixer();
                                }
                            });

                            // Audibility dot
                            let audible = if any_solo { *soloed && !muted } else { !muted };
                            let dot_color = if audible {
                                egui::Color32::from_rgb(80, 200, 100)
                            } else {
                                egui::Color32::from_gray(60)
                            };
                            ui.colored_label(dot_color, "●");
                        });
                    });
                }

                // ── Master channel strip ──────────────────────────────────
                ui.separator();
                ui.group(|ui| {
                    ui.set_min_width(80.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Master").strong().small());

                        let m_reading = self.mixer_meters.as_ref().map(|m| m.master.read());
                        let m_peak = m_reading
                            .as_ref()
                            .map(|r| r.peak_left.max(r.peak_right))
                            .unwrap_or(0.0);
                        let m_rms = m_reading
                            .as_ref()
                            .map(|r| r.rms_left.max(r.rms_right))
                            .unwrap_or(0.0);

                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(20.0, 80.0), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));
                        let rms_h = m_rms.min(1.0) * rect.height();
                        let rms_rect = egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.bottom() - rms_h),
                            egui::vec2(rect.width(), rms_h),
                        );
                        let rms_color = if m_rms > 0.9 {
                            egui::Color32::RED
                        } else if m_rms > 0.7 {
                            egui::Color32::YELLOW
                        } else {
                            egui::Color32::from_rgb(80, 200, 100)
                        };
                        painter.rect_filled(rms_rect, 0.0, rms_color);
                        if m_peak > 0.01 {
                            let pk_y = rect.bottom() - (m_peak.min(1.0) * rect.height());
                            painter.hline(
                                rect.left()..=rect.right(),
                                pk_y,
                                egui::Stroke::new(1.5, egui::Color32::WHITE),
                            );
                        }

                        ui.label(format!("pk {m_peak:.2}"));
                        ui.label(format!("rms {m_rms:.2}"));
                    });
                });
            });
        });
    }

    fn draw_device_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Audio Devices");
        ui.add_space(4.0);

        let device_names: Vec<String> =
            self.output_devices.iter().map(|d| d.name.clone()).collect();
        let mut new_device: Option<String> = None;

        egui::ComboBox::from_label("Output Device")
            .selected_text(&self.selected_device_name)
            .show_ui(ui, |ui| {
                for name in &device_names {
                    if ui
                        .selectable_label(*name == self.selected_device_name, name)
                        .clicked()
                    {
                        new_device = Some(name.clone());
                    }
                }
            });

        if let Some(name) = new_device {
            self.switch_device(&name);
        }

        ui.separator();
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
                });
            }
        }

        ui.separator();
        ui.heading("Audio Input & Recording");
        ui.add_space(4.0);

        let in_device_names: Vec<String> =
            self.input_devices.iter().map(|d| d.name.clone()).collect();
        let mut new_in_device: Option<String> = None;

        egui::ComboBox::from_label("Input Device")
            .selected_text(&self.selected_input_device_name)
            .show_ui(ui, |ui| {
                for name in &in_device_names {
                    if ui
                        .selectable_label(*name == self.selected_input_device_name, name)
                        .clicked()
                    {
                        new_in_device = Some(name.clone());
                    }
                }
            });

        if let Some(name) = new_in_device {
            self.switch_input_device(&name);
        }

        let max_ch = self.input_channels.max(1);
        let mut ch_changed = false;
        egui::ComboBox::from_label("Input Channel")
            .selected_text(format!("Ch {} (Mono)", self.selected_input_channel + 1))
            .show_ui(ui, |ui| {
                for c in 0..max_ch {
                    if ui
                        .selectable_label(
                            self.selected_input_channel == c,
                            format!("Ch {} (Mono)", c + 1),
                        )
                        .clicked()
                    {
                        self.selected_input_channel = c;
                        ch_changed = true;
                    }
                }
            });
        if ch_changed {
            self.recorder
                .set_channel_selection(InputChannelSelection::Mono(self.selected_input_channel));
            self.switch_input_device(&self.selected_input_device_name.clone());
        }

        if ui
            .checkbox(&mut self.punch_enabled, "Punch-In at Loop Region")
            .changed()
        {
            self.recorder.set_punch_region(PunchRegion {
                enabled: self.punch_enabled,
                start_frame: self.loop_start_frame,
                end_frame: self.loop_end_frame,
            });
        }

        let rec_tel = self.recorder.telemetry();
        ui.label(format!(
            "Input: {} Hz, {}ch | Peak: {:.3}",
            self.input_sample_rate,
            self.input_channels,
            rec_tel.input_peak()
        ));
        if !self.recorder.takes().is_empty() {
            ui.label(format!("Recorded takes: {}", self.recorder.takes().len()));
        }

        ui.separator();
        ui.heading("MIDI Input");
        ui.add_space(4.0);

        let selected_midi_label = self
            .selected_midi_port
            .and_then(|idx| self.midi_ports.get(idx))
            .map_or("None".to_string(), |p| p.name.clone());
        let mut connect_idx: Option<usize> = None;
        egui::ComboBox::from_label("MIDI Keyboard")
            .selected_text(&selected_midi_label)
            .show_ui(ui, |ui| {
                for port in &self.midi_ports {
                    if ui
                        .selectable_label(self.selected_midi_port == Some(port.index), &port.name)
                        .clicked()
                    {
                        connect_idx = Some(port.index);
                    }
                }
            });
        if ui.button("Refresh MIDI Ports").clicked() {
            self.midi_ports = MidiInputManager::enumerate_input_ports().unwrap_or_default();
        }
        if let Some(idx) = connect_idx {
            self.connect_midi_port(idx);
        }

        ui.separator();
        ui.heading("Tone Generator (Fallback)");
        ui.add_space(4.0);

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
    }

    fn draw_timeline(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();

        // ── Zoom / track-add / clip actions bar (Logic Pro Tracks sub-bar) ──
        ui.horizontal(|ui| {
            if ui.button("➕ Audio").clicked() {
                let idx = self.project_session.project().tracks.len() + 1;
                let _ = self.project_session.execute(ProjectCommand::AddTrack {
                    name: format!("Track {idx}"),
                });
            }
            if ui.button("➕ MIDI").clicked() {
                self.add_midi_track();
            }
            if let Some(sel_id) = self.selected_clip {
                ui.separator();
                if ui.button("🗑 Delete Clip").clicked() {
                    let _ = self
                        .project_session
                        .execute(ProjectCommand::DeleteClip { clip_id: sel_id });
                    self.selected_clip = None;
                    self.rebuild_schedule();
                }
            }
            if let (Some(tid), Some(cid)) = (self.active_midi_track_id, self.active_midi_clip_id) {
                ui.separator();
                if ui.button("🗑 Delete Region").clicked() {
                    let proj = self.project_session.project_mut();
                    if let Some(t) = proj.midi_tracks.iter_mut().find(|t| t.id == tid) {
                        t.clips.retain(|c| c.id != cid);
                    }
                    self.active_midi_clip_id = None;
                    self.rebuild_midi_schedule();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Fit").clicked() {
                    self.timeline_zoom = DEFAULT_ZOOM;
                }
                ui.add(
                    egui::Slider::new(&mut self.timeline_zoom, MIN_ZOOM..=0.05).show_value(false),
                );
                ui.weak("↔");
            });
        });

        ui.separator();

        // ── Scrollable timeline region ────────────────────────────────────────
        let scroll_area = egui::ScrollArea::both()
            .id_salt("timeline_scroll")
            .max_height((available.y - 40.0).max(120.0));

        let mut pending_commands: Vec<ProjectCommand> = Vec::new();
        let mut click_seek_frame: Option<u64> = None;
        let mut new_midi_region_req: Option<(u64, u64)> = None;
        let mut midi_schedule_dirty = false;
        let mut midi_track_toggles: Vec<(u64, bool, bool, bool)> = Vec::new(); // (id, mute, solo, arm)
        let mut midi_track_gains: Vec<(u64, f32)> = Vec::new();

        scroll_area.show(ui, |ui| {
            let sr = self.active_sample_rate.max(1) as f64;
            let sr_u32 = self.active_sample_rate.max(1);
            let tracks = self.project_session.project().tracks.clone();
            let midi_tracks = self.project_session.project().midi_tracks.clone();
            let tempo_map = self.project_session.project().tempo_map.clone();
            let num_audio_tracks = tracks.len();
            let num_tracks = num_audio_tracks + midi_tracks.len();

            let total_frames = (5.0 * 60.0 * sr) as u64;
            let content_width =
                (total_frames as f32 * self.timeline_zoom + TRACK_HEADER_WIDTH).max(available.x);
            let content_height =
                (RULER_HEIGHT + num_tracks as f32 * TRACK_HEIGHT + 40.0).max(available.y - 60.0);

            let (response, painter) = ui.allocate_painter(
                egui::vec2(content_width, content_height),
                egui::Sense::click_and_drag(),
            );

            let rect = response.rect;
            let lane_x_start = rect.left() + TRACK_HEADER_WIDTH;

            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 30, 32));
            // Track-header column background
            painter.rect_filled(
                egui::Rect::from_min_max(
                    rect.min,
                    egui::pos2(lane_x_start, rect.bottom()),
                ),
                0.0,
                egui::Color32::from_rgb(44, 44, 46),
            );
            painter.vline(
                lane_x_start,
                rect.top()..=rect.bottom(),
                egui::Stroke::new(1.0, egui::Color32::from_rgb(24, 24, 26)),
            );

            // ── Time ruler (Bars & Beats synced with LCD & Piano Roll) ──────
            let ruler_rect =
                egui::Rect::from_min_size(rect.min, egui::vec2(content_width, RULER_HEIGHT));
            painter.rect_filled(ruler_rect, 0.0, egui::Color32::from_rgb(40, 40, 42));

            let frames_per_px = 1.0 / self.timeline_zoom;
            let bar_ticks = PPQN * 4;
            let samples_per_bar = tempo_map.tick_to_sample(bar_ticks, sr_u32).max(1) as f32;
            let px_per_bar = samples_per_bar * self.timeline_zoom;
            let bar_step = if px_per_bar > 52.0 {
                1u64
            } else if px_per_bar > 24.0 {
                2
            } else if px_per_bar > 10.0 {
                4
            } else {
                8
            };
            let max_tick = tempo_map.sample_to_tick(total_frames, sr_u32);
            let mut tick_cursor = 0u64;
            let mut bar_num = 1u64;
            while tick_cursor <= max_tick {
                let mf = tempo_map.tick_to_sample(tick_cursor, sr_u32);
                let x = lane_x_start + mf as f32 * self.timeline_zoom;
                if bar_num.saturating_sub(1).is_multiple_of(bar_step) {
                    painter.vline(
                        x,
                        (rect.top() + RULER_HEIGHT)..=rect.bottom(),
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(44, 44, 48)),
                    );
                    painter.line_segment(
                        [
                            egui::pos2(x, rect.top() + RULER_HEIGHT - 10.0),
                            egui::pos2(x, rect.top() + RULER_HEIGHT),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(145)),
                    );
                    painter.text(
                        egui::pos2(x + 3.0, rect.top() + 3.0),
                        egui::Align2::LEFT_TOP,
                        format!("{bar_num}"),
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_gray(195),
                    );
                }
                // Beat sub-ticks when zoomed in
                if bar_step == 1 && px_per_bar > 64.0 {
                    for b in 1..4u64 {
                        let bf = tempo_map.tick_to_sample(tick_cursor + b * PPQN, sr_u32);
                        let bx = lane_x_start + bf as f32 * self.timeline_zoom;
                        painter.line_segment(
                            [
                                egui::pos2(bx, rect.top() + RULER_HEIGHT - 5.0),
                                egui::pos2(bx, rect.top() + RULER_HEIGHT),
                            ],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(95)),
                        );
                    }
                }
                tick_cursor = tick_cursor.saturating_add(bar_ticks);
                bar_num += 1;
            }

            // ── Loop region shading ───────────────────────────────────────────
            if self.loop_enabled && self.loop_end_frame > self.loop_start_frame {
                let lx0 = lane_x_start + self.loop_start_frame as f32 * self.timeline_zoom;
                let lx1 = lane_x_start + self.loop_end_frame as f32 * self.timeline_zoom;
                painter.rect_filled(
                    egui::Rect::from_x_y_ranges(
                        lx0..=lx1,
                        (rect.top() + RULER_HEIGHT)..=rect.bottom(),
                    ),
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(80, 160, 255, 30),
                );
            }

            // ── Track lanes & interactive headers ─────────────────────────────
            for (track_idx, track) in tracks.iter().enumerate() {
                let lane_top = rect.top() + RULER_HEIGHT + track_idx as f32 * TRACK_HEIGHT;
                let lane_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), lane_top),
                    egui::vec2(content_width, TRACK_HEIGHT),
                );

                let lane_bg = if track_idx % 2 == 0 {
                    egui::Color32::from_gray(38)
                } else {
                    egui::Color32::from_gray(33)
                };
                painter.rect_filled(lane_rect, 0.0, lane_bg);

                // Track header background (Logic Pro style card with blue accent stripe).
                let header_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), lane_top + 1.0),
                    egui::vec2(TRACK_HEADER_WIDTH - 2.0, TRACK_HEIGHT - 2.0),
                );
                painter.rect_filled(header_rect, 2.0, egui::Color32::from_rgb(84, 84, 90));
                painter.rect_filled(
                    egui::Rect::from_min_size(header_rect.min, egui::vec2(4.0, header_rect.height())),
                    0.0,
                    egui::Color32::from_rgb(55, 135, 225),
                );

                // Interactive track header widgets using ui.put / child Ui.
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(
                        egui::Rect::from_min_max(
                            egui::pos2(header_rect.left() + 8.0, header_rect.top() + 5.0),
                            egui::pos2(header_rect.right() - 6.0, header_rect.bottom() - 4.0),
                        ),
                    ),
                    |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&track.name);
                        });
                        ui.horizontal(|ui| {
                            let mute_btn = egui::Button::new("M").fill(if track.muted {
                                egui::Color32::from_rgb(45, 130, 225)
                            } else {
                                egui::Color32::from_gray(72)
                            });
                            if ui.add(mute_btn).clicked() {
                                pending_commands.push(ProjectCommand::SetTrackMute {
                                    track_id: track.id,
                                    muted: !track.muted,
                                });
                            }

                            let solo_btn = egui::Button::new("S").fill(if track.soloed {
                                egui::Color32::from_rgb(210, 175, 50)
                            } else {
                                egui::Color32::from_gray(72)
                            });
                            if ui.add(solo_btn).clicked() {
                                pending_commands.push(ProjectCommand::SetTrackSolo {
                                    track_id: track.id,
                                    soloed: !track.soloed,
                                });
                            }

                            let arm_btn = egui::Button::new("R").fill(if track.record_armed {
                                egui::Color32::from_rgb(200, 45, 45)
                            } else {
                                egui::Color32::from_gray(72)
                            });
                            if ui.add(arm_btn).clicked() {
                                pending_commands.push(ProjectCommand::SetTrackRecordArmed {
                                    track_id: track.id,
                                    armed: !track.record_armed,
                                });
                            }

                            let mut gain = track.gain_db;
                            if ui
                                .add(
                                    egui::Slider::new(&mut gain, -60.0..=6.0)
                                        .show_value(false),
                                )
                                .on_hover_text(format!("{gain:.1} dB"))
                                .changed()
                            {
                                pending_commands.push(ProjectCommand::ChangeTrackGain {
                                    track_id: track.id,
                                    gain_db: gain,
                                });
                            }
                        });
                    });
                });

                // ── Clips on this track ───────────────────────────────────────
                for clip in &track.clips {
                    let preview_start = if let Some(ref drag) = self.drag_state
                        && drag.clip_id == clip.id
                    {
                        (drag.original_start_frame as i64 + drag.current_offset_frames).max(0)
                            as u64
                    } else {
                        clip.timeline_start_frame
                    };

                    let clip_start_px = lane_x_start + preview_start as f32 * self.timeline_zoom;
                    let clip_dur_px = (clip.duration_frames as f32 * self.timeline_zoom).max(6.0);

                    let clip_rect = egui::Rect::from_min_size(
                        egui::pos2(clip_start_px, lane_top + 4.0),
                        egui::vec2(clip_dur_px, TRACK_HEIGHT - 8.0),
                    );

                    let is_selected = self.selected_clip == Some(clip.id);
                    let clip_color = if is_selected {
                        egui::Color32::from_rgb(95, 175, 245)
                    } else {
                        egui::Color32::from_rgb(58, 122, 192)
                    };

                    painter.rect_filled(clip_rect, 4.0, clip_color);
                    painter.rect_stroke(
                        clip_rect,
                        4.0,
                        egui::Stroke::new(
                            if is_selected { 2.0 } else { 1.0 },
                            egui::Color32::from_gray(210),
                        ),
                        egui::StrokeKind::Inside,
                    );

                    // Waveform thumbnail.
                    if let Some(audio) = self.asset_manager.get(&clip.source_id) {
                        let step = (frames_per_px as usize).max(64);
                        let peaks = self.thumbnail_cache.get_or_compute(
                            clip.source_id,
                            &audio,
                            step,
                        );
                        if !peaks.is_empty() {
                            let wave_h = clip_rect.height() * 0.72;
                            let wave_y_mid = clip_rect.center().y + 5.0;
                            let total_w = clip_rect.width();
                            let n = peaks.len();

                            for (i, &peak) in peaks.iter().enumerate() {
                                let x = clip_rect.left() + (i as f32 / n as f32) * total_w;
                                if x > clip_rect.right() {
                                    break;
                                }
                                let h = (peak * wave_h).max(1.0);
                                painter.line_segment(
                                    [
                                        egui::pos2(x, wave_y_mid - h * 0.5),
                                        egui::pos2(x, wave_y_mid + h * 0.5),
                                    ],
                                    egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_rgba_unmultiplied(230, 245, 255, 190),
                                    ),
                                );
                            }
                        }
                    }

                    painter.text(
                        egui::pos2(clip_rect.left() + 5.0, clip_rect.top() + 3.0),
                        egui::Align2::LEFT_TOP,
                        &clip.name,
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );

                    // Handle clip press / drag start.
                    if response.drag_started()
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        self.selected_clip = Some(clip.id);
                        self.drag_state = Some(DragState {
                            clip_id: clip.id,
                            track_id: track.id,
                            original_start_frame: clip.timeline_start_frame,
                            drag_start_px: pos.x,
                            current_offset_frames: 0,
                        });
                    } else if response.clicked()
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        self.selected_clip = Some(clip.id);
                    }
                }
            }

            // ── MIDI track lanes, headers & MIDI regions ─────────────────────
            for (m_idx, mtrack) in midi_tracks.iter().enumerate() {
                let track_row = num_audio_tracks + m_idx;
                let lane_top = rect.top() + RULER_HEIGHT + track_row as f32 * TRACK_HEIGHT;
                let lane_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), lane_top),
                    egui::vec2(content_width, TRACK_HEIGHT),
                );
                let lane_bg = if track_row.is_multiple_of(2) {
                    egui::Color32::from_gray(38)
                } else {
                    egui::Color32::from_gray(33)
                };
                painter.rect_filled(lane_rect, 0.0, lane_bg);

                let header_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), lane_top + 1.0),
                    egui::vec2(TRACK_HEADER_WIDTH - 2.0, TRACK_HEIGHT - 2.0),
                );
                let is_active_midi = self.active_midi_track_id == Some(mtrack.id);
                painter.rect_filled(
                    header_rect,
                    2.0,
                    if is_active_midi {
                        egui::Color32::from_rgb(90, 92, 96)
                    } else {
                        egui::Color32::from_rgb(80, 80, 86)
                    },
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(header_rect.min, egui::vec2(4.0, header_rect.height())),
                    0.0,
                    egui::Color32::from_rgb(50, 195, 90),
                );

                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                        egui::pos2(header_rect.left() + 8.0, header_rect.top() + 5.0),
                        egui::pos2(header_rect.right() - 6.0, header_rect.bottom() - 4.0),
                    )),
                    |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(format!("🎹 {}", mtrack.name));
                            if ui.small_button("+Region").clicked() {
                                let ph = self.engine_handle.playhead_frame();
                                let tick = tempo_map.sample_to_tick(ph, sr_u32);
                                new_midi_region_req = Some((mtrack.id, tick));
                            }
                        });
                        ui.horizontal(|ui| {
                            let mute_btn = egui::Button::new("M").fill(if mtrack.muted {
                                egui::Color32::from_rgb(45, 130, 225)
                            } else {
                                egui::Color32::from_gray(72)
                            });
                            let solo_btn = egui::Button::new("S").fill(if mtrack.soloed {
                                egui::Color32::from_rgb(210, 175, 50)
                            } else {
                                egui::Color32::from_gray(72)
                            });
                            let arm_btn = egui::Button::new("R").fill(if mtrack.record_armed {
                                egui::Color32::from_rgb(200, 45, 45)
                            } else {
                                egui::Color32::from_gray(72)
                            });
                            let m_click = ui.add(mute_btn).clicked();
                            let s_click = ui.add(solo_btn).clicked();
                            let r_click = ui.add(arm_btn).clicked();
                            if m_click || s_click || r_click {
                                midi_track_toggles.push((mtrack.id, m_click, s_click, r_click));
                            }
                            let mut gain = mtrack.gain_db;
                            if ui
                                .add(egui::Slider::new(&mut gain, -60.0..=6.0).show_value(false))
                                .on_hover_text(format!("{gain:.1} dB ({})", mtrack.instrument.display_name()))
                                .changed()
                            {
                                midi_track_gains.push((mtrack.id, gain));
                            }
                        });
                    });
                });

                // Render MIDI regions on this track
                for clip in &mtrack.clips {
                    let preview_start_tick = if let Some(ref drag) = self.midi_drag_state
                        && drag.track_id == mtrack.id
                        && drag.clip_id == clip.id
                    {
                        (drag.original_start_tick as i64 + drag.current_offset_ticks).max(0) as u64
                    } else {
                        clip.start_tick
                    };

                    let start_frame = tempo_map.tick_to_sample(preview_start_tick, sr_u32);
                    let end_frame = tempo_map
                        .tick_to_sample(preview_start_tick.saturating_add(clip.length_ticks), sr_u32);
                    let dur_frames = end_frame.saturating_sub(start_frame).max(1);

                    let clip_x = lane_x_start + start_frame as f32 * self.timeline_zoom;
                    let clip_w = (dur_frames as f32 * self.timeline_zoom).max(12.0);
                    let clip_rect = egui::Rect::from_min_size(
                        egui::pos2(clip_x, lane_top + 4.0),
                        egui::vec2(clip_w, TRACK_HEIGHT - 8.0),
                    );

                    let is_selected = self.active_midi_track_id == Some(mtrack.id)
                        && self.active_midi_clip_id == Some(clip.id);
                    let fill = if is_selected {
                        egui::Color32::from_rgb(70, 195, 110)
                    } else {
                        egui::Color32::from_rgb(45, 145, 80)
                    };
                    painter.rect_filled(clip_rect, 4.0, fill);
                    painter.rect_stroke(
                        clip_rect,
                        4.0,
                        egui::Stroke::new(
                            if is_selected { 2.0 } else { 1.0 },
                            egui::Color32::from_gray(220),
                        ),
                        egui::StrokeKind::Inside,
                    );

                    // Mini note preview bars inside the region
                    let len_ticks = clip.length_ticks.max(1) as f32;
                    for note in &clip.notes {
                        let nx0 = clip_rect.left()
                            + (note.start_tick as f32 / len_ticks) * clip_rect.width();
                        let nw = ((note.duration_ticks as f32 / len_ticks) * clip_rect.width())
                            .max(2.0);
                        let norm_pitch = 1.0 - (f32::from(note.pitch.clamp(24, 108) - 24) / 84.0);
                        let ny = clip_rect.top() + 16.0 + norm_pitch * (clip_rect.height() - 22.0);
                        painter.line_segment(
                            [egui::pos2(nx0, ny), egui::pos2((nx0 + nw).min(clip_rect.right()), ny)],
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(230, 255, 235)),
                        );
                    }

                    painter.text(
                        egui::pos2(clip_rect.left() + 5.0, clip_rect.top() + 3.0),
                        egui::Align2::LEFT_TOP,
                        &clip.name,
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );

                    if response.drag_started()
                        && !self.scrubbing_playhead
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        self.active_midi_track_id = Some(mtrack.id);
                        self.active_midi_clip_id = Some(clip.id);
                        self.show_piano_roll = true;
                        self.midi_drag_state = Some(MidiClipDragState {
                            track_id: mtrack.id,
                            clip_id: clip.id,
                            original_start_tick: clip.start_tick,
                            drag_start_px: pos.x,
                            current_offset_ticks: 0,
                        });
                    } else if response.clicked()
                        && let Some(pos) = response.interact_pointer_pos()
                        && clip_rect.contains(pos)
                    {
                        self.active_midi_track_id = Some(mtrack.id);
                        self.active_midi_clip_id = Some(clip.id);
                        self.show_piano_roll = true;
                        self.piano_roll_state.scroll_to_tick(clip.start_tick);
                    }
                }

                // Double-click empty MIDI lane area creates a new 4-bar MIDI region there
                if response.double_clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && pos.x >= lane_x_start
                    && pos.y >= lane_top
                    && pos.y <= lane_top + TRACK_HEIGHT
                {
                    let frame = ((pos.x - lane_x_start) / self.timeline_zoom).max(0.0) as u64;
                    let tick = tempo_map.sample_to_tick(frame, sr_u32);
                    // Snap to bar start
                    let bar_ticks = PPQN * 4;
                    let snapped = (tick / bar_ticks) * bar_ticks;
                    new_midi_region_req = Some((mtrack.id, snapped));
                }
            }

            // ── Active MIDI region drag update / commit ───────────────────────
            if let Some(ref mut mdrag) = self.midi_drag_state {
                if response.dragged()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let delta_px = pos.x - mdrag.drag_start_px;
                    let delta_frames = (delta_px / self.timeline_zoom).round() as i64;
                    let orig_frame = tempo_map.tick_to_sample(mdrag.original_start_tick, sr_u32) as i64;
                    let target_frame = (orig_frame + delta_frames).max(0) as u64;
                    let target_tick = tempo_map.sample_to_tick(target_frame, sr_u32) as i64;
                    mdrag.current_offset_ticks = target_tick - mdrag.original_start_tick as i64;
                }
                if response.drag_stopped() {
                    let new_start_tick =
                        (mdrag.original_start_tick as i64 + mdrag.current_offset_ticks).max(0)
                            as u64;
                    let tid = mdrag.track_id;
                    let cid = mdrag.clip_id;
                    let proj = self.project_session.project_mut();
                    if let Some(t) = proj.midi_tracks.iter_mut().find(|t| t.id == tid)
                        && let Some(c) = t.find_clip_mut(cid)
                    {
                        c.start_tick = new_start_tick;
                        midi_schedule_dirty = true;
                    }
                    self.midi_drag_state = None;
                }
            }

            // ── Active clip drag update / commit ──────────────────────────────
            if let Some(ref mut drag) = self.drag_state {
                if response.dragged()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let delta_px = pos.x - drag.drag_start_px;
                    drag.current_offset_frames = (delta_px / self.timeline_zoom).round() as i64;
                }
                if response.drag_stopped() {
                    let new_start =
                        (drag.original_start_frame as i64 + drag.current_offset_frames).max(0)
                            as u64;
                    pending_commands.push(ProjectCommand::MoveClip {
                        clip_id: drag.clip_id,
                        target_track_id: drag.track_id,
                        new_start_frame: new_start,
                    });
                    self.drag_state = None;
                }
            }

            // ── Ruler playhead scrubbing (click or drag on ruler / head) ──────
            if response.drag_started()
                && self.drag_state.is_none()
                && self.midi_drag_state.is_none()
                && let Some(pos) = response.interact_pointer_pos()
                && ruler_rect.contains(pos)
                && pos.x >= lane_x_start
            {
                self.scrubbing_playhead = true;
                let frame = ((pos.x - lane_x_start) / self.timeline_zoom).max(0.0) as u64;
                click_seek_frame = Some(frame);
            }
            if self.scrubbing_playhead {
                if response.dragged()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let frame = ((pos.x - lane_x_start) / self.timeline_zoom).max(0.0) as u64;
                    click_seek_frame = Some(frame);
                }
                if response.drag_stopped() {
                    self.scrubbing_playhead = false;
                }
            }

            // ── Click on ruler or empty lane → seek playhead ──────────────────
            if response.clicked()
                && self.drag_state.is_none()
                && self.midi_drag_state.is_none()
                && let Some(pos) = response.interact_pointer_pos()
                && pos.x >= lane_x_start
            {
                let clicked_on_audio_clip = tracks.iter().enumerate().any(|(idx, t)| {
                    let lane_top = rect.top() + RULER_HEIGHT + idx as f32 * TRACK_HEIGHT;
                    t.clips.iter().any(|c| {
                        let x0 = lane_x_start + c.timeline_start_frame as f32 * self.timeline_zoom;
                        let w = (c.duration_frames as f32 * self.timeline_zoom).max(6.0);
                        egui::Rect::from_min_size(
                            egui::pos2(x0, lane_top + 4.0),
                            egui::vec2(w, TRACK_HEIGHT - 8.0),
                        )
                        .contains(pos)
                    })
                });
                let clicked_on_midi_clip = midi_tracks.iter().enumerate().any(|(m_idx, t)| {
                    let lane_top =
                        rect.top() + RULER_HEIGHT + (num_audio_tracks + m_idx) as f32 * TRACK_HEIGHT;
                    t.clips.iter().any(|c| {
                        let s = tempo_map.tick_to_sample(c.start_tick, sr_u32);
                        let e = tempo_map.tick_to_sample(c.start_tick + c.length_ticks, sr_u32);
                        let x0 = lane_x_start + s as f32 * self.timeline_zoom;
                        let w = ((e.saturating_sub(s)) as f32 * self.timeline_zoom).max(12.0);
                        egui::Rect::from_min_size(
                            egui::pos2(x0, lane_top + 4.0),
                            egui::vec2(w, TRACK_HEIGHT - 8.0),
                        )
                        .contains(pos)
                    })
                });
                if !clicked_on_audio_clip && !clicked_on_midi_clip {
                    let frame = ((pos.x - lane_x_start) / self.timeline_zoom).max(0.0) as u64;
                    click_seek_frame = Some(frame);
                }
            }

            // ── Playhead line + draggable pentagon head on ruler ─────────────
            let ph_frame = self.engine_handle.playhead_frame();
            let ph_x = lane_x_start + ph_frame as f32 * self.timeline_zoom;
            let head_top = rect.top() + 2.0;
            let head_mid = rect.top() + RULER_HEIGHT - 7.0;
            let head_tip = rect.top() + RULER_HEIGHT;
            let head_pts = vec![
                egui::pos2(ph_x - 6.0, head_top),
                egui::pos2(ph_x + 6.0, head_top),
                egui::pos2(ph_x + 6.0, head_mid),
                egui::pos2(ph_x, head_tip),
                egui::pos2(ph_x - 6.0, head_mid),
            ];
            painter.line_segment(
                [egui::pos2(ph_x, head_tip), egui::pos2(ph_x, rect.bottom())],
                egui::Stroke::new(1.5, egui::Color32::from_gray(240)),
            );
            painter.add(egui::Shape::convex_polygon(
                head_pts,
                egui::Color32::from_gray(235),
                egui::Stroke::new(1.0, egui::Color32::from_gray(40)),
            ));

            // ── Ctrl+wheel zoom ───────────────────────────────────────────────
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            if ui.input(|i| i.modifiers.ctrl) && scroll_delta.y.abs() > 0.5 {
                let factor = 1.0 + scroll_delta.y * 0.01;
                self.timeline_zoom = (self.timeline_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            }

            if tracks.is_empty() && midi_tracks.is_empty() {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Import an audio file or add a MIDI track (🎹 +MIDI) to get started.\nDouble-click a MIDI track lane or click +Region to create a MIDI region.",
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_gray(140),
                );
            }
        });

        // ── Apply deferred commands and rebuild schedule ──────────────────────
        if !pending_commands.is_empty() {
            for cmd in pending_commands {
                if let Err(e) = self.project_session.execute(cmd) {
                    self.status_message = format!("Command error: {e}");
                }
            }
            self.rebuild_schedule();
        }

        if !midi_track_toggles.is_empty() || !midi_track_gains.is_empty() {
            let proj = self.project_session.project_mut();
            for (tid, m, s, r) in midi_track_toggles {
                if let Some(t) = proj.midi_tracks.iter_mut().find(|t| t.id == tid) {
                    if m {
                        t.muted = !t.muted;
                    }
                    if s {
                        t.soloed = !t.soloed;
                    }
                    if r {
                        t.record_armed = !t.record_armed;
                    }
                }
            }
            for (tid, g) in midi_track_gains {
                if let Some(t) = proj.midi_tracks.iter_mut().find(|t| t.id == tid) {
                    t.gain_db = g;
                }
            }
            midi_schedule_dirty = true;
        }

        if let Some((tid, start_tick)) = new_midi_region_req {
            self.create_midi_region(tid, start_tick);
        } else if midi_schedule_dirty {
            self.rebuild_midi_schedule();
        }

        if let Some(frame) = click_seek_frame {
            self.engine_handle.seek(frame);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn db_to_linear(db: f32) -> f32 {
    if db <= -96.0 {
        0.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// main
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(not(feature = "component-lab"))]
pub fn run() -> eframe::Result<()> {
    eframe::run_native(
        "Esperia",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1_280.0, 800.0])
                .with_min_inner_size([800.0, 500.0])
                .with_title("Esperia — Audio Workstation"),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(EsperiaApp::new(cc)))),
    )
}
