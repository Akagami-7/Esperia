//! Structured domain errors for `esperia-recording`.

use std::path::PathBuf;

/// All errors produced by the recording subsystem.
#[derive(Debug)]
pub enum RecordingError {
    /// No track is armed for recording.
    NotArmed,
    /// A recording session is already active.
    AlreadyRecording,
    /// Stop was called while not recording.
    NotRecording,
    /// Invalid recording configuration (e.g. 0 sample rate or 0 channels).
    InvalidConfig(String),
    /// Requested input channel exceeds the device's channel count.
    UnsupportedChannel { requested: u16, available: u16 },
    /// Preallocated capture ring buffer overflowed; samples were dropped.
    RingBufferOverrun { dropped_samples: u64 },
    /// Disk or file-system I/O error.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Background recording worker failed or disconnected.
    WorkerFailed(String),
    /// Sample-rate conversion failed.
    ResampleFailed(String),
}

impl std::fmt::Display for RecordingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotArmed => write!(f, "no track is armed for recording"),
            Self::AlreadyRecording => write!(f, "recording is already in progress"),
            Self::NotRecording => write!(f, "no recording is currently active"),
            Self::InvalidConfig(msg) => write!(f, "invalid recording config: {msg}"),
            Self::UnsupportedChannel {
                requested,
                available,
            } => write!(
                f,
                "requested input channel {requested} exceeds available channels ({available})"
            ),
            Self::RingBufferOverrun { dropped_samples } => {
                write!(
                    f,
                    "recording buffer overrun: {dropped_samples} samples dropped"
                )
            }
            Self::Io { path, source } => {
                write!(f, "recording I/O error on {}: {source}", path.display())
            }
            Self::WorkerFailed(msg) => write!(f, "recording worker failed: {msg}"),
            Self::ResampleFailed(msg) => write!(f, "recording resample failed: {msg}"),
        }
    }
}

impl std::error::Error for RecordingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
