//! Recording coordinator, background disk writer, take management, and
//! device-aware latency compensation.

use crate::buffer_manager::CaptureRingBuffer;
use crate::file_writer::{WavFileWriter, write_complete_wav};
use crate::input_capture::InputCapture;
use crate::recording_errors::RecordingError;
use crate::recording_state::{
    InputChannelSelection, PunchRegion, RecordingStatus, RecordingTelemetry, TakeInfo,
};
use esperia_media::decode::decode_audio;
use esperia_media::resample::resample_linear;
use esperia_project_model::track::TrackId;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// Latency compensation (M4.4)
// ─────────────────────────────────────────────────────────────────────────────

/// Device-aware timing inputs used to align a recorded take with the playback
/// timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyConfig {
    /// Measured or estimated input latency (ADC → input callback) in nanoseconds.
    pub input_latency_ns: u64,
    /// Measured or estimated output latency (output callback → DAC) in nanoseconds.
    pub output_latency_ns: u64,
    /// Input stream buffer size in frames (used when hardware timestamp is absent).
    pub input_buffer_frames: u32,
    /// Output stream buffer size in frames (used when hardware timestamp is absent).
    pub output_buffer_frames: u32,
    /// Input device sample rate (Hz).
    pub input_sample_rate: u32,
    /// Output device sample rate (Hz).
    pub output_sample_rate: u32,
    /// Project timeline sample rate (Hz).
    pub project_sample_rate: u32,
    /// Optional manual calibration offset in project frames (positive delays,
    /// negative advances).
    pub manual_offset_frames: i64,
}

impl LatencyConfig {
    /// Compute total round-trip latency in project sample frames.
    ///
    /// Rules:
    /// * If the host reports hardware timestamps (`input_latency_ns > 0` /
    ///   `output_latency_ns > 0`), those measured values are used directly.
    /// * Otherwise, we fall back to buffer-size-derived latency
    ///   (`buffer_frames / sample_rate`).
    pub fn total_roundtrip_frames(&self) -> i64 {
        let proj_sr = self.project_sample_rate.max(1) as f64;

        let in_secs = if self.input_latency_ns > 0 {
            self.input_latency_ns as f64 / 1_000_000_000.0
        } else {
            self.input_buffer_frames as f64 / self.input_sample_rate.max(1) as f64
        };

        let out_secs = if self.output_latency_ns > 0 {
            self.output_latency_ns as f64 / 1_000_000_000.0
        } else {
            self.output_buffer_frames as f64 / self.output_sample_rate.max(1) as f64
        };

        let measured_frames = ((in_secs + out_secs) * proj_sr).round() as i64;
        measured_frames + self.manual_offset_frames
    }
}

/// Result of aligning a recording start frame against round-trip latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignedPlacement {
    /// Compensated start frame on the project timeline (>= 0).
    pub timeline_start_frame: u64,
    /// Number of leading frames to trim from the recorded audio if compensation
    /// pushed the start before frame 0.
    pub trim_leading_frames: u64,
    /// Signed compensation applied, in project frames.
    pub compensation_frames: i64,
}

