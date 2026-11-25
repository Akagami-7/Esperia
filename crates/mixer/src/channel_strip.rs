//! Channel strip DSP: preallocated input buffer, gain, constant-power pan,
//! mute/solo, metering, and output routing.

use crate::channel::ChannelConfig;
use crate::gain::Gain;
use crate::meter::AtomicMeter;
use crate::panning::StereoPan;
use crate::routing::{RouteTarget, SendRoute};
use std::sync::Arc;

/// A single mixer channel strip with preallocated stereo processing buffer.
pub struct ChannelStrip {
    pub id: u64,
    pub name: String,
    pub gain: Gain,
    pub pan: StereoPan,
    pub muted: bool,
    pub soloed: bool,
    pub output: RouteTarget,
    pub sends: Vec<SendRoute>,
    pub meter: Arc<AtomicMeter>,
    /// Preallocated interleaved stereo input/processing buffer (`max_frames * 2`).
    buffer: Vec<f32>,
}

impl ChannelStrip {
    pub fn from_config(cfg: &ChannelConfig, max_frames: usize) -> Self {
        Self {
            id: cfg.id,
            name: cfg.name.clone(),
            gain: Gain::from_db(cfg.gain_db),
            pan: StereoPan::from_pan(cfg.pan),
            muted: cfg.muted,
            soloed: cfg.soloed,
            output: cfg.output,
            sends: cfg.sends.clone(),
            meter: Arc::new(AtomicMeter::new()),
            buffer: vec![0.0f32; max_frames.max(64) * 2],
        }
    }

    /// Clear the preallocated input buffer for `frames` stereo frames.
    /// Real-time safe: zero allocations.
    #[inline]
    pub fn clear(&mut self, frames: usize) {
        let len = (frames * 2).min(self.buffer.len());
        self.buffer[..len].fill(0.0);
    }

    /// Mutable access to the preallocated interleaved stereo input buffer.
    #[inline]
    pub fn input_buffer_mut(&mut self, frames: usize) -> &mut [f32] {
        let len = (frames * 2).min(self.buffer.len());
        &mut self.buffer[..len]
    }

    /// Read-only access to the processed stereo buffer (used for pre-fader or
    /// post-fader routing summation).
    #[inline]
    pub fn buffer(&self, frames: usize) -> &[f32] {
        let len = (frames * 2).min(self.buffer.len());
        &self.buffer[..len]
    }

    /// Apply channel gain and constant-power stereo panning in-place, and update
    /// the channel's peak/RMS/clip meter.
    ///
    /// If `audible` is `false` (muted or non-soloed when another channel is soloed),
    /// the buffer is zeroed after metering silence.
    #[inline]
    pub fn process_in_place(&mut self, frames: usize, audible: bool) {
        let len = (frames * 2).min(self.buffer.len());
        let actual_frames = len / 2;

        if !audible {
            self.buffer[..len].fill(0.0);
            self.meter
                .measure_stereo(&self.buffer[..len], actual_frames);
            return;
        }

        let g = self.gain.linear();
        let (pan_l, pan_r) = self.pan.gains();
        // Normalize center pan (+3 dB compensation so unity 0dB center mono/stereo
        // preserves level: sqrt(2) * 0.7071 ≈ 1.0).
        const CENTER_COMP: f32 = std::f32::consts::SQRT_2;
        let gl = g * pan_l * CENTER_COMP;
        let gr = g * pan_r * CENTER_COMP;

        for f in 0..actual_frames {
            let idx = f * 2;
            self.buffer[idx] *= gl;
            self.buffer[idx + 1] *= gr;
        }

        self.meter
            .measure_stereo(&self.buffer[..len], actual_frames);
    }
}
