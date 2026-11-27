//! Group / auxiliary summing bus with preallocated buffer, metering, and routing.

use crate::gain::Gain;
use crate::meter::AtomicMeter;
use crate::panning::StereoPan;
use crate::routing::{RouteTarget, SendRoute};
use std::sync::Arc;

/// Configuration for a summing/aux bus.
#[derive(Debug, Clone, PartialEq)]
pub struct BusConfig {
    pub id: u64,
    pub name: String,
    pub gain_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub output: RouteTarget,
    pub sends: Vec<SendRoute>,
}

impl BusConfig {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            gain_db: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            output: RouteTarget::Master,
            sends: Vec::new(),
        }
    }
}

/// Summing bus with preallocated stereo accumulation buffer and atomic meter.
pub struct Bus {
    pub id: u64,
    pub name: String,
    pub gain: Gain,
    pub pan: StereoPan,
    pub muted: bool,
    pub soloed: bool,
    pub output: RouteTarget,
    pub sends: Vec<SendRoute>,
    pub meter: Arc<AtomicMeter>,
    buffer: Vec<f32>,
}

impl Bus {
    pub fn from_config(cfg: &BusConfig, max_frames: usize) -> Self {
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

    #[inline]
    pub fn clear(&mut self, frames: usize) {
        let len = (frames * 2).min(self.buffer.len());
        self.buffer[..len].fill(0.0);
    }

    /// Accumulate a stereo signal scaled by `scale` into this bus.
    /// Real-time safe: zero allocations.
    #[inline]
    pub fn accumulate_scaled(&mut self, src: &[f32], frames: usize, scale: f32) {
        let len = (frames * 2).min(self.buffer.len()).min(src.len());
        for (destination, source) in self.buffer.iter_mut().zip(src).take(len) {
            *destination += source * scale;
        }
    }

    #[inline]
    pub fn buffer(&self, frames: usize) -> &[f32] {
        let len = (frames * 2).min(self.buffer.len());
        &self.buffer[..len]
    }

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
