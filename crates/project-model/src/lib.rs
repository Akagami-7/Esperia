//! `esperia-project-model` — Hardware- and GUI-independent project data model,
//! application command layer, and directory-bundle persistence.

pub mod audio_source;
pub mod clip;
pub mod commands;
pub mod error;
pub mod persistence;
pub mod project;
pub mod timeline;
pub mod track;

pub use audio_source::{AudioSource, SourceId};
pub use clip::{Clip, ClipId};
pub use commands::{CommandOutcome, ProjectCommand, ProjectSession};
pub use error::ProjectError;
pub use persistence::{
    AssetValidationReport, LoadReport, autosave_project, create_project_bundle,
    delete_project_bundle, load_project, save_project, validate_assets,
};
pub use project::{CURRENT_SCHEMA_VERSION, Project, ProjectMetadata};
pub use timeline::Timeline;
pub use track::{MAX_TRACK_GAIN_DB, MIN_TRACK_GAIN_DB, Track, TrackId};
