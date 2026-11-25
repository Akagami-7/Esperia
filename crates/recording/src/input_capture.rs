//! Real-time audio input callback handler.
//!
//! # Real-time safety contract
//! * Zero heap allocations inside `on_input_samples()`.
//! * Zero disk I/O and zero blocking mutexes.
//! * Uses a preallocated scratch buffer to extract selected input channels.

use crate::buffer_manager::CaptureRingBuffer;
use crate::recording_state::{InputChannelSelection, PunchRegion, RecordingTelemetry};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Maximum frames expected in a single hardware input callback.
const MAX_CALLBACK_FRAMES: usize = 4096;
/// Maximum output channels per captured stream.
const MAX_CAPTURE_CHANNELS: usize = 8;

/// Real-time input capture processor owned by the CPAL input stream closure.
pub struct InputCapture {
    ring: Arc<CaptureRingBuffer>,
    monitor_ring: Option<Arc<CaptureRingBuffer>>,
    telemetry: Arc<RecordingTelemetry>,
    channel_selection: InputChannelSelection,
    device_channels: u16,
    punch: PunchRegion,
    monitoring_enabled: Arc<AtomicBool>,
    /// Preallocated scratch buffer for channel extraction (never resized in callback).
    scratch: Vec<f32>,
    last_captured_len: usize,
}

impl InputCapture {
    pub fn new(
        ring: Arc<CaptureRingBuffer>,
        monitor_ring: Option<Arc<CaptureRingBuffer>>,
        telemetry: Arc<RecordingTelemetry>,
        channel_selection: InputChannelSelection,
        device_channels: u16,
        punch: PunchRegion,
        monitoring_enabled: Arc<AtomicBool>,
    ) -> Self {
        let scratch = vec![0.0f32; MAX_CALLBACK_FRAMES * MAX_CAPTURE_CHANNELS];
        Self {
            ring,
            monitor_ring,
            telemetry,
            channel_selection,
            device_channels: device_channels.max(1),
            punch,
            monitoring_enabled,
            scratch,
            last_captured_len: 0,
        }
    }

    /// Slice of samples extracted during the most recent `on_input_samples()` call.
    pub fn last_captured_slice(&self) -> &[f32] {
        &self.scratch[..self.last_captured_len]
    }

