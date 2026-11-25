//! Track model and channel strip parameters.

use crate::clip::{Clip, ClipId};
use crate::error::ProjectError;
use serde::{Deserialize, Serialize};

/// Minimum allowed track fader gain in dB.
pub const MIN_TRACK_GAIN_DB: f32 = -96.0;
/// Maximum allowed track fader gain in dB.
pub const MAX_TRACK_GAIN_DB: f32 = 6.0;

/// Unique identifier for a track in a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TrackId(pub u64);

/// A sequential lane in the arrangement holding audio clips and mixer state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub gain_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    #[serde(default)]
    pub record_armed: bool,
    #[serde(default)]
    pub input_channel: Option<u16>,
    pub clips: Vec<Clip>,
}

impl Track {
    /// Create a new empty track with unity gain (`0.0 dB`) and center pan (`0.0`).
    pub fn new(id: TrackId, name: String) -> Result<Self, ProjectError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ProjectError::InvalidTrackName(
                "Track name cannot be empty".to_string(),
            ));
        }
        Ok(Self {
            id,
            name: trimmed.to_string(),
            gain_db: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            record_armed: false,
            input_channel: None,
            clips: Vec::new(),
        })
    }

    /// Validate gain in dB (`-96.0..=6.0`, finite).
    pub fn validate_gain_db(gain_db: f32) -> Result<f32, ProjectError> {
        if !gain_db.is_finite() || !(MIN_TRACK_GAIN_DB..=MAX_TRACK_GAIN_DB).contains(&gain_db) {
            return Err(ProjectError::InvalidGain(gain_db));
        }
        Ok(gain_db)
    }

    /// Validate stereo pan (`-1.0..=1.0`, finite).
    pub fn validate_pan(pan: f32) -> Result<f32, ProjectError> {
        if !pan.is_finite() || !(-1.0..=1.0).contains(&pan) {
            return Err(ProjectError::InvalidPan(pan));
        }
        Ok(pan)
    }

    /// Find a clip by ID on this track.
    pub fn find_clip(&self, clip_id: ClipId) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == clip_id)
    }

    /// Find a mutable clip by ID on this track.
    pub fn find_clip_mut(&mut self, clip_id: ClipId) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|c| c.id == clip_id)
    }
}
