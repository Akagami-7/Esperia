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

use eframe::egui;
use esperia_audio_engine::{AudioEngine, ClipSchedule, EngineHandle, MonitorRing, ScheduledClip};
use esperia_audio_io::{AudioDeviceInfo, AudioDeviceManager, StreamErrorChannel, cpal};
use esperia_media::{AssetManager, DecodedAudio, LoadStatus};
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

// ─────────────────────────────────────────────────────────────────────────────
// Timeline constants
// ─────────────────────────────────────────────────────────────────────────────

const TRACK_HEADER_WIDTH: f32 = 176.0;
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

    // ── UI state ─────────────────────────────────────────────────────────
    frequency: f32,
    amplitude: f32,
    status_message: String,
    show_device_panel: bool,

    // ── Timeline state ───────────────────────────────────────────────────
    timeline_zoom: f32,
    selected_clip: Option<ClipId>,
    drag_state: Option<DragState>,
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
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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

        let project = Project::new("Untitled".to_string(), active_sr, 120.0)
            .expect("default project parameters are valid");
        let project_session = ProjectSession::new(project);
        let asset_manager = AssetManager::new(Some(active_sr));

        Self {
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
            show_device_panel: false,
            timeline_zoom: DEFAULT_ZOOM,
            selected_clip: None,
            drag_state: None,
            loop_enabled: false,
            loop_start_frame: 0,
            loop_end_frame: (active_sr as u64) * 4,
        }
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
        self.engine_handle.set_clip_schedule(schedule);
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
        let ctx = ui.ctx().clone();
        ctx.request_repaint();

        self.poll_pending_imports();

        if let Some(err) = self.stream_errors.take() {
            self.stream_active.store(false, Ordering::Release);
            self.status_message = format!("CPAL output error: {err}");
        }
        if let Some(err) = self.input_stream_errors.take() {
            self.input_stream_active.store(false, Ordering::Release);
            self.status_message = format!("CPAL input error: {err}");
        }

        // ── Menu bar ─────────────────────────────────────────────────────────
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
                });
            });
        });

        // ── Status bar ────────────────────────────────────────────────────────
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Status:");
                ui.monospace(&self.status_message);
                ui.separator();
                let ph = self.engine_handle.playhead_frame();
                let ph_secs = ph as f64 / self.active_sample_rate.max(1) as f64;
                ui.label(format!(
                    "Position: {:.3} s  |  {:02}:{:02}.{:03}",
                    ph_secs,
                    ph_secs as u64 / 60,
                    ph_secs as u64 % 60,
                    ((ph_secs * 1000.0) as u64 % 1000)
                ));
            });
        });

        // ── Transport bar ─────────────────────────────────────────────────────
        egui::Panel::top("transport").show(ui, |ui| {
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
        ui.horizontal(|ui| {
            let is_playing = self.engine_handle.is_playing();
            let transport_state = self.engine_handle.telemetry.transport_state();

            let play_label = if transport_state == 1 {
                "⏸ Pause"
            } else {
                "▶ Play"
            };
            if ui.button(play_label).clicked() {
                self.engine_handle.toggle_play();
            }

            if ui.button("⏹ Stop").clicked() {
                if matches!(self.recorder.status(), RecordingStatus::Recording { .. }) {
                    self.toggle_recording();
                } else {
                    self.engine_handle.stop();
                }
            }

            let is_recording = matches!(self.recorder.status(), RecordingStatus::Recording { .. });
            let rec_btn = egui::Button::new(if is_recording {
                "⏹ Stop Rec"
            } else {
                "⏺ Record"
            })
            .fill(if is_recording {
                egui::Color32::from_rgb(200, 40, 40)
            } else {
                egui::Color32::from_gray(65)
            });
            if ui.add(rec_btn).clicked() {
                self.toggle_recording();
            }

            let mon_btn = egui::Button::new(if self.input_monitoring {
                "🎧 Mon ON"
            } else {
                "🎧 Mon"
            })
            .fill(if self.input_monitoring {
                egui::Color32::from_rgb(55, 130, 190)
            } else {
                egui::Color32::from_gray(65)
            });
            if ui.add(mon_btn).clicked() {
                self.input_monitoring = !self.input_monitoring;
                self.recorder.set_monitoring_enabled(self.input_monitoring);
                self.engine_handle
                    .set_input_monitoring(self.input_monitoring);
            }

            let in_peak = self.recorder.telemetry().input_peak();
            ui.weak(format!("In {in_peak:.2}"));

            ui.separator();

            let loop_text = if self.loop_enabled {
                "⟳ Loop ON"
            } else {
                "⟳ Loop OFF"
            };
            if ui.button(loop_text).clicked() {
                self.loop_enabled = !self.loop_enabled;
                self.engine_handle.enable_loop(self.loop_enabled);
                if self.loop_enabled {
                    self.engine_handle
                        .set_loop_region(self.loop_start_frame, self.loop_end_frame);
                }
            }

            ui.separator();

            let ph = self.engine_handle.playhead_frame();
            let sr = self.active_sample_rate.max(1) as f64;
            let ph_secs = ph as f64 / sr;
            ui.monospace(format!(
                "{:02}:{:02}.{:03}",
                ph_secs as u64 / 60,
                ph_secs as u64 % 60,
                ((ph_secs * 1000.0) as u64 % 1000)
            ));

            ui.separator();

            let tempo = self.project_session.project().timeline.tempo_bpm;
            ui.label(format!("{tempo:.0} BPM"));

            ui.separator();

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
            ui.colored_label(load_color, format!("DSP {:.1}%", load * 100.0));

            ui.separator();

            let peak = self.engine_handle.telemetry.peak();
            if is_playing {
                ui.colored_label(
                    egui::Color32::from_rgb(80, 220, 100),
                    format!("● peak {peak:.3}"),
                );
            } else {
                ui.colored_label(egui::Color32::GRAY, "○ stopped");
            }

            ui.separator();

            if ui.button("📂 Import Audio").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "wave"])
                    .pick_file()
            {
                self.import_audio_file(path, None);
            }

            if ui.button("➕ Track").clicked() {
                let idx = self.project_session.project().tracks.len() + 1;
                let _ = self.project_session.execute(ProjectCommand::AddTrack {
                    name: format!("Track {idx}"),
                });
            }

            if ui.button("💾 Save").clicked() {
                self.save_project_dialog();
            }
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

        // ── Zoom / scroll / clip actions bar ──────────────────────────────────
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            if ui.button("−").clicked() {
                self.timeline_zoom = (self.timeline_zoom * 0.8).max(MIN_ZOOM);
            }
            ui.label(format!(
                "{:.0} px/s",
                self.timeline_zoom * self.active_sample_rate as f32
            ));
            if ui.button("+").clicked() {
                self.timeline_zoom = (self.timeline_zoom * 1.25).min(MAX_ZOOM);
            }
            if ui.button("Fit").clicked() {
                self.timeline_zoom = DEFAULT_ZOOM;
            }
            if let Some(sel_id) = self.selected_clip {
                ui.separator();
                if ui.button("🗑 Delete Selected Clip").clicked() {
                    let _ = self
                        .project_session
                        .execute(ProjectCommand::DeleteClip { clip_id: sel_id });
                    self.selected_clip = None;
                    self.rebuild_schedule();
                }
            }
        });

        ui.separator();

        // ── Scrollable timeline region ────────────────────────────────────────
        let scroll_area = egui::ScrollArea::both()
            .id_salt("timeline_scroll")
            .max_height((available.y - 40.0).max(120.0));

        let mut pending_commands: Vec<ProjectCommand> = Vec::new();
        let mut click_seek_frame: Option<u64> = None;

        scroll_area.show(ui, |ui| {
            let sr = self.active_sample_rate.max(1) as f64;
            let tracks = self.project_session.project().tracks.clone();
            let num_tracks = tracks.len();

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

            painter.rect_filled(rect, 0.0, egui::Color32::from_gray(28));

            // ── Time ruler ────────────────────────────────────────────────────
            let ruler_rect =
                egui::Rect::from_min_size(rect.min, egui::vec2(content_width, RULER_HEIGHT));
            painter.rect_filled(ruler_rect, 0.0, egui::Color32::from_gray(44));

            let frames_per_px = 1.0 / self.timeline_zoom;
            let px_per_sec = sr as f32 * self.timeline_zoom;
            let marker_step_secs = if px_per_sec > 200.0 {
                0.25
            } else if px_per_sec > 60.0 {
                1.0
            } else if px_per_sec > 15.0 {
                5.0
            } else {
                15.0
            };
            let frames_per_marker = ((marker_step_secs * sr as f32) as u64).max(1);
            let mut marker_frame = 0u64;
            while marker_frame <= total_frames {
                let x = lane_x_start + marker_frame as f32 * self.timeline_zoom;
                painter.line_segment(
                    [
                        egui::pos2(x, rect.top() + RULER_HEIGHT - 8.0),
                        egui::pos2(x, rect.top() + RULER_HEIGHT),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(130)),
                );
                let secs = marker_frame as f64 / sr;
                painter.text(
                    egui::pos2(x + 3.0, rect.top() + 4.0),
                    egui::Align2::LEFT_TOP,
                    format!("{secs:.1}s"),
                    egui::FontId::monospace(10.0),
                    egui::Color32::from_gray(185),
                );
                marker_frame += frames_per_marker;
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

                // Track header background.
                let header_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), lane_top + 1.0),
                    egui::vec2(TRACK_HEADER_WIDTH - 4.0, TRACK_HEIGHT - 2.0),
                );
                painter.rect_filled(header_rect, 4.0, egui::Color32::from_gray(54));

                // Interactive track header widgets using ui.put / child Ui.
                ui.scope_builder(egui::UiBuilder::new().max_rect(header_rect.shrink(6.0)), |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&track.name);
                        });
                        ui.horizontal(|ui| {
                            let mute_btn = egui::Button::new("M").fill(if track.muted {
                                egui::Color32::from_rgb(200, 70, 70)
                            } else {
                                egui::Color32::from_gray(75)
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
                                egui::Color32::from_gray(75)
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
                                egui::Color32::from_gray(75)
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
                                    egui::DragValue::new(&mut gain)
                                        .range(-60.0..=6.0)
                                        .speed(0.2)
                                        .suffix(" dB"),
                                )
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

            // ── Click on ruler or empty lane → seek playhead ──────────────────
            if response.clicked()
                && self.drag_state.is_none()
                && let Some(pos) = response.interact_pointer_pos()
                && pos.x >= lane_x_start
            {
                let clicked_on_clip = tracks.iter().enumerate().any(|(idx, t)| {
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
                if !clicked_on_clip {
                    let frame = ((pos.x - lane_x_start) / self.timeline_zoom).max(0.0) as u64;
                    click_seek_frame = Some(frame);
                }
            }

            // ── Playhead ──────────────────────────────────────────────────────
            let ph_frame = self.engine_handle.playhead_frame();
            let ph_x = lane_x_start + ph_frame as f32 * self.timeline_zoom;
            painter.line_segment(
                [egui::pos2(ph_x, rect.top()), egui::pos2(ph_x, rect.bottom())],
                egui::Stroke::new(2.0, egui::Color32::RED),
            );

            // ── Ctrl+wheel zoom ───────────────────────────────────────────────
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            if ui.input(|i| i.modifiers.ctrl) && scroll_delta.y.abs() > 0.5 {
                let factor = 1.0 + scroll_delta.y * 0.01;
                self.timeline_zoom = (self.timeline_zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            }

            if tracks.is_empty() {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Import an audio file to get started.\nUse File › Import Audio… or the 📂 Import Audio button.",
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

fn main() -> eframe::Result<()> {
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
