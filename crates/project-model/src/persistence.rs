//! Native directory-bundle persistence, atomic saves, recovery, and schema migrations.
//!
//! Bundle layout:
//! ```text
//! MyComposition.esperia/
//! ├── project.json
//! ├── Audio/
//! ├── Peaks/
//! ├── Autosave/
//! └── Cache/
//! ```

use crate::audio_source::SourceId;
use crate::error::ProjectError;
use crate::project::{CURRENT_SCHEMA_VERSION, Project, ProjectMetadata, current_unix_timestamp};
use crate::timeline::Timeline;
use crate::track::Track;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "project.json";
pub const MANIFEST_TMP_FILE: &str = "project.json.tmp";
pub const MANIFEST_BAK_FILE: &str = "project.json.bak";
pub const AUDIO_DIR: &str = "Audio";
pub const PEAKS_DIR: &str = "Peaks";
pub const AUTOSAVE_DIR: &str = "Autosave";
pub const CACHE_DIR: &str = "Cache";

/// Report of audio asset integrity discovered when opening a project bundle.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssetValidationReport {
    pub valid_sources: Vec<SourceId>,
    pub missing_sources: Vec<(SourceId, PathBuf)>,
}

/// Result returned when loading a project bundle from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadReport {
    pub project: Project,
    pub recovered_from_backup: bool,
    pub migrated_from_version: Option<u32>,
    pub assets: AssetValidationReport,
    pub regenerated_peak_caches: usize,
}

/// Legacy schema v1 format used to test and perform forward migrations to v2.
#[derive(Debug, Deserialize)]
struct LegacyProjectV1 {
    name: String,
    sample_rate: u32,
    tempo_bpm: f64,
    #[serde(default)]
    tracks: Vec<Track>,
    #[serde(default = "default_next_id")]
    next_id: u64,
}

fn default_next_id() -> u64 {
    1
}

#[derive(Debug, Deserialize)]
struct SchemaProbe {
    schema_version: u32,
}

/// Ensure all standard bundle subdirectories exist inside `bundle_dir`.
pub fn ensure_bundle_directories(bundle_dir: &Path) -> Result<(), ProjectError> {
    fs::create_dir_all(bundle_dir)?;
    for sub in [AUDIO_DIR, PEAKS_DIR, AUTOSAVE_DIR, CACHE_DIR] {
        fs::create_dir_all(bundle_dir.join(sub))?;
    }
    Ok(())
}

/// Create a brand-new project directory bundle on disk.
///
/// Fails with [`ProjectError::ProjectAlreadyExists`] if `bundle_dir` already exists.
pub fn create_project_bundle(bundle_dir: &Path, project: &Project) -> Result<(), ProjectError> {
    if bundle_dir.exists() {
        return Err(ProjectError::ProjectAlreadyExists(bundle_dir.to_path_buf()));
    }
    save_project(bundle_dir, project)
}

/// Delete an existing project directory bundle from disk.
pub fn delete_project_bundle(bundle_dir: &Path) -> Result<(), ProjectError> {
    if !bundle_dir.exists() {
        return Err(ProjectError::ProjectNotFound(bundle_dir.to_path_buf()));
    }
    fs::remove_dir_all(bundle_dir)?;
    Ok(())
}

/// Atomically save `project` into `bundle_dir/project.json`.
///
/// Writes to `project.json.tmp`, flushes to OS storage, backs up any existing
/// `project.json` to `project.json.bak`, and atomically renames the temp file.
pub fn save_project(bundle_dir: &Path, project: &Project) -> Result<(), ProjectError> {
    ensure_bundle_directories(bundle_dir)?;

    let manifest_path = bundle_dir.join(MANIFEST_FILE);
    let tmp_path = bundle_dir.join(MANIFEST_TMP_FILE);
    let bak_path = bundle_dir.join(MANIFEST_BAK_FILE);

    let json = serde_json::to_string_pretty(project)?;
    write_and_sync(&tmp_path, json.as_bytes())?;

    if manifest_path.exists() {
        fs::copy(&manifest_path, &bak_path)?;
    }

    fs::rename(&tmp_path, &manifest_path)?;
    Ok(())
}

/// Write an isolated autosave snapshot into `bundle_dir/Autosave/` without
/// modifying `project.json` or `project.json.bak`.
pub fn autosave_project(
    bundle_dir: &Path,
    project: &Project,
    snapshot_id: u64,
) -> Result<PathBuf, ProjectError> {
    ensure_bundle_directories(bundle_dir)?;

    let autosave_dir = bundle_dir.join(AUTOSAVE_DIR);
    let target = autosave_dir.join(format!("autosave_{snapshot_id}.json"));
    let tmp = autosave_dir.join(format!("autosave_{snapshot_id}.json.tmp"));

    let json = serde_json::to_string_pretty(project)?;
    write_and_sync(&tmp, json.as_bytes())?;
    fs::rename(&tmp, &target)?;

    Ok(target)
}

