//! Instantaneous peak, RMS, and clipping/overload metering.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Snapshot of stereo peak, RMS, and overload state for UI display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MeterReading {
    pub peak_left: f32,
    pub peak_right: f32,
    pub rms_left: f32,
    pub rms_right: f32,
    pub clipped: bool,
}

/// Lock-free atomic meter updated by the audio thread and read by the UI thread.
pub struct AtomicMeter {
    peak_left_bits: AtomicU32,
    peak_right_bits: AtomicU32,
    rms_left_bits: AtomicU32,
    rms_right_bits: AtomicU32,
    clipped: AtomicBool,
}

impl AtomicMeter {
    pub fn new() -> Self {
        Self {
            peak_left_bits: AtomicU32::new(0.0f32.to_bits()),
            peak_right_bits: AtomicU32::new(0.0f32.to_bits()),
            rms_left_bits: AtomicU32::new(0.0f32.to_bits()),
            rms_right_bits: AtomicU32::new(0.0f32.to_bits()),
            clipped: AtomicBool::new(false),
        }
    }

    /// Analyze an interleaved stereo buffer (`[L0, R0, L1, R1, ...]`) and publish
    /// peak, RMS, and clipping state atomically. Real-time safe.
    #[inline]
    pub fn measure_stereo(&self, interleaved: &[f32], frames: usize) {
        if frames == 0 || interleaved.len() < frames * 2 {
            self.store(MeterReading::default());
            return;
        }
        let mut pk_l = 0.0f32;
        let mut pk_r = 0.0f32;
        let mut sum_sq_l = 0.0f32;
        let mut sum_sq_r = 0.0f32;

        for f in 0..frames {
            let l = interleaved[f * 2];
            let r = interleaved[f * 2 + 1];
            pk_l = pk_l.max(l.abs());
            pk_r = pk_r.max(r.abs());
            sum_sq_l += l * l;
            sum_sq_r += r * r;
        }

        let inv_n = 1.0 / frames as f32;
        let reading = MeterReading {
            peak_left: pk_l,
            peak_right: pk_r,
            rms_left: (sum_sq_l * inv_n).sqrt(),
            rms_right: (sum_sq_r * inv_n).sqrt(),
            clipped: pk_l > 1.0 || pk_r > 1.0,
        };
        self.store(reading);
    }

    #[inline]
    fn store(&self, reading: MeterReading) {
        self.peak_left_bits
            .store(reading.peak_left.to_bits(), Ordering::Release);
        self.peak_right_bits
            .store(reading.peak_right.to_bits(), Ordering::Release);
        self.rms_left_bits
            .store(reading.rms_left.to_bits(), Ordering::Release);
        self.rms_right_bits
            .store(reading.rms_right.to_bits(), Ordering::Release);
        if reading.clipped {
            self.clipped.store(true, Ordering::Release);
        }
    }

    pub fn read(&self) -> MeterReading {
        MeterReading {
            peak_left: f32::from_bits(self.peak_left_bits.load(Ordering::Acquire)),
            peak_right: f32::from_bits(self.peak_right_bits.load(Ordering::Acquire)),
            rms_left: f32::from_bits(self.rms_left_bits.load(Ordering::Acquire)),
            rms_right: f32::from_bits(self.rms_right_bits.load(Ordering::Acquire)),
            clipped: self.clipped.load(Ordering::Acquire),
        }
    }

    pub fn clear_clip(&self) {
        self.clipped.store(false, Ordering::Release);
    }
}

impl Default for AtomicMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_rms_and_clip_detection() {
        let meter = AtomicMeter::new();
        // Constant 0.5 L, 0.25 R
        let buf = [0.5f32, 0.25, 0.5, 0.25];
        meter.measure_stereo(&buf, 2);
        let r = meter.read();
        assert!((r.peak_left - 0.5).abs() < 1e-6);
        assert!((r.rms_left - 0.5).abs() < 1e-6);
        assert!((r.peak_right - 0.25).abs() < 1e-6);
        assert!(!r.clipped);

        let hot = [1.2f32, 0.5];
        meter.measure_stereo(&hot, 1);
        assert!(meter.read().clipped);
        meter.clear_clip();
        assert!(!meter.read().clipped);
    }
}
