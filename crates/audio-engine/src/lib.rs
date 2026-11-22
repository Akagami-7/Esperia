use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// A mathematically continuous, real-time-safe sine wave generator.
#[derive(Debug, Clone)]
pub struct SineGenerator {
    pub frequency: f32,
    pub amplitude: f32,
    pub sample_rate: f32,
    pub channels: usize,
    phase: f32,
    is_playing: bool,
}

impl SineGenerator {
    pub fn new(frequency: f32, amplitude: f32, sample_rate: f32, channels: usize) -> Self {
        Self {
            frequency: frequency.clamp(20.0, 20000.0),
            amplitude: amplitude.clamp(0.0, 1.0),
            sample_rate: if sample_rate > 0.0 {
                sample_rate
            } else {
                48000.0
            },
            channels: channels.max(1),
            phase: 0.0,
            is_playing: false,
        }
    }

    #[inline]
    pub fn set_frequency(&mut self, freq: f32) {
        self.frequency = freq.clamp(20.0, 20000.0);
    }

    #[inline]
    pub fn set_amplitude(&mut self, amp: f32) {
        self.amplitude = amp.clamp(0.0, 1.0);
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sr: f32) {
        if sr > 0.0 {
            self.sample_rate = sr;
        }
    }

    #[inline]
    pub fn set_channels(&mut self, ch: usize) {
        self.channels = ch.max(1);
    }

    #[inline]
    pub fn set_playing(&mut self, playing: bool) {
        self.is_playing = playing;
    }

    #[inline]
    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Renders interleaved audio frames directly into `output`.
    /// Zero heap allocations, pure mathematical evaluation.
    #[inline]
    pub fn render_interleaved(&mut self, output: &mut [f32], channels: usize) {
        let ch = channels.max(1);
        if !self.is_playing || self.amplitude <= 0.0 {
            for sample in output.iter_mut() {
                *sample = 0.0;
            }
            return;
        }

        let phase_step = (TAU * self.frequency) / self.sample_rate;
        let mut p = self.phase;
        let amp = self.amplitude;

        let frames = output.len() / ch;
        for i in 0..frames {
            let sample_value = amp * p.sin();
            let base_idx = i * ch;
            for c in 0..ch {
                output[base_idx + c] = sample_value;
            }

            p += phase_step;
            if p >= TAU {
                p -= TAU;
            }
        }

        self.phase = p;
    }
}

/// Thread-safe, lock-free control interface to communicate between UI and Audio thread.
#[derive(Debug, Clone)]
pub struct EngineControl {
    is_playing: Arc<AtomicBool>,
    frequency_bits: Arc<AtomicU32>,
    amplitude_bits: Arc<AtomicU32>,
}

impl EngineControl {
    pub fn new(initial_freq: f32, initial_amp: f32) -> Self {
        Self {
            is_playing: Arc::new(AtomicBool::new(false)),
            frequency_bits: Arc::new(AtomicU32::new(initial_freq.to_bits())),
            amplitude_bits: Arc::new(AtomicU32::new(initial_amp.to_bits())),
        }
    }

    pub fn play(&self) {
        self.is_playing.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.is_playing.store(false, Ordering::Release);
    }

    pub fn toggle_play(&self) -> bool {
        let current = self.is_playing.load(Ordering::Acquire);
        self.is_playing.store(!current, Ordering::Release);
        !current
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Acquire)
    }

    pub fn set_frequency(&self, freq: f32) {
        let clamped = freq.clamp(20.0, 20000.0);
        self.frequency_bits
            .store(clamped.to_bits(), Ordering::Release);
    }

    pub fn get_frequency(&self) -> f32 {
        f32::from_bits(self.frequency_bits.load(Ordering::Acquire))
    }

    pub fn set_amplitude(&self, amp: f32) {
        let clamped = amp.clamp(0.0, 1.0);
        self.amplitude_bits
            .store(clamped.to_bits(), Ordering::Release);
    }

    pub fn get_amplitude(&self) -> f32 {
        f32::from_bits(self.amplitude_bits.load(Ordering::Acquire))
    }
}

/// The core real-time audio engine handler.
pub struct AudioEngine {
    generator: SineGenerator,
    control: EngineControl,
    channels: usize,
}

impl AudioEngine {
    pub fn new(sample_rate: f32, channels: u16) -> (Self, EngineControl) {
        let control = EngineControl::new(440.0, 0.1);
        let ch = (channels as usize).max(1);
        let generator = SineGenerator::new(440.0, 0.1, sample_rate, ch);

        let engine = Self {
            generator,
            control: control.clone(),
            channels: ch,
        };

        (engine, control)
    }

