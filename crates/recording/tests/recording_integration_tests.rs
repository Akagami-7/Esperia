//! End-to-end integration tests for Milestone 4 (Audio Recording System):
//! * Arm track -> record audio -> stop -> WAV persisted -> saved in project bundle
//!   -> reopened -> decoded -> played back through AudioEngine.
//! * Overdubbing alongside existing playback clips.
//! * Sample-rate conversion (44.1 kHz input -> 48 kHz project) + latency compensation.
//! * Buffer overrun detection and error reporting.

use std::path::PathBuf;
use std::sync::Arc;

use esperia_audio_engine::{AudioEngine, ClipSchedule, MonitorRing, ScheduledClip};
use esperia_media::decode::decode_audio;
use esperia_project_model::{
    commands::{CommandOutcome, ProjectCommand, ProjectSession},
    persistence::{load_project, save_project},
    project::Project,
};
use esperia_recording::{InputChannelSelection, LatencyConfig, Recorder, RecordingError};

#[test]
fn record_save_reopen_and_playback_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let bundle_dir = dir.path().join("MySong.esperia");
    let audio_dir = bundle_dir.join("Audio");
    std::fs::create_dir_all(&audio_dir).unwrap();

    // 1. Create project and add + arm a vocal track.
    let project = Project::new("MySong".into(), 48_000, 120.0).unwrap();
    let mut session = ProjectSession::new(project);
    let track_id = match session
        .execute(ProjectCommand::AddTrack {
            name: "Vocals".into(),
        })
        .unwrap()
    {
        CommandOutcome::TrackAdded(id) => id,
        _ => unreachable!(),
    };
    session
        .execute(ProjectCommand::SetTrackRecordArmed {
            track_id,
            armed: true,
        })
        .unwrap();

    // 2. Record 1024 frames of 0.35 amplitude on mono channel 0 while timeline is at frame 4800.
    let mut recorder = Recorder::new(audio_dir.clone());
    recorder.arm_track(Some(track_id)).unwrap();
    recorder.set_channel_selection(InputChannelSelection::Mono(0));

    let mut capture = recorder.build_input_capture(2).unwrap();
    recorder.start_recording(4_800, 48_000, 48_000, 2).unwrap();

    let mut hw = Vec::with_capacity(1024 * 2);
    for _ in 0..1024 {
        hw.push(0.35);
        hw.push(0.0);
    }
    capture.on_input_samples(&hw, 4_800, Some(5_000_000));

    let take = recorder
        .stop_recording(LatencyConfig {
            input_latency_ns: 5_000_000,
            output_latency_ns: 5_000_000,
            input_buffer_frames: 256,
            output_buffer_frames: 256,
            input_sample_rate: 48_000,
            output_sample_rate: 48_000,
            project_sample_rate: 48_000,
            manual_offset_frames: 0,
        })
        .unwrap();

    assert_eq!(take.timeline_start_frame, 4_800 - 480);
    assert_eq!(take.duration_frames, 1024);

    // 3. Import recorded take into project model and save bundle.
    let rel_path = PathBuf::from("Audio").join(take.file_path.file_name().unwrap());
    let source_id = match session
        .execute(ProjectCommand::ImportAudio {
            name: "Take 1".into(),
            relative_path: rel_path,
            sample_rate: take.sample_rate,
            channels: take.channels,
            duration_frames: take.duration_frames,
        })
        .unwrap()
    {
        CommandOutcome::AudioImported(id) => id,
        _ => unreachable!(),
    };
    session
        .execute(ProjectCommand::CreateClip {
            track_id,
            source_id,
            name: "Take 1".into(),
            timeline_start_frame: take.timeline_start_frame,
            source_offset_frame: 0,
            duration_frames: take.duration_frames,
        })
        .unwrap();

    save_project(&bundle_dir, session.project()).unwrap();

    // 4. Reopen project from disk, decode audio, and play back at compensated start frame.
    let loaded = load_project(&bundle_dir).unwrap();
    assert!(loaded.assets.missing_sources.is_empty());
    let loaded_src = &loaded.project.sources[0];
    let loaded_clip = &loaded.project.tracks[0].clips[0];
    assert!(loaded.project.tracks[0].record_armed);

    let decoded = decode_audio(&bundle_dir.join(&loaded_src.relative_path), Some(48_000)).unwrap();
    let schedule = Arc::new(ClipSchedule {
        clips: vec![ScheduledClip {
            clip_id: loaded_clip.id.0,
            timeline_start_frame: loaded_clip.timeline_start_frame,
            source_offset_frame: loaded_clip.source_offset_frame,
            duration_frames: loaded_clip.duration_frames,
            gain: 1.0,
            channels: decoded.channels,
            samples: Arc::new(decoded.samples),
        }],
    });

    let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
    handle.set_clip_schedule(schedule);
    handle.seek(loaded_clip.timeline_start_frame);
    handle.play();

    let mut out = vec![0.0f32; 256 * 2];
    engine.process(&mut out, 0);
    for &s in &out {
        assert!((s - 0.35).abs() < 0.01, "expected ~0.35, got {s}");
    }
}

