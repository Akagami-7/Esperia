//! Application command layer with validation and deterministic Undo/Redo history.
//!
//! GUI components and controllers should mutate projects exclusively through
//! [`ProjectSession::execute`] rather than modifying [`Project`] fields directly.

use crate::audio_source::{AudioSource, SourceId};
use crate::clip::{Clip, ClipId};
use crate::error::ProjectError;
use crate::project::Project;
use crate::track::{Track, TrackId};
use std::path::PathBuf;

/// Maximum number of undo steps retained in memory.
const DEFAULT_MAX_HISTORY: usize = 200;

/// Declarative commands representing user or automation actions on a project.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectCommand {
    CreateProject {
        name: String,
        sample_rate: u32,
        tempo_bpm: f64,
    },
    AddTrack {
        name: String,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    RenameTrack {
        track_id: TrackId,
        name: String,
    },
    ImportAudio {
        name: String,
        relative_path: PathBuf,
        sample_rate: u32,
        channels: u16,
        duration_frames: u64,
    },
    CreateClip {
        track_id: TrackId,
        source_id: SourceId,
        name: String,
        timeline_start_frame: u64,
        source_offset_frame: u64,
        duration_frames: u64,
    },
    MoveClip {
        clip_id: ClipId,
        target_track_id: TrackId,
        new_start_frame: u64,
    },
    TrimClip {
        clip_id: ClipId,
        new_start_frame: u64,
        new_source_offset_frame: u64,
        new_duration_frames: u64,
    },
    DeleteClip {
        clip_id: ClipId,
    },
    ChangeTrackGain {
        track_id: TrackId,
        gain_db: f32,
    },
    ChangeTrackPan {
        track_id: TrackId,
        pan: f32,
    },
    SetTrackMute {
        track_id: TrackId,
        muted: bool,
    },
    SetTrackSolo {
        track_id: TrackId,
        soloed: bool,
    },
    SetTrackRecordArmed {
        track_id: TrackId,
        armed: bool,
    },
    SetTrackInputChannel {
        track_id: TrackId,
        channel: Option<u16>,
    },
}

/// Identifier or confirmation returned after executing a [`ProjectCommand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    ProjectCreated,
    TrackAdded(TrackId),
    TrackRemoved(TrackId),
    TrackRenamed(TrackId),
    AudioImported(SourceId),
    ClipCreated(ClipId),
    ClipMoved(ClipId),
    ClipTrimmed(ClipId),
    ClipDeleted(ClipId),
    TrackUpdated(TrackId),
}

/// Manages a live [`Project`] along with an Undo/Redo history stack and dirty flag.
#[derive(Debug, Clone)]
pub struct ProjectSession {
    project: Project,
    undo_stack: Vec<Project>,
    redo_stack: Vec<Project>,
    max_history: usize,
    dirty: bool,
}

impl ProjectSession {
    /// Wrap an existing [`Project`] in a clean session.
    pub fn new(project: Project) -> Self {
        Self {
            project,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history: DEFAULT_MAX_HISTORY,
            dirty: false,
        }
    }

    /// Read-only access to the current project state.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Whether the project has unsaved modifications.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the session clean (e.g. immediately after saving).
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Whether at least one undo step is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether at least one redo step is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Validate and execute a [`ProjectCommand`], recording an undo snapshot on success.
    ///
    /// If validation fails, the project state and history stacks remain untouched.
    pub fn execute(&mut self, cmd: ProjectCommand) -> Result<CommandOutcome, ProjectError> {
        // Validate and prepare next project state on a clone first so failed
        // commands never leave partial mutations behind.
        let mut next = self.project.clone();
        let outcome = apply_command(&mut next, cmd)?;
        next.touch();

        let previous = std::mem::replace(&mut self.project, next);
        self.undo_stack.push(previous);
        if self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.dirty = true;

        Ok(outcome)
    }

    /// Revert the most recent command.
    pub fn undo(&mut self) -> Result<(), ProjectError> {
        let prev = self.undo_stack.pop().ok_or(ProjectError::NothingToUndo)?;
        let current = std::mem::replace(&mut self.project, prev);
        self.redo_stack.push(current);
        self.dirty = true;
        Ok(())
    }

    /// Re-apply the most recently undone command.
    pub fn redo(&mut self) -> Result<(), ProjectError> {
        let next = self.redo_stack.pop().ok_or(ProjectError::NothingToRedo)?;
        let current = std::mem::replace(&mut self.project, next);
        self.undo_stack.push(current);
        self.dirty = true;
        Ok(())
    }
}

