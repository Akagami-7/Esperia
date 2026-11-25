//! Domain error types for `esperia-project-model`.

use crate::audio_source::SourceId;
use crate::clip::ClipId;
use crate::track::TrackId;
use std::fmt;
use std::path::PathBuf;

/// Structured errors returned by project model, command, and persistence operations.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectError {
    InvalidProjectName(String),
    InvalidSampleRate(u32),
    InvalidTempo(f64),
    InvalidTimeSignature(u8, u8),
    InvalidTrackName(String),
    InvalidSourceName(String),
    InvalidSourcePath(String),
    InvalidGain(f32),
    InvalidPan(f32),
    InvalidClipRange(String),
    TrackNotFound(TrackId),
    SourceNotFound(SourceId),
    ClipNotFound(ClipId),
    SourceInUse(SourceId),
    NothingToUndo,
    NothingToRedo,
    UnsupportedSchemaVersion { found: u32, max_supported: u32 },
    CorruptProjectFile(String),
    ProjectAlreadyExists(PathBuf),
    ProjectNotFound(PathBuf),
    Io(String),
    Serialization(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProjectName(msg) => write!(f, "Invalid project name: {msg}"),
            Self::InvalidSampleRate(sr) => write!(f, "Unsupported sample rate: {sr} Hz"),
            Self::InvalidTempo(bpm) => {
                write!(f, "Invalid tempo: {bpm} BPM (expected 20.0..=300.0)")
            }
            Self::InvalidTimeSignature(num, den) => {
                write!(f, "Invalid time signature: {num}/{den}")
            }
            Self::InvalidTrackName(msg) => write!(f, "Invalid track name: {msg}"),
            Self::InvalidSourceName(msg) => write!(f, "Invalid audio source name: {msg}"),
            Self::InvalidSourcePath(msg) => write!(f, "Invalid audio source path: {msg}"),
            Self::InvalidGain(db) => write!(f, "Invalid gain: {db} dB"),
            Self::InvalidPan(pan) => write!(f, "Invalid pan: {pan} (expected -1.0..=1.0)"),
            Self::InvalidClipRange(msg) => write!(f, "Invalid clip range: {msg}"),
            Self::TrackNotFound(id) => write!(f, "Track not found: {id:?}"),
            Self::SourceNotFound(id) => write!(f, "Audio source not found: {id:?}"),
            Self::ClipNotFound(id) => write!(f, "Clip not found: {id:?}"),
            Self::SourceInUse(id) => write!(f, "Audio source is still referenced by clips: {id:?}"),
            Self::NothingToUndo => write!(f, "Nothing to undo"),
            Self::NothingToRedo => write!(f, "Nothing to redo"),
            Self::UnsupportedSchemaVersion {
                found,
                max_supported,
            } => write!(
                f,
                "Unsupported project schema version {found} (max supported: {max_supported})"
            ),
            Self::CorruptProjectFile(msg) => write!(f, "Corrupt project file: {msg}"),
            Self::ProjectAlreadyExists(path) => {
                write!(f, "Project bundle already exists at {}", path.display())
            }
            Self::ProjectNotFound(path) => {
                write!(f, "Project bundle not found at {}", path.display())
            }
            Self::Io(msg) => write!(f, "Filesystem I/O error: {msg}"),
            Self::Serialization(msg) => write!(f, "Serialization error: {msg}"),
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<std::io::Error> for ProjectError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<serde_json::Error> for ProjectError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}