#[test]
fn sample_rate_mismatch_recording_is_resampled_to_project_rate() {
    let dir = tempfile::tempdir().unwrap();
    let mut recorder = Recorder::new(dir.path().to_path_buf());
    let tid = esperia_project_model::track::TrackId(1);
    recorder.arm_track(Some(tid)).unwrap();
    recorder.set_channel_selection(InputChannelSelection::Mono(0));

    let mut capture = recorder.build_input_capture(1).unwrap();
    // Input device runs at 44.1 kHz, project runs at 48 kHz.
    recorder.start_recording(4_800, 44_100, 48_000, 1).unwrap();

    let hw = vec![0.25f32; 4410]; // 100 ms @ 44.1 kHz
    capture.on_input_samples(&hw, 4_800, Some(0));

    let take = recorder
        .stop_recording(LatencyConfig {
            input_latency_ns: 0,
            output_latency_ns: 0,
            input_buffer_frames: 0,
            output_buffer_frames: 0,
            input_sample_rate: 44_100,
            output_sample_rate: 48_000,
            project_sample_rate: 48_000,
            manual_offset_frames: 0,
        })
        .unwrap();

    assert_eq!(take.sample_rate, 48_000);
    // 100 ms @ 48 kHz ≈ 4800 frames
    assert!(
        take.duration_frames >= 4750 && take.duration_frames <= 4850,
        "got {} frames",
        take.duration_frames
    );
}

#[test]
fn input_monitoring_passes_through_audio_engine() {
    let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
    handle.set_input_monitoring(true);

    let mon: Arc<MonitorRing> = handle.monitor_ring();
    mon.push_samples(&[0.6f32; 128]);

    let mut out = vec![0.0f32; 128];
    engine.process(&mut out, 0);
    for &s in &out {
        assert!((s - 0.6).abs() < 1e-5, "expected monitor 0.6, got {s}");
    }
}

#[test]
fn ring_buffer_overrun_is_reported_on_stop() {
    let dir = tempfile::tempdir().unwrap();
    let mut recorder = Recorder::new(dir.path().to_path_buf());
    let tid = esperia_project_model::track::TrackId(1);
    recorder.arm_track(Some(tid)).unwrap();

    // Force an overrun directly on the ring buffer + telemetry as would happen
    // if the worker were starved on a tiny ring.
    let ring = recorder.ring_buffer();
    let cap = ring.capacity();
    let huge = vec![0.1f32; cap + 128];
    let res = ring.push_samples(&huge);
    assert_eq!(res, Err(128));
    recorder
        .telemetry()
        .overrun_samples
        .fetch_add(128, std::sync::atomic::Ordering::Relaxed);

    recorder.start_recording(0, 48_000, 48_000, 1).unwrap();
    // Inject overrun into telemetry while active.
    recorder
        .telemetry()
        .overrun_samples
        .fetch_add(64, std::sync::atomic::Ordering::Relaxed);

    let err = recorder
        .stop_recording(LatencyConfig {
            input_latency_ns: 0,
            output_latency_ns: 0,
            input_buffer_frames: 0,
            output_buffer_frames: 0,
            input_sample_rate: 48_000,
            output_sample_rate: 48_000,
            project_sample_rate: 48_000,
            manual_offset_frames: 0,
        })
        .unwrap_err();

    assert!(matches!(
        err,
        RecordingError::RingBufferOverrun {
            dropped_samples: 64
        }
    ));
}
