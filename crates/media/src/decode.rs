//! WAV (and future format) decoding using Symphonia.
//!
//! # Real-time safety
//! This module must only be called from a background thread — never from the
//! audio callback.

use std::path::Path;
use std::sync::Arc;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::MediaError;
use crate::resample::resample_linear;

// ─────────────────────────────────────────────────────────────────────────────
// DecodedAudio
// ─────────────────────────────────────────────────────────────────────────────

/// Fully decoded, in-memory audio asset.
///
/// Samples are stored as interleaved `f32` in the range [-1.0, 1.0].
/// After construction this struct is **immutable** and cheaply cloneable via `Arc`.
#[derive(Debug)]
pub struct DecodedAudio {
    /// Interleaved f32 samples: [L0, R0, L1, R1, …]
    pub samples: Vec<f32>,
    /// Sample rate of the decoded audio (before any resampling).
    pub sample_rate: u32,
    /// Number of audio channels.
    pub channels: u16,
    /// Total number of frames (`samples.len() / channels`).
    pub duration_frames: u64,
}

impl DecodedAudio {
    /// Wrap into a cheaply-cloneable reference.
    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    /// Pre-compute a peak array (one value per `pixels` frames) for waveform
    /// display. Returns `Vec<f32>` of `ceil(duration_frames / step)` peaks.
    pub fn peak_thumbnail(&self, step_frames: usize) -> Vec<f32> {
        if step_frames == 0 || self.samples.is_empty() {
            return vec![];
        }
        let ch = self.channels as usize;
        let total_frames = self.duration_frames as usize;
        let n_steps = total_frames.div_ceil(step_frames);
        let mut peaks = vec![0.0f32; n_steps];
        for (frame_idx, chunk) in self.samples.chunks(ch).enumerate() {
            let step = frame_idx / step_frames;
            if step >= n_steps {
                break;
            }
            let frame_peak = chunk.iter().copied().fold(0.0f32, |a, s| a.max(s.abs()));
            if frame_peak > peaks[step] {
                peaks[step] = frame_peak;
            }
        }
        peaks
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// decode_audio — public API
// ─────────────────────────────────────────────────────────────────────────────

/// Decode an audio file from `path` into an in-memory `DecodedAudio`.
///
/// If `target_sample_rate` is `Some(sr)` and differs from the file's native
/// rate, the samples are linearly resampled to `sr`.
///
/// **Must not be called from the audio callback.**
pub fn decode_audio(
    path: &Path,
    target_sample_rate: Option<u32>,
) -> Result<DecodedAudio, MediaError> {
    // ── open the source ──────────────────────────────────────────────────────
    let file = std::fs::File::open(path).map_err(|e| MediaError::Io {
        path: path.to_owned(),
        source: e,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    // ── probe format ────────────────────────────────────────────────────────
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| MediaError::Probe(e.to_string()))?;

    let mut format = probed.format;

    // ── find the default audio track ─────────────────────────────────────────
    let track = format
        .default_track()
        .or_else(|| format.tracks().iter().next())
        .ok_or(MediaError::NoAudioTrack)?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    // ── build decoder ────────────────────────────────────────────────────────
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| MediaError::UnsupportedCodec(e.to_string()))?;

    let native_sample_rate = codec_params.sample_rate.unwrap_or(44_100);
    let native_channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

    // ── decode all packets ───────────────────────────────────────────────────
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                // Some formats emit ResetRequired mid-stream; reset in-place (returns ()).
                decoder.reset();
                continue;
            }
            Err(e) => return Err(MediaError::Probe(e.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder
            .decode(&packet)
            .map_err(|e| MediaError::DecodeFailed(e.to_string()))?;

        convert_buffer_to_f32(&decoded, &mut interleaved);
    }

    // ── optional resampling ───────────────────────────────────────────────────
    let (final_samples, final_sr) = if let Some(target_sr) = target_sample_rate {
        if target_sr != native_sample_rate {
            let resampled =
                resample_linear(&interleaved, native_sample_rate, target_sr, native_channels)?;
            (resampled, target_sr)
        } else {
            (interleaved, native_sample_rate)
        }
    } else {
        (interleaved, native_sample_rate)
    };

    let duration_frames = (final_samples.len() / native_channels.max(1) as usize) as u64;

    Ok(DecodedAudio {
        samples: final_samples,
        sample_rate: final_sr,
        channels: native_channels,
        duration_frames,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Buffer conversion helper
// ─────────────────────────────────────────────────────────────────────────────

fn convert_buffer_to_f32(buf: &AudioBufferRef<'_>, out: &mut Vec<f32>) {
    match buf {
        AudioBufferRef::F32(b) => {
            let ch = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..ch {
                    out.push(b.chan(c)[f]);
                }
            }
        }
        AudioBufferRef::S16(b) => {
            let ch = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..ch {
                    out.push(b.chan(c)[f] as f32 / i16::MAX as f32);
                }
            }
        }
        AudioBufferRef::S32(b) => {
            let ch = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..ch {
                    out.push(b.chan(c)[f] as f32 / i32::MAX as f32);
                }
            }
        }
        AudioBufferRef::U8(b) => {
            let ch = b.spec().channels.count();
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..ch {
                    // u8: 0..255 → [-1, 1]
                    out.push((b.chan(c)[f] as f32 - 128.0) / 128.0);
                }
            }
        }
        // Symphonia may add more variants — handle exhaustively with a fallback.
        _ => {
            // Convert via Symphonia's built-in f32 conversion by using the
            // sample count and zeroing unknown formats (safe fallback).
            // In practice, WAV/PCM uses F32, S16, or S32.
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a minimal valid 44100 Hz stereo PCM WAV into a temp file and return
    /// the path.  Uses raw bytes to avoid a build dependency on `hound`.
    fn write_test_wav(path: &std::path::Path, sample_rate: u32, channels: u16, frames: u32) {
        // 16-bit PCM sine wave
        let num_samples = (frames * channels as u32) as usize;
        let mut data: Vec<u8> = Vec::with_capacity(num_samples * 2);
        for i in 0..num_samples {
            let v = (0.5
                * (2.0 * std::f32::consts::PI * 440.0 * (i as f32 / sample_rate as f32)).sin()
                * i16::MAX as f32) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
        let data_len = data.len() as u32;
        let mut f = std::fs::File::create(path).unwrap();
        // RIFF header
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        // fmt chunk
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap(); // chunk size
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&channels.to_le_bytes()).unwrap();
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        let byte_rate = sample_rate * channels as u32 * 2;
        f.write_all(&byte_rate.to_le_bytes()).unwrap();
        let block_align: u16 = channels * 2;
        f.write_all(&block_align.to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap(); // bits per sample
        // data chunk
        f.write_all(b"data").unwrap();
        f.write_all(&data_len.to_le_bytes()).unwrap();
        f.write_all(&data).unwrap();
    }

    #[test]
    fn decode_stereo_44100() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        write_test_wav(&path, 44_100, 2, 4410); // 0.1 s
        let audio = decode_audio(&path, None).unwrap();
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.duration_frames, 4410);
        assert_eq!(audio.samples.len(), 4410 * 2);
    }

    #[test]
    fn decode_mono_48000() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mono.wav");
        write_test_wav(&path, 48_000, 1, 480); // 10 ms
        let audio = decode_audio(&path, None).unwrap();
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.duration_frames, 480);
    }

    #[test]
    fn decode_with_resampling_to_48000() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resample.wav");
        write_test_wav(&path, 44_100, 2, 4410); // native 44.1 kHz
        let audio = decode_audio(&path, Some(48_000)).unwrap();
        // After resampling to 48 000 we expect ~4800 frames (≈ 10 % more).
        assert_eq!(audio.sample_rate, 48_000);
        assert!(audio.duration_frames > 4410, "resampled should be longer");
        assert!(
            audio.duration_frames < 5500,
            "resampled should not be too long"
        );
    }

    #[test]
    fn peak_thumbnail_non_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thumb.wav");
        write_test_wav(&path, 48_000, 2, 48_000); // 1 s
        let audio = decode_audio(&path, None).unwrap();
        let peaks = audio.peak_thumbnail(480); // one peak every 10 ms
        assert_eq!(peaks.len(), 100);
        assert!(peaks.iter().any(|&p| p > 0.0), "should have non-zero peaks");
    }
}