/// Load a project bundle from `bundle_dir`, performing:
/// 1. Incomplete-save recovery (falls back to `project.json.bak` if `project.json`
///    is missing or corrupt, and removes stale `project.json.tmp`).
/// 2. Schema version validation and forward migration (v1 -> v2).
/// 3. Asset reference verification (records missing audio files without failing).
/// 4. Missing waveform peak cache regeneration in `Peaks/`.
pub fn load_project(bundle_dir: &Path) -> Result<LoadReport, ProjectError> {
    if !bundle_dir.exists() {
        return Err(ProjectError::ProjectNotFound(bundle_dir.to_path_buf()));
    }

    ensure_bundle_directories(bundle_dir)?;

    // Clean up any leftover interrupted temp file from an incomplete save.
    let tmp_path = bundle_dir.join(MANIFEST_TMP_FILE);
    if tmp_path.exists() {
        let _ = fs::remove_file(&tmp_path);
    }

    let manifest_path = bundle_dir.join(MANIFEST_FILE);
    let bak_path = bundle_dir.join(MANIFEST_BAK_FILE);

    let (raw_json, recovered_from_backup) = match fs::read_to_string(&manifest_path) {
        Ok(content) => {
            // Verify it's valid JSON with a schema_version before accepting.
            if parse_and_migrate(&content).is_ok() {
                (content, false)
            } else if bak_path.exists() {
                let bak_content = fs::read_to_string(&bak_path)?;
                (bak_content, true)
            } else {
                (content, false)
            }
        }
        Err(_) if bak_path.exists() => {
            let bak_content = fs::read_to_string(&bak_path)?;
            (bak_content, true)
        }
        Err(err) => return Err(ProjectError::from(err)),
    };

    let (project, migrated_from_version) = parse_and_migrate(&raw_json)?;
    let assets = validate_assets(bundle_dir, &project);
    let regenerated_peak_caches = regenerate_missing_peak_caches(bundle_dir, &project, &assets)?;

    Ok(LoadReport {
        project,
        recovered_from_backup,
        migrated_from_version,
        assets,
        regenerated_peak_caches,
    })
}

/// Validate that all referenced audio sources exist inside `bundle_dir`.
pub fn validate_assets(bundle_dir: &Path, project: &Project) -> AssetValidationReport {
    let mut report = AssetValidationReport::default();
    for source in &project.sources {
        let full_path = bundle_dir.join(&source.relative_path);
        if full_path.is_file() {
            report.valid_sources.push(source.id);
        } else {
            report
                .missing_sources
                .push((source.id, source.relative_path.clone()));
        }
    }
    report
}

/// Regenerate missing waveform cache descriptors in `Peaks/` for valid sources.
pub fn regenerate_missing_peak_caches(
    bundle_dir: &Path,
    project: &Project,
    assets: &AssetValidationReport,
) -> Result<usize, ProjectError> {
    let peaks_dir = bundle_dir.join(PEAKS_DIR);
    fs::create_dir_all(&peaks_dir)?;

    let mut regenerated = 0usize;
    for source in &project.sources {
        if !assets.valid_sources.contains(&source.id) {
            continue;
        }
        let cache_file = peaks_dir.join(format!("source_{}.peaks", source.id.0));
        if !cache_file.exists() {
            let placeholder = format!(
                "ESPERIA_PEAKS_V1 source_id={} sample_rate={} frames={}\n",
                source.id.0, source.sample_rate, source.duration_frames
            );
            write_and_sync(&cache_file, placeholder.as_bytes())?;
            regenerated += 1;
        }
    }

    Ok(regenerated)
}

fn parse_and_migrate(raw_json: &str) -> Result<(Project, Option<u32>), ProjectError> {
    let probe: SchemaProbe = serde_json::from_str(raw_json)
        .map_err(|e| ProjectError::CorruptProjectFile(e.to_string()))?;

    match probe.schema_version {
        0 => Err(ProjectError::CorruptProjectFile(
            "Schema version 0 is invalid".to_string(),
        )),
        1 => {
            let v1: LegacyProjectV1 = serde_json::from_str(raw_json)
                .map_err(|e| ProjectError::CorruptProjectFile(e.to_string()))?;
            let timeline = Timeline::new(v1.sample_rate, v1.tempo_bpm)?;
            let now = current_unix_timestamp();
            let project = Project {
                schema_version: CURRENT_SCHEMA_VERSION,
                metadata: ProjectMetadata {
                    name: v1.name,
                    author: String::new(),
                    created_at_unix: now,
                    modified_at_unix: now,
                },
                timeline,
                sources: Vec::new(),
                tracks: v1.tracks,
                midi_tracks: Vec::new(),
                tempo_map: esperia_midi::TempoMap::constant(v1.tempo_bpm),
                next_id: v1.next_id.max(1),
            };
            Ok((project, Some(1)))
        }
        CURRENT_SCHEMA_VERSION => {
            let project: Project = serde_json::from_str(raw_json)
                .map_err(|e| ProjectError::CorruptProjectFile(e.to_string()))?;
            Timeline::validate_sample_rate(project.timeline.sample_rate)?;
            Timeline::validate_tempo(project.timeline.tempo_bpm)?;
            Ok((project, None))
        }
        other => Err(ProjectError::UnsupportedSchemaVersion {
            found: other,
            max_supported: CURRENT_SCHEMA_VERSION,
        }),
    }
}

fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
