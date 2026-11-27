//! Master output bus with defined headroom behaviour and metering.

use crate::gain::Gain;
use crate::meter::AtomicMeter;
use std::sync::Arc;

/// Master output channel strip.
pub struct MasterChannel {
    pub gain: Gain,
    pub muted: bool,
    /// Headroom ceiling (linear amplitude). Samples exceeding this are soft/hard
    /// limited when `limit_headroom` is enabled, and always flagged on the meter.
    pub headroom_ceiling: f32,
    pub limit_headroom: bool,
    pub meter: Arc<AtomicMeter>,
    buffer: Vec<f32>,
}

impl MasterChannel {
    pub fn new(gain_db: f32, max_frames: usize) -> Self {
        Self {
            gain: Gain::from_db(gain_db),
            muted: false,
            headroom_ceiling: 1.0,
            limit_headroom: false,
            meter: Arc::new(AtomicMeter::new()),
            buffer: vec![0.0f32; max_frames.max(64) * 2],
        }
    }

    #[inline]
    pub fn clear(&mut self, frames: usize) {
        let len = (frames * 2).min(self.buffer.len());
        self.buffer[..len].fill(0.0);
    }

    #[inline]
    pub fn accumulate(&mut self, src: &[f32], frames: usize) {
        let len = (frames * 2).min(self.buffer.len()).min(src.len());
        for (destination, source) in self.buffer.iter_mut().zip(src).take(len) {
            *destination += source;
        }
    }

    /// Apply master gain, update master peak/RMS/clip meter, apply optional
    /// headroom ceiling, and write to interleaved `output` (`out_channels` >= 1).
    #[inline]
    pub fn process_to_output(&mut self, frames: usize, out_channels: usize, output: &mut [f32]) {
        let ch = out_channels.max(1);
        let actual_frames = frames.min(self.buffer.len() / 2).min(output.len() / ch);
        let stereo_len = actual_frames * 2;

        if self.muted {
            self.buffer[..stereo_len].fill(0.0);
            self.meter
                .measure_stereo(&self.buffer[..stereo_len], actual_frames);
            output.fill(0.0);
            return;
        }

        let g = self.gain.linear();
        for s in &mut self.buffer[..stereo_len] {
            *s *= g;
        }

        // Measure before optional safety limiting so overload/clipping is accurately reported.
        self.meter
            .measure_stereo(&self.buffer[..stereo_len], actual_frames);

        if self.limit_headroom && self.headroom_ceiling > 0.0 {
            let ceil = self.headroom_ceiling;
            for s in &mut self.buffer[..stereo_len] {
                *s = s.clamp(-ceil, ceil);
            }
        }

        for f in 0..actual_frames {
            let l = self.buffer[f * 2];
            let r = self.buffer[f * 2 + 1];
            let base = f * ch;
            if ch == 1 {
                output[base] = 0.5 * (l + r);
            } else {
                output[base] = l;
                output[base + 1] = r;
                for c in 2..ch {
                    output[base + c] = 0.0;
                }
            }
        }
    }
}
