use esperia_project_model::persistence::{
    AUDIO_DIR, AUTOSAVE_DIR, CACHE_DIR, MANIFEST_BAK_FILE, MANIFEST_FILE, MANIFEST_TMP_FILE,
    PEAKS_DIR,
};
use esperia_project_model::{
    CURRENT_SCHEMA_VERSION, CommandOutcome, Project, ProjectCommand, ProjectError, ProjectSession,
    SourceId, autosave_project, create_project_bundle, delete_project_bundle, load_project,
    save_project,
};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn test_create_and_delete_project_bundle() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("MySong.esperia");

    let project = Project::new("MySong".to_string(), 48_000, 120.0).expect("valid project");
    create_project_bundle(&bundle, &project).expect("create bundle");

    assert!(bundle.join(MANIFEST_FILE).is_file());
    assert!(bundle.join(AUDIO_DIR).is_dir());
    assert!(bundle.join(PEAKS_DIR).is_dir());
    assert!(bundle.join(AUTOSAVE_DIR).is_dir());
    assert!(bundle.join(CACHE_DIR).is_dir());

    // Creating again at the same path must fail.
    let err = create_project_bundle(&bundle, &project).unwrap_err();
    assert!(matches!(err, ProjectError::ProjectAlreadyExists(_)));

    delete_project_bundle(&bundle).expect("delete bundle");
    assert!(!bundle.exists());
}

#[test]
fn test_add_and_remove_tracks() {
    let project = Project::new("TrackTest".to_string(), 48_000, 128.0).expect("project");
    let mut session = ProjectSession::new(project);

    let t1 = match session
        .execute(ProjectCommand::AddTrack {
            name: "Drums".to_string(),
        })
        .expect("add drums")
    {
        CommandOutcome::TrackAdded(id) => id,
        other => panic!("unexpected outcome: {other:?}"),
    };

    let t2 = match session
        .execute(ProjectCommand::AddTrack {
            name: "Bass".to_string(),
        })
        .expect("add bass")
    {
        CommandOutcome::TrackAdded(id) => id,
        other => panic!("unexpected outcome: {other:?}"),
    };

    assert_eq!(session.project().tracks.len(), 2);

    session
        .execute(ProjectCommand::RemoveTrack { track_id: t1 })
        .expect("remove drums");
    assert_eq!(session.project().tracks.len(), 1);
    assert_eq!(session.project().tracks[0].id, t2);

    // Removing non-existent track fails cleanly.
    let err = session
        .execute(ProjectCommand::RemoveTrack { track_id: t1 })
        .unwrap_err();
    assert!(matches!(err, ProjectError::TrackNotFound(_)));
}

#[test]
fn test_multiple_clips_reference_single_audio_source() {
    let project = Project::new("SharedSource".to_string(), 48_000, 120.0).expect("project");
    let mut session = ProjectSession::new(project);

    let t1 = match session
        .execute(ProjectCommand::AddTrack {
            name: "Track 1".to_string(),
        })
        .expect("track 1")
    {
        CommandOutcome::TrackAdded(id) => id,
        _ => unreachable!(),
    };

    let t2 = match session
        .execute(ProjectCommand::AddTrack {
            name: "Track 2".to_string(),
        })
        .expect("track 2")
    {
        CommandOutcome::TrackAdded(id) => id,
        _ => unreachable!(),
    };

    let src = match session
        .execute(ProjectCommand::ImportAudio {
            name: "Vocal Take".to_string(),
            relative_path: PathBuf::from("Audio/vocal_001.wav"),
            sample_rate: 48_000,
            channels: 2,
            duration_frames: 480_000,
        })
        .expect("import")
    {
        CommandOutcome::AudioImported(id) => id,
        _ => unreachable!(),
    };

    for (idx, (track_id, start)) in [(t1, 0), (t1, 100_000), (t2, 200_000)]
        .into_iter()
        .enumerate()
    {
        session
            .execute(ProjectCommand::CreateClip {
                track_id,
                source_id: src,
                name: format!("Clip {idx}"),
                timeline_start_frame: start,
                source_offset_frame: 0,
                duration_frames: 48_000,
            })
            .expect("create clip");
    }

    assert_eq!(session.project().sources.len(), 1);
    assert_eq!(session.project().clip_count_for_source(src), 3);
}