/// Compute aligned timeline placement for a recording started at
/// `recording_start_timeline_frame`.
pub fn compute_aligned_placement(
    recording_start_timeline_frame: u64,
    latency: &LatencyConfig,
) -> AlignedPlacement {
    let comp = latency.total_roundtrip_frames();
    let ideal_start = recording_start_timeline_frame as i64 - comp;
    if ideal_start >= 0 {
        AlignedPlacement {
            timeline_start_frame: ideal_start as u64,
            trim_leading_frames: 0,
            compensation_frames: -comp,
        }
    } else {
        AlignedPlacement {
            timeline_start_frame: 0,
            trim_leading_frames: (-ideal_start) as u64,
            compensation_frames: -(recording_start_timeline_frame as i64),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Active recording session (owned while recording)
// ─────────────────────────────────────────────────────────────────────────────

struct ActiveRecording {
    track_id: TrackId,
    take_number: usize,
    file_path: PathBuf,
    recording_start_timeline_frame: u64,
    input_sample_rate: u32,
    project_sample_rate: u32,
    channels: u16,
    stop_flag: Arc<AtomicBool>,
    worker_handle: JoinHandle<Result<u64, RecordingError>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Recorder
// ─────────────────────────────────────────────────────────────────────────────

/// High-level recording coordinator.
///
/// Manages track arming, preallocated capture ring buffers, the background
/// disk-writer worker, take history, and latency compensation.
pub struct Recorder {
    status: RecordingStatus,
    armed_track: Option<TrackId>,
    channel_selection: InputChannelSelection,
    punch: PunchRegion,
    output_dir: PathBuf,
    ring: Arc<CaptureRingBuffer>,
    monitor_ring: Option<Arc<CaptureRingBuffer>>,
    monitoring_enabled: Arc<AtomicBool>,
    telemetry: Arc<RecordingTelemetry>,
    takes: Vec<TakeInfo>,
    active: Option<ActiveRecording>,
    manual_offset_frames: i64,
}

impl Recorder {
    /// Create a new recorder storing takes under `output_dir`.
    ///
    /// Allocates a 4-second capture ring buffer at 48 kHz stereo (~384k samples)
    /// once up front so the input callback never allocates.
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            status: RecordingStatus::Idle,
            armed_track: None,
            channel_selection: InputChannelSelection::Mono(0),
            punch: PunchRegion::default(),
            output_dir,
            ring: Arc::new(CaptureRingBuffer::new(48_000 * 4 * 2)),
            monitor_ring: None,
            monitoring_enabled: Arc::new(AtomicBool::new(false)),
            telemetry: Arc::new(RecordingTelemetry::new()),
            takes: Vec::new(),
            active: None,
            manual_offset_frames: 0,
        }
    }

    pub fn set_output_dir(&mut self, dir: PathBuf) {
        self.output_dir = dir;
    }

    pub fn set_monitor_ring(&mut self, monitor_ring: Option<Arc<CaptureRingBuffer>>) {
        self.monitor_ring = monitor_ring;
    }

    pub fn set_monitoring_enabled(&self, enabled: bool) {
        self.monitoring_enabled.store(enabled, Ordering::Release);
    }

    pub fn is_monitoring_enabled(&self) -> bool {
        self.monitoring_enabled.load(Ordering::Acquire)
    }

    pub fn set_channel_selection(&mut self, selection: InputChannelSelection) {
        self.channel_selection = selection;
    }

    pub fn channel_selection(&self) -> InputChannelSelection {
        self.channel_selection
    }

    pub fn set_punch_region(&mut self, punch: PunchRegion) {
        self.punch = punch;
    }

    pub fn punch_region(&self) -> PunchRegion {
        self.punch
    }

    pub fn set_manual_offset_frames(&mut self, frames: i64) {
        self.manual_offset_frames = frames;
    }

    pub fn telemetry(&self) -> Arc<RecordingTelemetry> {
        Arc::clone(&self.telemetry)
    }

    pub fn status(&self) -> &RecordingStatus {
        &self.status
    }

    pub fn armed_track(&self) -> Option<TrackId> {
        self.armed_track
    }

    pub fn takes(&self) -> &[TakeInfo] {
        &self.takes
    }

    /// Arm (or disarm) a track for recording.
    pub fn arm_track(&mut self, track_id: Option<TrackId>) -> Result<(), RecordingError> {
        if self.active.is_some() {
            return Err(RecordingError::AlreadyRecording);
        }
        self.armed_track = track_id;
        self.status = match track_id {
            Some(id) => RecordingStatus::Armed { track_id: id },
            None => RecordingStatus::Idle,
        };
        Ok(())
    }

    /// Build an `InputCapture` instance to move into the CPAL input stream
    /// callback.
    pub fn build_input_capture(
        &self,
        device_channels: u16,
    ) -> Result<InputCapture, RecordingError> {
        if device_channels == 0 {
            return Err(RecordingError::InvalidConfig(
                "device_channels must be >= 1".into(),
            ));
        }
        match self.channel_selection {
            InputChannelSelection::Mono(ch) if ch >= device_channels => {
                return Err(RecordingError::UnsupportedChannel {
                    requested: ch,
                    available: device_channels,
                });
            }
            InputChannelSelection::StereoPair(ch) if ch >= device_channels => {
                return Err(RecordingError::UnsupportedChannel {
                    requested: ch,
                    available: device_channels,
                });
            }
            _ => {}
        }

        Ok(InputCapture::new(
            Arc::clone(&self.ring),
            self.monitor_ring.as_ref().map(Arc::clone),
            Arc::clone(&self.telemetry),
            self.channel_selection,
            device_channels,
            self.punch,
            Arc::clone(&self.monitoring_enabled),
        ))
    }

    /// Start recording onto the currently armed track.
    pub fn start_recording(
        &mut self,
        recording_start_timeline_frame: u64,
        input_sample_rate: u32,
        project_sample_rate: u32,
        device_channels: u16,
    ) -> Result<(), RecordingError> {
        if self.active.is_some() {
            return Err(RecordingError::AlreadyRecording);
        }
        let track_id = self.armed_track.ok_or(RecordingError::NotArmed)?;
        if input_sample_rate == 0 || project_sample_rate == 0 || device_channels == 0 {
            return Err(RecordingError::InvalidConfig(
                "sample rates and channel count must be > 0".into(),
            ));
        }

        let channels = self.channel_selection.output_channels(device_channels);
        let take_number = self.takes.len() + 1;
        let file_path = self
            .output_dir
            .join(format!("take_{:03}_track_{}.wav", take_number, track_id.0));

        let mut writer = WavFileWriter::create(file_path.clone(), input_sample_rate, channels)?;

        self.ring.clear();
        self.telemetry.reset_for_take();
        self.telemetry.is_recording.store(true, Ordering::Release);

        let ring = Arc::clone(&self.ring);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);

        let worker_handle = thread::Builder::new()
            .name(format!("esperia-rec-worker-{take_number}"))
            .spawn(move || -> Result<u64, RecordingError> {
                let mut scratch = vec![0.0f32; 4096];
                loop {
                    let read = ring.pop_samples(&mut scratch);
                    if read > 0 {
                        writer.write_f32_samples(&scratch[..read])?;
                    } else if worker_stop.load(Ordering::Acquire) {
                        // Final drain after stop flag is set.
                        loop {
                            let rem = ring.pop_samples(&mut scratch);
                            if rem == 0 {
                                break;
                            }
                            writer.write_f32_samples(&scratch[..rem])?;
                        }
                        break;
                    } else {
                        thread::sleep(Duration::from_millis(2));
                    }
                }
                writer.finalize()
            })
            .map_err(|e| RecordingError::WorkerFailed(e.to_string()))?;

        self.active = Some(ActiveRecording {
            track_id,
            take_number,
            file_path,
            recording_start_timeline_frame,
            input_sample_rate,
            project_sample_rate,
            channels,
            stop_flag,
            worker_handle,
        });
        self.status = RecordingStatus::Recording {
            track_id,
            take_number,
        };
        Ok(())
    }

    /// Stop the active recording, flush the worker, apply sample-rate conversion
    /// and latency compensation, and return the resulting `TakeInfo`.
    pub fn stop_recording(
        &mut self,
        mut latency: LatencyConfig,
    ) -> Result<TakeInfo, RecordingError> {
        let active = self.active.take().ok_or(RecordingError::NotRecording)?;
        self.status = RecordingStatus::Finalizing;
        self.telemetry.is_recording.store(false, Ordering::Release);
        active.stop_flag.store(true, Ordering::Release);

        let raw_frames = active
            .worker_handle
            .join()
            .map_err(|_| RecordingError::WorkerFailed("worker thread panicked".into()))??;

        let overrun_samples = self.telemetry.overrun_samples();

        // Use measured latencies from telemetry if caller didn't override them.
        if latency.input_latency_ns == 0 {
            latency.input_latency_ns = self.telemetry.last_input_latency_ns.load(Ordering::Acquire);
        }
        if latency.output_latency_ns == 0 {
            latency.output_latency_ns = self
                .telemetry
                .last_output_latency_ns
                .load(Ordering::Acquire);
        }
        latency.manual_offset_frames += self.manual_offset_frames;

        let placement = compute_aligned_placement(active.recording_start_timeline_frame, &latency);

        // Apply sample-rate conversion and/or leading latency trim if needed.
        let mut final_frames = raw_frames;
        let needs_resample = active.input_sample_rate != active.project_sample_rate;
        let needs_trim = placement.trim_leading_frames > 0;

        if raw_frames > 0 && (needs_resample || needs_trim) {
            let decoded = decode_audio(&active.file_path, None)
                .map_err(|e| RecordingError::ResampleFailed(e.to_string()))?;
            let resampled = if needs_resample {
                resample_linear(
                    &decoded.samples,
                    active.input_sample_rate,
                    active.project_sample_rate,
                    active.channels,
                )
                .map_err(|e| RecordingError::ResampleFailed(e.to_string()))?
            } else {
                decoded.samples
            };

            let ch = active.channels.max(1) as usize;
            let trim_samples = (placement.trim_leading_frames as usize * ch).min(resampled.len());
            let trimmed = &resampled[trim_samples..];
            final_frames = write_complete_wav(
                &active.file_path,
                trimmed,
                active.project_sample_rate,
                active.channels,
            )?;
        }

        let take = TakeInfo {
            take_number: active.take_number,
            track_id: active.track_id,
            file_path: active.file_path,
            timeline_start_frame: placement.timeline_start_frame,
            duration_frames: final_frames,
            sample_rate: active.project_sample_rate,
            channels: active.channels,
            applied_latency_compensation_frames: placement.compensation_frames,
            overrun_samples,
        };

        self.takes.push(take.clone());
        self.status = match self.armed_track {
            Some(track_id) => RecordingStatus::Armed { track_id },
            None => RecordingStatus::Idle,
        };

        if overrun_samples > 0 {
            return Err(RecordingError::RingBufferOverrun {
                dropped_samples: overrun_samples,
            });
        }

        Ok(take)
    }

    /// Expose the ring buffer for deterministic unit/integration tests.
    pub fn ring_buffer(&self) -> Arc<CaptureRingBuffer> {
        Arc::clone(&self.ring)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_compensation_uses_measured_timestamps_when_present() {
        let cfg = LatencyConfig {
            input_latency_ns: 10_000_000,  // 10 ms = 480 frames @ 48 kHz
            output_latency_ns: 10_000_000, // 10 ms = 480 frames @ 48 kHz
            input_buffer_frames: 512,
            output_buffer_frames: 512,
            input_sample_rate: 48_000,
            output_sample_rate: 48_000,
            project_sample_rate: 48_000,
            manual_offset_frames: 0,
        };
        assert_eq!(cfg.total_roundtrip_frames(), 960);

        let placed = compute_aligned_placement(10_000, &cfg);
        assert_eq!(placed.timeline_start_frame, 9_040);
        assert_eq!(placed.trim_leading_frames, 0);
        assert_eq!(placed.compensation_frames, -960);
    }

    #[test]
    fn latency_compensation_trims_leading_samples_near_zero() {
        let cfg = LatencyConfig {
            input_latency_ns: 10_000_000,
            output_latency_ns: 10_000_000,
            input_buffer_frames: 256,
            output_buffer_frames: 256,
            input_sample_rate: 48_000,
            output_sample_rate: 48_000,
            project_sample_rate: 48_000,
            manual_offset_frames: 0,
        };
        // Recording started at frame 200, roundtrip is 960 -> trim 760 frames, start at 0.
        let placed = compute_aligned_placement(200, &cfg);
        assert_eq!(placed.timeline_start_frame, 0);
        assert_eq!(placed.trim_leading_frames, 760);
    }

    #[test]
    fn record_and_stop_writes_wav_and_creates_take() {
        let dir = tempfile::tempdir().unwrap();
        let mut recorder = Recorder::new(dir.path().to_path_buf());
        let track_id = TrackId(1);
        recorder.arm_track(Some(track_id)).unwrap();
        recorder.set_channel_selection(InputChannelSelection::Mono(0));

        let mut capture = recorder.build_input_capture(2).unwrap();
        recorder.start_recording(48_000, 48_000, 48_000, 2).unwrap();

        // Simulate 480 stereo hardware frames (channel 0 = 0.25, channel 1 = -0.25)
        let mut hw = Vec::with_capacity(480 * 2);
        for _ in 0..480 {
            hw.push(0.25);
            hw.push(-0.25);
        }
        capture.on_input_samples(&hw, 48_000, Some(5_000_000));

        let take = recorder
            .stop_recording(LatencyConfig {
                input_latency_ns: 5_000_000,
                output_latency_ns: 5_000_000,
                input_buffer_frames: 256,
                output_buffer_frames: 256,
                input_sample_rate: 48_000,
                output_sample_rate: 48_000,
                project_sample_rate: 48_000,
                manual_offset_frames: 0,
            })
            .unwrap();

        assert_eq!(take.take_number, 1);
        assert_eq!(take.duration_frames, 480);
        assert_eq!(take.channels, 1);
        assert_eq!(take.timeline_start_frame, 48_000 - 480);
        assert!(take.file_path.exists());
    }
}
