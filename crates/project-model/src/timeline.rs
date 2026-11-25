//! Project timeline configuration and sample-frame timebase utilities.

use crate::error::ProjectError;
use serde::{Deserialize, Serialize};

/// Minimum allowed project tempo in BPM.
pub const MIN_TEMPO_BPM: f64 = 20.0;
/// Maximum allowed project tempo in BPM.
pub const MAX_TEMPO_BPM: f64 = 300.0;

/// Global timeline settings for a project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub sample_rate: u32,
    pub tempo_bpm: f64,
    pub time_signature_numerator: u8,
    pub time_signature_denominator: u8,
    /// Optional loop region `(start_frame, end_frame)`.
    pub loop_range: Option<(u64, u64)>,
}

impl Timeline {
    /// Create a validated `Timeline` with default 4/4 time signature.
    pub fn new(sample_rate: u32, tempo_bpm: f64) -> Result<Self, ProjectError> {
        Self::validate_sample_rate(sample_rate)?;
        Self::validate_tempo(tempo_bpm)?;
        Ok(Self {
            sample_rate,
            tempo_bpm,
            time_signature_numerator: 4,
            time_signature_denominator: 4,
            loop_range: None,
        })
    }

    /// Validate project sample rate.
    pub fn validate_sample_rate(sample_rate: u32) -> Result<u32, ProjectError> {
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(ProjectError::InvalidSampleRate(sample_rate));
        }
        Ok(sample_rate)
    }

    /// Validate project tempo in BPM.
    pub fn validate_tempo(tempo_bpm: f64) -> Result<f64, ProjectError> {
        if !tempo_bpm.is_finite() || !(MIN_TEMPO_BPM..=MAX_TEMPO_BPM).contains(&tempo_bpm) {
            return Err(ProjectError::InvalidTempo(tempo_bpm));
        }
        Ok(tempo_bpm)
    }

    /// Validate time signature `(numerator, denominator)`.
    pub fn validate_time_signature(num: u8, den: u8) -> Result<(u8, u8), ProjectError> {
        let valid_den = matches!(den, 1 | 2 | 4 | 8 | 16 | 32);
        if !(1..=32).contains(&num) || !valid_den {
            return Err(ProjectError::InvalidTimeSignature(num, den));
        }
        Ok((num, den))
    }

    /// Convert a sample frame index to elapsed seconds.
    pub fn frames_to_seconds(&self, frames: u64) -> f64 {
        frames as f64 / self.sample_rate as f64
    }

    /// Convert elapsed seconds to sample frames (rounded to nearest frame).
    pub fn seconds_to_frames(&self, seconds: f64) -> u64 {
        if seconds <= 0.0 || !seconds.is_finite() {
            return 0;
        }
        (seconds * self.sample_rate as f64).round() as u64
    }
}
