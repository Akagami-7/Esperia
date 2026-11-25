//! Linear resampler for audio sample rate conversion.
//!
//! This is intentionally a simple linear interpolation resampler — sufficient
//! for initial correctness.  A higher-quality sinc resampler can replace it
//! later without changing any public API.
//!
//! # Accuracy
//! Linear interpolation introduces aliasing artefacts at high frequencies.
//! For the prototype this is acceptable; replace with a windowed-sinc or
//! rubato-based resampler before shipping to end users.

use crate::error::MediaError;

/// Resample interleaved `f32` audio from `from_rate` to `to_rate`.
///
/// `channels` is the number of interleaved channels in `input`.
/// Returns a new `Vec<f32>` at the target sample rate.
///
/// Real-time safety: **not** safe — allocates.  Call from a background thread.
pub fn resample_linear(
    input: &[f32],
    from_rate: u32,
    to_rate: u32,
    channels: u16,
) -> Result<Vec<f32>, MediaError> {
    if from_rate == 0 || to_rate == 0 {
        return Err(MediaError::InvalidResamplerParams { from_rate, to_rate });
    }
    if from_rate == to_rate {
        return Ok(input.to_vec());
    }

    let ch = channels.max(1) as usize;
    let in_frames = input.len() / ch;
    if in_frames == 0 {
        return Ok(Vec::new());
    }

    // Output frame count: scale proportionally, then add 1 to cover the last frame.
    let out_frames = (in_frames as u64 * to_rate as u64).div_ceil(from_rate as u64) as usize;

    let mut output = vec![0.0f32; out_frames * ch];

    let ratio = from_rate as f64 / to_rate as f64; // source step per output frame

    for out_frame in 0..out_frames {
        let src_pos = out_frame as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = (src_pos - src_idx as f64) as f32;

        let src_a = src_idx.min(in_frames - 1);
        let src_b = (src_idx + 1).min(in_frames - 1);

        let base_out = out_frame * ch;
        let base_a = src_a * ch;
        let base_b = src_b * ch;

        for c in 0..ch {
            let a = input[base_a + c];
            let b = input[base_b + c];
            output[base_out + c] = a + (b - a) * frac;
        }
    }

    Ok(output)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passthrough() {
        let input: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let out = resample_linear(&input, 44_100, 44_100, 1).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn upsample_doubles_length() {
        // 10 mono frames at 22 050 → 48 000 should produce ~21 frames.
        let input = vec![0.0f32; 10];
        let out = resample_linear(&input, 22_050, 44_100, 1).unwrap();
        // Should be close to 20 frames (2×).
        assert!(out.len() >= 19 && out.len() <= 21, "got {}", out.len());
    }

    #[test]
    fn downsample_halves_length() {
        let input = vec![0.5f32; 100]; // 100 mono frames at 48 kHz
        let out = resample_linear(&input, 48_000, 24_000, 1).unwrap();
        // Should be ~50 frames.
        assert!(out.len() >= 49 && out.len() <= 51, "got {}", out.len());
    }

    #[test]
    fn stereo_resample_preserves_channel_interleave() {
        // 4 stereo frames: [L0,R0, L1,R1, L2,R2, L3,R3]
        let input: Vec<f32> = vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
        let out = resample_linear(&input, 44_100, 44_100, 2).unwrap();
        assert_eq!(out, input);
        // All left channels should be positive.
        for i in (0..out.len()).step_by(2) {
            assert!(out[i] >= 0.0);
        }
    }

    #[test]
    fn zero_input_produces_empty_output() {
        let out = resample_linear(&[], 44_100, 48_000, 2).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_rate_returns_error() {
        let r = resample_linear(&[0.0], 0, 48_000, 1);
        assert!(matches!(r, Err(MediaError::InvalidResamplerParams { .. })));
    }
}