#[test]
fn test_save_and_reopen_roundtrip() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("Roundtrip.esperia");

    let project = Project::new("Roundtrip".to_string(), 44_100, 110.5).expect("project");
    let mut session = ProjectSession::new(project);

    let track_id = match session
        .execute(ProjectCommand::AddTrack {
            name: "Guitar".to_string(),
        })
        .expect("add track")
    {
        CommandOutcome::TrackAdded(id) => id,
        _ => unreachable!(),
    };

    session
        .execute(ProjectCommand::ChangeTrackGain {
            track_id,
            gain_db: -3.5,
        })
        .expect("gain");
    session
        .execute(ProjectCommand::ChangeTrackPan {
            track_id,
            pan: -0.25,
        })
        .expect("pan");

    let source_id = match session
        .execute(ProjectCommand::ImportAudio {
            name: "riff.wav".to_string(),
            relative_path: PathBuf::from("Audio/riff.wav"),
            sample_rate: 44_100,
            channels: 2,
            duration_frames: 88_200,
        })
        .expect("import")
    {
        CommandOutcome::AudioImported(id) => id,
        _ => unreachable!(),
    };

    session
        .execute(ProjectCommand::CreateClip {
            track_id,
            source_id,
            name: "Intro Riff".to_string(),
            timeline_start_frame: 0,
            source_offset_frame: 0,
            duration_frames: 44_100,
        })
        .expect("clip");

    save_project(&bundle, session.project()).expect("save");
    // Write dummy audio file so asset check passes and peak cache regenerates.
    fs::write(bundle.join("Audio/riff.wav"), b"RIFF").expect("write dummy wav");

    let report = load_project(&bundle).expect("load");
    assert!(!report.recovered_from_backup);
    assert_eq!(report.migrated_from_version, None);
    assert_eq!(report.assets.valid_sources, vec![source_id]);
    assert!(report.assets.missing_sources.is_empty());
    assert_eq!(report.regenerated_peak_caches, 1);
    assert_eq!(&report.project, session.project());
}

#[test]
fn test_handle_missing_audio_files_gracefully() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("MissingAudio.esperia");

    let project = Project::new("MissingAudio".to_string(), 48_000, 120.0).expect("project");
    let mut session = ProjectSession::new(project);

    let source_id = match session
        .execute(ProjectCommand::ImportAudio {
            name: "missing.wav".to_string(),
            relative_path: PathBuf::from("Audio/missing.wav"),
            sample_rate: 48_000,
            channels: 2,
            duration_frames: 48_000,
        })
        .expect("import")
    {
        CommandOutcome::AudioImported(id) => id,
        _ => unreachable!(),
    };

    save_project(&bundle, session.project()).expect("save");
    // Do NOT create Audio/missing.wav on disk.

    let report = load_project(&bundle).expect("load should succeed with missing asset report");
    assert!(report.assets.valid_sources.is_empty());
    assert_eq!(
        report.assets.missing_sources,
        vec![(source_id, PathBuf::from("Audio/missing.wav"))]
    );
}

#[test]
fn test_handle_invalid_project_files_and_incomplete_save_recovery() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("Corrupt.esperia");

    let project = Project::new("CorruptTest".to_string(), 48_000, 120.0).expect("project");
    save_project(&bundle, &project).expect("initial save");

    // Second save creates project.json.bak.
    let mut session = ProjectSession::new(project);
    session
        .execute(ProjectCommand::AddTrack {
            name: "SafeTrack".to_string(),
        })
        .expect("add track");
    save_project(&bundle, session.project()).expect("second save");
    assert!(bundle.join(MANIFEST_BAK_FILE).is_file());

    // Simulate incomplete/corrupted project.json + leftover .tmp file.
    fs::write(bundle.join(MANIFEST_FILE), b"{corrupt json").expect("corrupt main");
    fs::write(bundle.join(MANIFEST_TMP_FILE), b"{partial").expect("write tmp");

    let recovered = load_project(&bundle).expect("should recover from backup");
    assert!(recovered.recovered_from_backup);
    assert!(!bundle.join(MANIFEST_TMP_FILE).exists());

    // Now corrupt BOTH main and backup -> must fail with CorruptProjectFile.
    fs::write(bundle.join(MANIFEST_FILE), b"not json").expect("corrupt main");
    fs::write(bundle.join(MANIFEST_BAK_FILE), b"also not json").expect("corrupt bak");
    let err = load_project(&bundle).unwrap_err();
    assert!(matches!(err, ProjectError::CorruptProjectFile(_)));
}

