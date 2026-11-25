//! Audio clip representation on a track timeline.
//!
//! A `Clip` is a lightweight, non-destructive reference to an [`crate::audio_source::AudioSource`].

use crate::audio_source::SourceId;
use crate::error::ProjectError;
use serde::{Deserialize, Serialize};

/// Unique identifier for a clip on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClipId(pub u64);

/// A non-destructive region of an [`AudioSource`] placed on a [`crate::track::Track`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub source_id: SourceId,
    pub name: String,
    /// Position on the timeline in project sample frames.
    pub timeline_start_frame: u64,
    /// Offset into the referenced `AudioSource` in sample frames.
    pub source_offset_frame: u64,
    /// Length of the clip on the timeline in sample frames.
    pub duration_frames: u64,
    /// Clip-level gain trim in dB.
    pub gain_db: f32,
    pub fade_in_frames: u64,
    pub fade_out_frames: u64,
}

impl Clip {
    /// Construct a validated `Clip`.
    pub fn new(
        id: ClipId,
        source_id: SourceId,
        name: String,
        timeline_start_frame: u64,
        source_offset_frame: u64,
        duration_frames: u64,
    ) -> Result<Self, ProjectError> {
        if duration_frames == 0 {
            return Err(ProjectError::InvalidClipRange(
                "Clip duration must be greater than zero frames".to_string(),
            ));
        }
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            "Untitled Clip".to_string()
        } else {
            trimmed.to_string()
        };

        Ok(Self {
            id,
            source_id,
            name: final_name,
            timeline_start_frame,
            source_offset_frame,
            duration_frames,
            gain_db: 0.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
        })
    }

    /// Exclusive end position of this clip on the timeline in sample frames.
    pub fn timeline_end_frame(&self) -> u64 {
        self.timeline_start_frame
            .saturating_add(self.duration_frames)
    }
}
