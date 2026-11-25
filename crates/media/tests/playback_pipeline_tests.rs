//! End-to-end integration tests for the Esperia audio playback pipeline:
//! WAV decoding (`esperia-media`) + resampling + multitrack mixing & transport
//! (`esperia-audio-engine`).

use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use esperia_audio_engine::{AudioEngine, ClipSchedule, ScheduledClip};
use esperia_media::{AssetManager, LoadStatus, SourceId, decode::decode_audio};

/// Write a constant-amplitude PCM16 WAV file for deterministic mixing tests.
fn write_dc_wav(path: &std::path::Path, sample_rate: u32, channels: u16, frames: u32, value: i16) {
    let num_samples = (frames * channels as u32) as usize;
    let mut data: Vec<u8> = Vec::with_capacity(num_samples * 2);
    for _ in 0..num_samples {
        data.extend_from_slice(&value.to_le_bytes());
    }
    let data_len = data.len() as u32;
    let mut f = std::fs::File::create(path).expect("create wav");
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&channels.to_le_bytes()).unwrap();
    f.write_all(&sample_rate.to_le_bytes()).unwrap();
    f.write_all(&(sample_rate * channels as u32 * 2).to_le_bytes())
        .unwrap();
    f.write_all(&(channels * 2).to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(&data).unwrap();
}

#[test]
fn multitrack_simultaneous_clips_decoded_and_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let wav_a = dir.path().join("a.wav");
    let wav_b = dir.path().join("b.wav");

    // 48 kHz stereo, ~0.25 amplitude and ~0.25 amplitude -> summed ~0.5
    let amp_quarter = (i16::MAX as f32 * 0.25) as i16;
    write_dc_wav(&wav_a, 48_000, 2, 1024, amp_quarter);
    write_dc_wav(&wav_b, 48_000, 2, 1024, amp_quarter);

    let dec_a = decode_audio(&wav_a, Some(48_000)).unwrap();
    let dec_b = decode_audio(&wav_b, Some(48_000)).unwrap();

    let schedule = Arc::new(ClipSchedule {
        clips: vec![
            ScheduledClip {
                clip_id: 1,
                track_id: 1,
                timeline_start_frame: 0,
                source_offset_frame: 0,
                duration_frames: dec_a.duration_frames,
                gain: 1.0,
                channels: dec_a.channels,
                samples: Arc::new(dec_a.samples),
            },
            ScheduledClip {
                clip_id: 2,
                track_id: 2,
                timeline_start_frame: 0,
                source_offset_frame: 0,
                duration_frames: dec_b.duration_frames,
                gain: 1.0,
                channels: dec_b.channels,
                samples: Arc::new(dec_b.samples),
            },
        ],
    });

    let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
    handle.set_clip_schedule(schedule);
    handle.play();

    let mut out = vec![0.0f32; 256 * 2];
    engine.process(&mut out, 0);

    for &s in &out {
        assert!((s - 0.5).abs() < 0.01, "expected mixed ~0.5, got {s}");
    }
}

#[test]
fn different_sample_rates_resampled_and_played_across_buffer_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let wav_441 = dir.path().join("44100.wav");
    let amp_half = (i16::MAX as f32 * 0.5) as i16;
    write_dc_wav(&wav_441, 44_100, 2, 4410, amp_half);

    let mgr = AssetManager::new(Some(48_000));
    let id = SourceId(1);
    mgr.load_async(id, wav_441, Some(48_000)).unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(mgr.status(&id), Some(LoadStatus::Ready)) {
        assert!(Instant::now() < deadline, "timed out loading asset");
        std::thread::sleep(Duration::from_millis(5));
    }

    let decoded = mgr.get(&id).unwrap();
    assert_eq!(decoded.sample_rate, 48_000);

    for &buf_frames in &[32usize, 64, 128, 256, 512] {
        let schedule = Arc::new(ClipSchedule {
            clips: vec![ScheduledClip {
                clip_id: 1,
                track_id: 1,
                timeline_start_frame: 0,
                source_offset_frame: 0,
                duration_frames: decoded.duration_frames,
                gain: 1.0,
                channels: decoded.channels,
                samples: Arc::new(decoded.samples.clone()),
            }],
        });

        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.set_clip_schedule(schedule);
        handle.play();

        let mut out = vec![0.0f32; buf_frames * 2];
        engine.process(&mut out, 0);

        for &s in &out {
            assert!(
                (s - 0.5).abs() < 0.01,
                "buf_frames={buf_frames}: expected ~0.5, got {s}"
            );
        }
    }
}

#[test]
fn rapid_seek_play_pause_stop_and_loop_stability() {
    let samples = Arc::new(vec![0.4f32; 48_000 * 2]); // 1 second stereo
    let schedule = Arc::new(ClipSchedule {
        clips: vec![ScheduledClip {
            clip_id: 10,
            track_id: 1,
            timeline_start_frame: 0,
            source_offset_frame: 0,
            duration_frames: 48_000,
            gain: 1.0,
            channels: 2,
            samples,
        }],
    });

    let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
    handle.set_clip_schedule(schedule);
    handle.set_loop_region(1000, 2000);
    handle.enable_loop(true);

    let mut buf = vec![0.0f32; 256 * 2];

    for i in 0..20 {
        handle.seek(900 + (i * 37) % 1000);
        handle.play();
        engine.process(&mut buf, 0);
        assert!(handle.is_playing());

        handle.pause();
        engine.process(&mut buf, 0);
        assert!(buf.iter().all(|&s| s == 0.0));
    }

    handle.stop();
    engine.process(&mut buf, 0);
    assert_eq!(handle.playhead_frame(), 0);
    assert!(!handle.is_playing());
}