fn apply_command(
    project: &mut Project,
    cmd: ProjectCommand,
) -> Result<CommandOutcome, ProjectError> {
    match cmd {
        ProjectCommand::CreateProject {
            name,
            sample_rate,
            tempo_bpm,
        } => {
            *project = Project::new(name, sample_rate, tempo_bpm)?;
            Ok(CommandOutcome::ProjectCreated)
        }
        ProjectCommand::AddTrack { name } => {
            let id = TrackId(project.allocate_id());
            let track = Track::new(id, name)?;
            project.tracks.push(track);
            Ok(CommandOutcome::TrackAdded(id))
        }
        ProjectCommand::RemoveTrack { track_id } => {
            let idx = project
                .tracks
                .iter()
                .position(|t| t.id == track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            project.tracks.remove(idx);
            Ok(CommandOutcome::TrackRemoved(track_id))
        }
        ProjectCommand::RenameTrack { track_id, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(ProjectError::InvalidTrackName(
                    "Track name cannot be empty".to_string(),
                ));
            }
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.name = trimmed.to_string();
            Ok(CommandOutcome::TrackRenamed(track_id))
        }
        ProjectCommand::ImportAudio {
            name,
            relative_path,
            sample_rate,
            channels,
            duration_frames,
        } => {
            let id = SourceId(project.allocate_id());
            let source = AudioSource::new(
                id,
                name,
                relative_path,
                sample_rate,
                channels,
                duration_frames,
            )?;
            project.sources.push(source);
            Ok(CommandOutcome::AudioImported(id))
        }
        ProjectCommand::CreateClip {
            track_id,
            source_id,
            name,
            timeline_start_frame,
            source_offset_frame,
            duration_frames,
        } => {
            let source = project
                .find_source(source_id)
                .ok_or(ProjectError::SourceNotFound(source_id))?;
            let source_end = source_offset_frame.saturating_add(duration_frames);
            if source_end > source.duration_frames {
                return Err(ProjectError::InvalidClipRange(format!(
                    "Clip range [{source_offset_frame}..{source_end}) exceeds source duration ({} frames)",
                    source.duration_frames
                )));
            }

            if project.find_track(track_id).is_none() {
                return Err(ProjectError::TrackNotFound(track_id));
            }

            let clip_id = ClipId(project.allocate_id());
            let clip = Clip::new(
                clip_id,
                source_id,
                name,
                timeline_start_frame,
                source_offset_frame,
                duration_frames,
            )?;

            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.clips.push(clip);
            Ok(CommandOutcome::ClipCreated(clip_id))
        }
        ProjectCommand::MoveClip {
            clip_id,
            target_track_id,
            new_start_frame,
        } => {
            if project.find_track(target_track_id).is_none() {
                return Err(ProjectError::TrackNotFound(target_track_id));
            }

            let (source_track_idx, clip_idx) = find_clip_indices(project, clip_id)?;
            let mut clip = project.tracks[source_track_idx].clips.remove(clip_idx);
            clip.timeline_start_frame = new_start_frame;

            let target_track = project
                .find_track_mut(target_track_id)
                .ok_or(ProjectError::TrackNotFound(target_track_id))?;
            target_track.clips.push(clip);
            Ok(CommandOutcome::ClipMoved(clip_id))
        }
        ProjectCommand::TrimClip {
            clip_id,
            new_start_frame,
            new_source_offset_frame,
            new_duration_frames,
        } => {
            if new_duration_frames == 0 {
                return Err(ProjectError::InvalidClipRange(
                    "Trimmed clip duration must be greater than zero frames".to_string(),
                ));
            }
            let (track_idx, clip_idx) = find_clip_indices(project, clip_id)?;
            let source_id = project.tracks[track_idx].clips[clip_idx].source_id;
            let source = project
                .find_source(source_id)
                .ok_or(ProjectError::SourceNotFound(source_id))?;

            let source_end = new_source_offset_frame.saturating_add(new_duration_frames);
            if source_end > source.duration_frames {
                return Err(ProjectError::InvalidClipRange(format!(
                    "Trimmed range [{new_source_offset_frame}..{source_end}) exceeds source duration ({} frames)",
                    source.duration_frames
                )));
            }

            let clip = &mut project.tracks[track_idx].clips[clip_idx];
            clip.timeline_start_frame = new_start_frame;
            clip.source_offset_frame = new_source_offset_frame;
            clip.duration_frames = new_duration_frames;
            Ok(CommandOutcome::ClipTrimmed(clip_id))
        }
        ProjectCommand::DeleteClip { clip_id } => {
            let (track_idx, clip_idx) = find_clip_indices(project, clip_id)?;
            project.tracks[track_idx].clips.remove(clip_idx);
            Ok(CommandOutcome::ClipDeleted(clip_id))
        }
        ProjectCommand::ChangeTrackGain { track_id, gain_db } => {
            let validated = Track::validate_gain_db(gain_db)?;
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.gain_db = validated;
            Ok(CommandOutcome::TrackUpdated(track_id))
        }
        ProjectCommand::ChangeTrackPan { track_id, pan } => {
            let validated = Track::validate_pan(pan)?;
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.pan = validated;
            Ok(CommandOutcome::TrackUpdated(track_id))
        }
        ProjectCommand::SetTrackMute { track_id, muted } => {
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.muted = muted;
            Ok(CommandOutcome::TrackUpdated(track_id))
        }
        ProjectCommand::SetTrackSolo { track_id, soloed } => {
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.soloed = soloed;
            Ok(CommandOutcome::TrackUpdated(track_id))
        }
        ProjectCommand::SetTrackRecordArmed { track_id, armed } => {
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.record_armed = armed;
            Ok(CommandOutcome::TrackUpdated(track_id))
        }
        ProjectCommand::SetTrackInputChannel { track_id, channel } => {
            let track = project
                .find_track_mut(track_id)
                .ok_or(ProjectError::TrackNotFound(track_id))?;
            track.input_channel = channel;
            Ok(CommandOutcome::TrackUpdated(track_id))
        }
    }
}

fn find_clip_indices(project: &Project, clip_id: ClipId) -> Result<(usize, usize), ProjectError> {
    for (track_idx, track) in project.tracks.iter().enumerate() {
        if let Some(clip_idx) = track.clips.iter().position(|c| c.id == clip_id) {
            return Ok((track_idx, clip_idx));
        }
    }
    Err(ProjectError::ClipNotFound(clip_id))
}
