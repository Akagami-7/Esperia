//! Root project aggregate: metadata, timeline settings, audio sources, and tracks.

use crate::audio_source::{AudioSource, SourceId};
use crate::clip::{Clip, ClipId};
use crate::error::ProjectError;
use crate::timeline::Timeline;
use crate::track::{Track, TrackId};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Current on-disk project manifest schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Descriptive metadata for an Esperia project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub name: String,
    pub author: String,
    pub created_at_unix: u64,
    pub modified_at_unix: u64,
}

/// Complete, hardware-independent data model for an Esperia project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub metadata: ProjectMetadata,
    pub timeline: Timeline,
    pub sources: Vec<AudioSource>,
    pub tracks: Vec<Track>,
    /// Monotonic ID counter persisted with the project so IDs never collide across reloads.
    pub next_id: u64,
}

impl Project {
    /// Create a new, empty project with validated metadata and timeline settings.
    pub fn new(name: String, sample_rate: u32, tempo_bpm: f64) -> Result<Self, ProjectError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ProjectError::InvalidProjectName(
                "Project name cannot be empty".to_string(),
            ));
        }

        let timeline = Timeline::new(sample_rate, tempo_bpm)?;
        let now = current_unix_timestamp();

        Ok(Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            metadata: ProjectMetadata {
                name: trimmed.to_string(),
                author: String::new(),
                created_at_unix: now,
                modified_at_unix: now,
            },
            timeline,
            sources: Vec::new(),
            tracks: Vec::new(),
            next_id: 1,
        })
    }

    /// Allocate the next unique identifier within this project.
    pub fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Update the `modified_at_unix` timestamp to now.
    pub fn touch(&mut self) {
        self.metadata.modified_at_unix = current_unix_timestamp();
    }

    /// Look up an audio source by ID.
    pub fn find_source(&self, source_id: SourceId) -> Option<&AudioSource> {
        self.sources.iter().find(|s| s.id == source_id)
    }

    /// Look up a track by ID.
    pub fn find_track(&self, track_id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == track_id)
    }

    /// Look up a mutable track by ID.
    pub fn find_track_mut(&mut self, track_id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == track_id)
    }

    /// Find a clip across all tracks, returning `(track_id, &Clip)`.
    pub fn find_clip(&self, clip_id: ClipId) -> Option<(TrackId, &Clip)> {
        for track in &self.tracks {
            if let Some(clip) = track.find_clip(clip_id) {
                return Some((track.id, clip));
            }
        }
        None
    }

    /// Count how many clips currently reference `source_id`.
    pub fn clip_count_for_source(&self, source_id: SourceId) -> usize {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.source_id == source_id)
            .count()
    }
}

pub(crate) fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