#[test]
fn test_undo_and_redo() {
    let project = Project::new("UndoRedo".to_string(), 48_000, 120.0).expect("project");
    let mut session = ProjectSession::new(project);

    assert!(!session.can_undo());
    assert!(!session.can_redo());

    let track_id = match session
        .execute(ProjectCommand::AddTrack {
            name: "Lead".to_string(),
        })
        .expect("add track")
    {
        CommandOutcome::TrackAdded(id) => id,
        _ => unreachable!(),
    };

    session
        .execute(ProjectCommand::ChangeTrackGain {
            track_id,
            gain_db: -6.0,
        })
        .expect("change gain");

    // Failed command must not alter state or undo stack.
    let err = session
        .execute(ProjectCommand::ChangeTrackGain {
            track_id,
            gain_db: 42.0,
        })
        .unwrap_err();
    assert!(matches!(err, ProjectError::InvalidGain(_)));
    assert_eq!(session.project().tracks[0].gain_db, -6.0);

    // Undo gain change
    session.undo().expect("undo gain");
    assert_eq!(session.project().tracks[0].gain_db, 0.0);
    assert!(session.can_redo());

    // Undo add track
    session.undo().expect("undo add track");
    assert!(session.project().tracks.is_empty());

    // Redo add track + gain
    session.redo().expect("redo add track");
    assert_eq!(session.project().tracks.len(), 1);
    session.redo().expect("redo gain");
    assert_eq!(session.project().tracks[0].gain_db, -6.0);
}

#[test]
fn test_project_version_migration_and_future_version_rejection() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("Migration.esperia");
    fs::create_dir_all(&bundle).expect("mkdir");

    // Write a v1 manifest.
    let v1_json = r#"{
        "schema_version": 1,
        "name": "Legacy Song",
        "sample_rate": 44100,
        "tempo_bpm": 98.0,
        "tracks": [],
        "next_id": 5
    }"#;
    fs::write(bundle.join(MANIFEST_FILE), v1_json).expect("write v1");

    let report = load_project(&bundle).expect("migrate v1 -> v2");
    assert_eq!(report.migrated_from_version, Some(1));
    assert_eq!(report.project.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(report.project.metadata.name, "Legacy Song");
    assert_eq!(report.project.timeline.sample_rate, 44_100);
    assert_eq!(report.project.timeline.tempo_bpm, 98.0);
    assert_eq!(report.project.next_id, 5);

    // Future schema version must be rejected cleanly.
    let future_json = format!(r#"{{"schema_version": {}}}"#, CURRENT_SCHEMA_VERSION + 10);
    fs::write(bundle.join(MANIFEST_FILE), future_json).expect("write future");
    let err = load_project(&bundle).unwrap_err();
    assert!(matches!(err, ProjectError::UnsupportedSchemaVersion { .. }));
}

#[test]
fn test_autosave_does_not_corrupt_original_project() {
    let dir = tempdir().expect("tempdir");
    let bundle = dir.path().join("AutosaveTest.esperia");

    let original = Project::new("Original".to_string(), 48_000, 120.0).expect("project");
    save_project(&bundle, &original).expect("save original");
    let original_bytes = fs::read(bundle.join(MANIFEST_FILE)).expect("read original");

    let mut session = ProjectSession::new(original);
    session
        .execute(ProjectCommand::AddTrack {
            name: "Unsaved Track".to_string(),
        })
        .expect("add track");

    let autosave_path = autosave_project(&bundle, session.project(), 1001).expect("autosave");
    assert!(autosave_path.is_file());
    assert_eq!(
        fs::read(bundle.join(MANIFEST_FILE)).expect("read original after autosave"),
        original_bytes
    );

    // Also verify SourceId equality works cleanly.
    assert_eq!(SourceId(1), SourceId(1));
}
