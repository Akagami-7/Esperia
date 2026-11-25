//! Recording state, take metadata, punch region, and lock-free telemetry.

use esperia_project_model::track::TrackId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// High-level lifecycle state of the recorder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingStatus {
    Idle,
    Armed {
        track_id: TrackId,
    },
    Recording {
        track_id: TrackId,
        take_number: usize,
    },
    Finalizing,
    Failed(String),
}

/// Which channel(s) of the hardware input stream to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputChannelSelection {
    /// Record a single hardware channel (0-indexed) as mono.
    Mono(u16),
    /// Record an adjacent stereo pair `(left, left + 1)` (0-indexed).
    StereoPair(u16),
    /// Record all hardware channels as-is.
    All,
}

impl InputChannelSelection {
    /// Number of output channels written to the recorded file.
    pub fn output_channels(self, device_channels: u16) -> u16 {
        match self {
            Self::Mono(_) => 1,
            Self::StereoPair(_) => 2.min(device_channels.max(1)),
            Self::All => device_channels.max(1),
        }
    }
}

/// Optional punch-in / punch-out region in project timeline frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PunchRegion {
    pub enabled: bool,
    pub start_frame: u64,
    pub end_frame: u64,
}

impl PunchRegion {
    /// True if `timeline_frame` is inside the active punch window (or punch is disabled).
    #[inline]
    pub fn allows_capture(&self, timeline_frame: u64) -> bool {
        if !self.enabled || self.end_frame <= self.start_frame {
            return true;
        }
        timeline_frame >= self.start_frame && timeline_frame < self.end_frame
    }
}

/// Metadata for a completed recording take.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeInfo {
    pub take_number: usize,
    pub track_id: TrackId,
    pub file_path: PathBuf,
    pub timeline_start_frame: u64,
    pub duration_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub applied_latency_compensation_frames: i64,
    pub overrun_samples: u64,
}

/// Lock-free telemetry updated from the input callback and background worker,
/// readable from the UI thread without blocking.
pub struct RecordingTelemetry {
    pub is_recording: AtomicBool,
    pub captured_frames: AtomicU64,
    pub overrun_samples: AtomicU64,
    pub input_peak_bits: AtomicU32,
    pub last_input_latency_ns: AtomicU64,
    pub last_output_latency_ns: AtomicU64,
}

impl RecordingTelemetry {
    pub fn new() -> Self {
        Self {
            is_recording: AtomicBool::new(false),
            captured_frames: AtomicU64::new(0),
            overrun_samples: AtomicU64::new(0),
            input_peak_bits: AtomicU32::new(0.0f32.to_bits()),
            last_input_latency_ns: AtomicU64::new(0),
            last_output_latency_ns: AtomicU64::new(0),
        }
    }

    pub fn reset_for_take(&self) {
        self.captured_frames.store(0, Ordering::Release);
        self.overrun_samples.store(0, Ordering::Release);
        self.input_peak_bits
            .store(0.0f32.to_bits(), Ordering::Release);
    }

    pub fn input_peak(&self) -> f32 {
        f32::from_bits(self.input_peak_bits.load(Ordering::Acquire))
    }

    pub fn captured_frames(&self) -> u64 {
        self.captured_frames.load(Ordering::Acquire)
    }

    pub fn overrun_samples(&self) -> u64 {
        self.overrun_samples.load(Ordering::Acquire)
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::Acquire)
    }
}

impl Default for RecordingTelemetry {
    fn default() -> Self {
        Self::new()
    }
}