    /// Process a hardware input buffer in the real-time audio callback.
    ///
    /// `timeline_frame` is the current playback timeline frame (for punch-in gating).
    /// `input_latency_ns` is the hardware ADC-to-callback latency if reported by CPAL.
    pub fn on_input_samples(
        &mut self,
        input: &[f32],
        timeline_frame: u64,
        input_latency_ns: Option<u64>,
    ) {
        if let Some(ns) = input_latency_ns {
            self.telemetry
                .last_input_latency_ns
                .store(ns, Ordering::Relaxed);
        }

        let dev_ch = self.device_channels as usize;
        let in_frames = input.len() / dev_ch;
        if in_frames == 0 {
            return;
        }

        let out_ch = self
            .channel_selection
            .output_channels(self.device_channels)
            .min(MAX_CAPTURE_CHANNELS as u16) as usize;

        // Bound frame count to preallocated scratch size so we never allocate.
        let frames = in_frames.min(self.scratch.len() / out_ch.max(1));
        let mut peak = 0.0f32;

        for f in 0..frames {
            let in_base = f * dev_ch;
            let out_base = f * out_ch;
            match self.channel_selection {
                InputChannelSelection::Mono(ch_idx) => {
                    let c = (ch_idx as usize).min(dev_ch - 1);
                    let s = input[in_base + c];
                    peak = peak.max(s.abs());
                    self.scratch[out_base] = s;
                }
                InputChannelSelection::StereoPair(first_ch) => {
                    let c0 = (first_ch as usize).min(dev_ch - 1);
                    let c1 = (first_ch as usize + 1).min(dev_ch - 1);
                    let s0 = input[in_base + c0];
                    peak = peak.max(s0.abs());
                    self.scratch[out_base] = s0;
                    if out_ch > 1 {
                        let s1 = input[in_base + c1];
                        peak = peak.max(s1.abs());
                        self.scratch[out_base + 1] = s1;
                    }
                }
                InputChannelSelection::All => {
                    for c in 0..out_ch {
                        let s = input[in_base + c];
                        peak = peak.max(s.abs());
                        self.scratch[out_base + c] = s;
                    }
                }
            }
        }

        self.telemetry
            .input_peak_bits
            .store(peak.to_bits(), Ordering::Release);

        self.last_captured_len = frames * out_ch;
        let captured_slice = &self.scratch[..self.last_captured_len];

        // Forward to input monitoring ring if enabled (even when not recording,
        // so armed tracks can be auditioned).
        if self.monitoring_enabled.load(Ordering::Acquire)
            && let Some(ref mon) = self.monitor_ring
        {
            let _ = mon.push_samples(captured_slice);
        }

        // Only push to the recording ring when recording is active and inside punch window.
        if !self.telemetry.is_recording.load(Ordering::Acquire) {
            return;
        }
        if !self.punch.allows_capture(timeline_frame) {
            return;
        }

        match self.ring.push_samples(captured_slice) {
            Ok(written) => {
                let written_frames = (written / out_ch.max(1)) as u64;
                self.telemetry
                    .captured_frames
                    .fetch_add(written_frames, Ordering::Relaxed);
            }
            Err(dropped) => {
                let written_samples = captured_slice.len().saturating_sub(dropped);
                let written_frames = (written_samples / out_ch.max(1)) as u64;
                self.telemetry
                    .captured_frames
                    .fetch_add(written_frames, Ordering::Relaxed);
                self.telemetry
                    .overrun_samples
                    .fetch_add(dropped as u64, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_channel_extraction_selects_requested_channel() {
        let ring = Arc::new(CaptureRingBuffer::new(256));
        let tel = Arc::new(RecordingTelemetry::new());
        tel.is_recording.store(true, Ordering::Release);
        let mon_flag = Arc::new(AtomicBool::new(false));

        // 2-channel device, select channel 1 (right).
        let mut capture = InputCapture::new(
            Arc::clone(&ring),
            None,
            Arc::clone(&tel),
            InputChannelSelection::Mono(1),
            2,
            PunchRegion::default(),
            mon_flag,
        );

        // Interleaved stereo: L=0.1, R=0.8
        let hw = [0.1f32, 0.8, 0.2, 0.9];
        capture.on_input_samples(&hw, 0, Some(2_000_000));

        let mut out = [0.0f32; 4];
        let n = ring.pop_samples(&mut out);
        assert_eq!(n, 2);
        assert_eq!(&out[..2], &[0.8, 0.9]);
        assert_eq!(tel.captured_frames(), 2);
        assert_eq!(tel.last_input_latency_ns.load(Ordering::Acquire), 2_000_000);
    }

    #[test]
    fn punch_region_gates_capture() {
        let ring = Arc::new(CaptureRingBuffer::new(256));
        let tel = Arc::new(RecordingTelemetry::new());
        tel.is_recording.store(true, Ordering::Release);
        let mon_flag = Arc::new(AtomicBool::new(false));

        let punch = PunchRegion {
            enabled: true,
            start_frame: 100,
            end_frame: 200,
        };
        let mut capture = InputCapture::new(
            Arc::clone(&ring),
            None,
            Arc::clone(&tel),
            InputChannelSelection::Mono(0),
            1,
            punch,
            mon_flag,
        );

        let hw = [0.5f32; 4];
        capture.on_input_samples(&hw, 50, None); // before punch
        assert_eq!(ring.available_read(), 0);

        capture.on_input_samples(&hw, 150, None); // inside punch
        assert_eq!(ring.available_read(), 4);

        capture.on_input_samples(&hw, 250, None); // after punch
        assert_eq!(ring.available_read(), 4);
    }
}