    pub fn reconfigure(&mut self, sample_rate: f32, channels: u16) {
        let ch = (channels as usize).max(1);
        self.channels = ch;
        self.generator.set_sample_rate(sample_rate);
        self.generator.set_channels(ch);
    }

    /// Executed within the real-time audio callback.
    /// Strictly no allocations or blocking calls.
    pub fn process(&mut self, output: &mut [f32]) {
        let playing = self.control.is_playing();
        let freq = self.control.get_frequency();
        let amp = self.control.get_amplitude();

        self.generator.set_playing(playing);
        self.generator.set_frequency(freq);
        self.generator.set_amplitude(amp);

        self.generator.render_interleaved(output, self.channels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sine_generator_silence_when_stopped() {
        let mut generator = SineGenerator::new(440.0, 0.5, 48000.0, 2);
        generator.set_playing(false);
        let mut buffer = [1.0f32; 128];
        generator.render_interleaved(&mut buffer, 2);

        for s in buffer {
            assert_eq!(s, 0.0);
        }
    }

    #[test]
    fn test_sine_generator_produces_values_when_playing() {
        let mut generator = SineGenerator::new(440.0, 0.2, 48000.0, 2);
        generator.set_playing(true);
        let mut buffer = [0.0f32; 128];
        generator.render_interleaved(&mut buffer, 2);

        let max_val = buffer.iter().copied().fold(0.0f32, f32::max);
        assert!(max_val > 0.0);
        assert!(max_val <= 0.2001);
    }

    #[test]
    fn test_phase_continuity_across_buffers() {
        let mut generator = SineGenerator::new(440.0, 1.0, 48000.0, 2);
        generator.set_playing(true);

        let mut buf1 = [0.0f32; 64];
        let mut buf2 = [0.0f32; 64];

        generator.render_interleaved(&mut buf1, 2);
        generator.render_interleaved(&mut buf2, 2);

        let last_sample = buf1[62];
        let first_sample = buf2[0];
        let diff = (first_sample - last_sample).abs();
        assert!(diff < 0.2, "Phase jump detected between blocks: {diff}");
    }

    #[test]
    fn test_engine_control_thread_safe_values() {
        let (mut engine, control) = AudioEngine::new(48000.0, 2);
        assert!(!control.is_playing());

        control.play();
        assert!(control.is_playing());

        control.set_frequency(880.0);
        assert_eq!(control.get_frequency(), 880.0);

        control.set_amplitude(0.5);
        assert_eq!(control.get_amplitude(), 0.5);

        let mut buffer = [0.0f32; 128];
        engine.process(&mut buffer);

        let max_val = buffer.iter().copied().fold(0.0f32, f32::max);
        assert!(max_val > 0.0);
    }

    #[test]
    fn test_sine_generator_various_sample_rates() {
        for &sr in &[44100.0, 48000.0, 88200.0, 96000.0, 192000.0] {
            let mut generator = SineGenerator::new(1000.0, 0.5, sr, 2);
            generator.set_playing(true);
            let mut buffer = [0.0f32; 256];
            generator.render_interleaved(&mut buffer, 2);
            let max_val = buffer.iter().copied().fold(0.0f32, f32::max);
            assert!(max_val > 0.0 && max_val <= 0.5001);
        }
    }

    #[test]
    fn test_sine_generator_clamping() {
        let mut generator = SineGenerator::new(-100.0, -0.5, 48000.0, 2);
        assert_eq!(generator.frequency, 20.0);
        assert_eq!(generator.amplitude, 0.0);

        generator.set_frequency(50000.0);
        assert_eq!(generator.frequency, 20000.0);

        generator.set_amplitude(5.0);
        assert_eq!(generator.amplitude, 1.0);
    }

    #[test]
    fn test_multichannel_interleaved() {
        for &ch in &[1, 2, 4, 6] {
            let mut generator = SineGenerator::new(440.0, 0.4, 48000.0, ch);
            generator.set_playing(true);
            let mut buffer = vec![0.0f32; ch * 64];
            generator.render_interleaved(&mut buffer, ch);

            // In an interleaved frame, all channels share the same tone value
            for frame in 0..64 {
                let first = buffer[frame * ch];
                for c in 1..ch {
                    assert_eq!(buffer[frame * ch + c], first);
                }
            }
        }
    }

    #[test]
    fn test_various_buffer_sizes() {
        let mut generator = SineGenerator::new(440.0, 0.5, 48000.0, 2);
        generator.set_playing(true);

        for &size in &[16, 32, 64, 128, 256, 512, 1024, 2048] {
            let mut buffer = vec![0.0f32; size * 2];
            generator.render_interleaved(&mut buffer, 2);
            let max_val = buffer.iter().copied().fold(0.0f32, f32::max);
            assert!(max_val > 0.0 && max_val <= 0.5001);
        }
    }
}
