//! Audio source asset representation.
//!
//! An `AudioSource` represents an original imported or recorded audio file in
//! the project's `Audio/` pool. Multiple [`crate::clip::Clip`] instances on the
//! timeline can reference the same `AudioSource` without duplicating files.

use crate::error::ProjectError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unique identifier for an imported or recorded audio source within a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceId(pub u64);

/// Metadata describing an original audio asset in the project's audio pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSource {
    pub id: SourceId,
    pub name: String,
    /// Path relative to the project bundle root (e.g. `Audio/imported_001.wav`).
    pub relative_path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_frames: u64,
}

impl AudioSource {
    /// Construct a validated `AudioSource`.
    pub fn new(
        id: SourceId,
        name: String,
        relative_path: PathBuf,
        sample_rate: u32,
        channels: u16,
        duration_frames: u64,
    ) -> Result<Self, ProjectError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ProjectError::InvalidSourceName(
                "Source name cannot be empty".to_string(),
            ));
        }
        if relative_path.as_os_str().is_empty() || relative_path.is_absolute() {
            return Err(ProjectError::InvalidSourcePath(
                "Source path must be a non-empty relative path".to_string(),
            ));
        }
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(ProjectError::InvalidSampleRate(sample_rate));
        }
        if channels == 0 {
            return Err(ProjectError::InvalidSourcePath(
                "Channel count must be at least 1".to_string(),
            ));
        }

        Ok(Self {
            id,
            name: trimmed.to_string(),
            relative_path,
            sample_rate,
            channels,
            duration_frames,
        })
    }
}
