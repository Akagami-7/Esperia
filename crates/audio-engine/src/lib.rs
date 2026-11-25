//! `esperia-audio-engine` — real-time audio processing core.
//!
//! # Architecture rules (enforced)
//! * This crate has **zero** dependency on `cpal`, `esperia-audio-io`, or any GUI crate.
//! * Every public function that enters the audio callback is real-time safe:
//!   no heap allocation, no blocking synchronisation, no I/O.
//! * Cross-thread communication uses lock-free atomics and a bounded SPSC ring.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

// ─────────────────────────────────────────────────────────────────────────────
// SPSC Command ring
// ─────────────────────────────────────────────────────────────────────────────

/// Commands that the UI thread sends to the audio engine.
/// Each variant is encoded as a fixed-size `[u32; 3]` to avoid allocations.
///
/// Encoding: `[tag, arg0_bits, arg1_bits]`
/// where `arg*_bits` are `f32::to_bits()` or raw `u32` values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EngineCommand {
    /// Start playback.
    Play,
    /// Stop playback.
    Stop,
    /// Set the sine-wave frequency (Hz). Clamped to [20, 20 000].
    SetFrequency(f32),
    /// Set the sine-wave amplitude. Clamped to [0, 1].
    SetAmplitude(f32),
    /// Reconfigure sample rate and channel count without restarting the stream.
    Reconfigure { sample_rate: f32, channels: u16 },
}

impl EngineCommand {
    fn encode(self) -> [u32; 3] {
        match self {
            Self::Play => [0, 0, 0],
            Self::Stop => [1, 0, 0],
            Self::SetFrequency(f) => [2, f.to_bits(), 0],
            Self::SetAmplitude(a) => [3, a.to_bits(), 0],
            Self::Reconfigure {
                sample_rate,
                channels,
            } => [4, sample_rate.to_bits(), channels as u32],
        }
    }

    fn decode(raw: [u32; 3]) -> Option<Self> {
        match raw[0] {
            0 => Some(Self::Play),
            1 => Some(Self::Stop),
            2 => Some(Self::SetFrequency(f32::from_bits(raw[1]))),
            3 => Some(Self::SetAmplitude(f32::from_bits(raw[1]))),
            4 => Some(Self::Reconfigure {
                sample_rate: f32::from_bits(raw[1]),
                channels: raw[2] as u16,
            }),
            _ => None,
        }
    }
}

/// Bounded, wait-free Single-Producer Single-Consumer command ring.
///
/// Capacity is `N` (must be a power of two).  The producer (UI thread) wraps
/// around and overwrites if the consumer (audio thread) falls behind — this
/// is acceptable because the audio thread must never block.
///
/// Layout: `slots[i] = [tag, arg0, arg1]` stored as three consecutive `AtomicU32`.
/// Slot selection: `index & (N - 1)`.
struct CommandRing<const N: usize> {
    /// Each slot is three `u32` words: `[tag, arg0, arg1]`.
    slots: Box<[[AtomicU32; 3]; N]>,
    /// Bit-packed `[valid_mask: N bits | head: 32 bits]` — not feasible; use
    /// two separate atomics.
    head: AtomicUsize, // producer writes here
    tail: AtomicUsize, // consumer reads here
}

impl<const N: usize> CommandRing<N> {
    fn new() -> Self {
        assert!(
            N.is_power_of_two(),
            "CommandRing capacity must be a power of two"
        );
        // SAFETY: AtomicU32 is zero-initialised, which is a valid state (tag=0 → Play,
        // but slots are only consumed after head advances past them).
        let slots: Vec<[AtomicU32; 3]> = (0..N)
            .map(|_| [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)])
            .collect();
        Self {
            slots: slots.into_boxed_slice().try_into().ok().unwrap(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Push a command (UI thread). Never blocks; oldest unread entry is
    /// silently overwritten when the ring is full.
    fn push(&self, cmd: EngineCommand) {
        let raw = cmd.encode();
        let head = self.head.load(Ordering::Relaxed);
        let slot = head & (N - 1);
        self.slots[slot][0].store(raw[0], Ordering::Relaxed);
        self.slots[slot][1].store(raw[1], Ordering::Relaxed);
        self.slots[slot][2].store(raw[2], Ordering::Release);
        self.head.store(head.wrapping_add(1), Ordering::Release);
    }

    /// Pop the next available command (audio thread). Returns `None` when empty.
    /// Real-time safe: no allocation, no blocking.
    fn pop(&self) -> Option<EngineCommand> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let slot = tail & (N - 1);
        // Acquire the write that stored arg2 last (see push).
        let _ = self.slots[slot][2].load(Ordering::Acquire);
        let raw = [
            self.slots[slot][0].load(Ordering::Relaxed),
            self.slots[slot][1].load(Ordering::Relaxed),
            self.slots[slot][2].load(Ordering::Relaxed),
        ];
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        EngineCommand::decode(raw)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Telemetry (audio → UI, lock-free atomics)
// ─────────────────────────────────────────────────────────────────────────────

/// Real-time telemetry produced by the audio callback and readable from the UI thread.
/// All fields are atomics — the audio thread never blocks on a mutex.
pub struct EngineTelemetry {
    /// Peak absolute sample value in the last rendered block (stored as f32 bits).
    pub peak_bits: AtomicU32,
    /// Number of underruns detected (callback took longer than its deadline).
    pub underrun_count: AtomicU64,
    /// Last callback wall-clock duration in nanoseconds.
    pub last_callback_ns: AtomicU64,
    /// Maximum callback duration seen so far, in nanoseconds.
    pub max_callback_ns: AtomicU64,
    /// Whether the engine is currently playing (mirrors engine state).
    pub is_playing: AtomicBool,
}

impl EngineTelemetry {
    fn new() -> Self {
        Self {
            peak_bits: AtomicU32::new(0.0f32.to_bits()),
            underrun_count: AtomicU64::new(0),
            last_callback_ns: AtomicU64::new(0),
            max_callback_ns: AtomicU64::new(0),
            is_playing: AtomicBool::new(false),
        }
    }

    /// Peak level in the last block, as a linear amplitude in [0, 1].
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.load(Ordering::Acquire))
    }

    /// DSP load: ratio of last callback duration to available window.
    /// Returns a value in [0.0, ∞); values > 1.0 indicate an overrun.
    pub fn dsp_load(&self, buffer_frames: u32, sample_rate: f32) -> f32 {
        if sample_rate <= 0.0 || buffer_frames == 0 {
            return 0.0;
        }
        let callback_ns = self.last_callback_ns.load(Ordering::Acquire);
        let budget_ns = (buffer_frames as f64 / sample_rate as f64 * 1_000_000_000.0) as u64;
        callback_ns as f32 / budget_ns.max(1) as f32
    }

    /// Number of times the callback detected an overrun.
    pub fn underruns(&self) -> u64 {
        self.underrun_count.load(Ordering::Acquire)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SineGenerator
// ─────────────────────────────────────────────────────────────────────────────

/// A mathematically continuous, real-time-safe sine wave generator.
/// Zero heap allocations. All state fits in a handful of `f32` and `bool`.
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
            frequency: frequency.clamp(20.0, 20_000.0),
            amplitude: amplitude.clamp(0.0, 1.0),
            sample_rate: sample_rate.max(1.0),
            channels: channels.max(1),
            phase: 0.0,
            is_playing: false,
        }
    }

    #[inline]
    pub fn set_frequency(&mut self, freq: f32) {
        self.frequency = freq.clamp(20.0, 20_000.0);
    }

    #[inline]
    pub fn set_amplitude(&mut self, amp: f32) {
        self.amplitude = amp.clamp(0.0, 1.0);
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr.max(1.0);
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

    /// Render interleaved audio into `output`.
    /// Real-time safe: zero allocations, bounded iteration.
    #[inline]
    pub fn render_interleaved(&mut self, output: &mut [f32], channels: usize) {
        let ch = channels.max(1);
        if !self.is_playing || self.amplitude <= 0.0 {
            output.fill(0.0);
            return;
        }

        let phase_step = (TAU * self.frequency) / self.sample_rate;
        let mut p = self.phase;
        let amp = self.amplitude;
        let frames = output.len() / ch;

        for i in 0..frames {
            let v = amp * p.sin();
            let base = i * ch;
            for c in 0..ch {
                output[base + c] = v;
            }
            p += phase_step;
            if p >= TAU {
                p -= TAU;
            }
        }
        self.phase = p;
    }

    /// Return the peak absolute sample value that would be produced for `frames`
    /// frames from the *current* phase, without advancing state.
    /// Used in tests to verify amplitude without calling `render_interleaved`.
    #[cfg(test)]
    pub fn peak_for_frames(&self, frames: usize) -> f32 {
        if !self.is_playing || self.amplitude <= 0.0 {
            return 0.0;
        }
        let phase_step = (TAU * self.frequency) / self.sample_rate;
        let mut p = self.phase;
        let mut peak = 0.0f32;
        for _ in 0..frames {
            peak = peak.max(self.amplitude * p.sin().abs());
            p += phase_step;
            if p >= TAU {
                p -= TAU;
            }
        }
        peak
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EngineHandle — the UI-side interface (clone-safe)
// ─────────────────────────────────────────────────────────────────────────────

/// Cloneable handle the UI thread holds to communicate with the audio engine.
/// Sending commands is wait-free (bounded ring, never blocks).
#[derive(Clone)]
pub struct EngineHandle {
    commands: Arc<CommandRing<64>>,
    /// Shared telemetry — readable any time by the UI thread.
    pub telemetry: Arc<EngineTelemetry>,
}

impl EngineHandle {
    /// Send a command to the audio engine. Wait-free; never panics.
    pub fn send(&self, cmd: EngineCommand) {
        self.commands.push(cmd);
    }

    pub fn play(&self) {
        self.send(EngineCommand::Play);
    }

    pub fn stop(&self) {
        self.send(EngineCommand::Stop);
    }

    pub fn toggle_play(&self) -> bool {
        let was = self.telemetry.is_playing.load(Ordering::Acquire);
        if was {
            self.stop();
        } else {
            self.play();
        }
        !was
    }

    pub fn is_playing(&self) -> bool {
        self.telemetry.is_playing.load(Ordering::Acquire)
    }

    pub fn set_frequency(&self, hz: f32) {
        self.send(EngineCommand::SetFrequency(hz));
    }

    pub fn set_amplitude(&self, amp: f32) {
        self.send(EngineCommand::SetAmplitude(amp));
    }

    pub fn reconfigure(&self, sample_rate: f32, channels: u16) {
        self.send(EngineCommand::Reconfigure {
            sample_rate,
            channels,
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AudioEngine — lives exclusively on the audio thread
// ─────────────────────────────────────────────────────────────────────────────

/// The real-time audio engine.  Owns the `SineGenerator` and preallocated state.
/// Must be used only from the audio callback thread after construction.
///
/// Construction happens on the UI thread; after handing `AudioEngine` to the
/// stream closure it must not be accessed from the UI thread again.
pub struct AudioEngine {
    generator: SineGenerator,
    commands: Arc<CommandRing<64>>,
    telemetry: Arc<EngineTelemetry>,
    channels: usize,
    sample_rate: f32,
    /// Preallocated scratch buffer (max expected frame count × channels).
    /// Avoids any per-callback allocation.
    _scratch: Vec<f32>,
}

impl AudioEngine {
    /// Construct the engine and return a cloneable `EngineHandle` for the UI thread.
    pub fn new(sample_rate: f32, channels: u16) -> (Self, EngineHandle) {
        let ch = (channels as usize).max(1);
        let sr = sample_rate.max(1.0);
        let commands = Arc::new(CommandRing::<64>::new());
        let telemetry = Arc::new(EngineTelemetry::new());

        let handle = EngineHandle {
            commands: Arc::clone(&commands),
            telemetry: Arc::clone(&telemetry),
        };

        // Preallocate for the largest expected buffer (2048 frames × channels).
        let scratch = vec![0.0f32; 2048 * ch];

        let engine = Self {
            generator: SineGenerator::new(440.0, 0.1, sr, ch),
            commands,
            telemetry,
            channels: ch,
            sample_rate: sr,
            _scratch: scratch,
        };

        (engine, handle)
    }

    /// Reconfigure sample rate and channel count in-place.
    /// Call via `EngineCommand::Reconfigure` from the audio thread only.
    fn apply_reconfigure(&mut self, sample_rate: f32, channels: u16) {
        let ch = (channels as usize).max(1);
        self.sample_rate = sample_rate.max(1.0);
        self.channels = ch;
        self.generator.set_sample_rate(self.sample_rate);
        self.generator.set_channels(ch);
    }

    /// **Audio callback entry point.**
    ///
    /// # Real-time safety contract
    /// * No heap allocation (preallocated scratch is used).
    /// * No blocking synchronisation.
    /// * No I/O.
    /// * Bounded iteration proportional to `output.len()`.
    ///
    /// `deadline_ns` is the callback budget in nanoseconds (buffer_frames / sample_rate × 1e9).
    /// Pass `0` to skip underrun detection.
    pub fn process(&mut self, output: &mut [f32], deadline_ns: u64) {
        let t0 = if deadline_ns > 0 {
            // `std::time::Instant::now()` performs a syscall but it is safe on
            // macOS CoreAudio threads and is the standard approach for DSP load
            // measurement.  We accept this single syscall per block.
            Some(std::time::Instant::now())
        } else {
            None
        };

        // ── drain commands (non-blocking) ──────────────────────────────────
        while let Some(cmd) = self.commands.pop() {
            match cmd {
                EngineCommand::Play => {
                    self.generator.set_playing(true);
                    self.telemetry.is_playing.store(true, Ordering::Release);
                }
                EngineCommand::Stop => {
                    self.generator.set_playing(false);
                    self.telemetry.is_playing.store(false, Ordering::Release);
                }
                EngineCommand::SetFrequency(f) => self.generator.set_frequency(f),
                EngineCommand::SetAmplitude(a) => self.generator.set_amplitude(a),
                EngineCommand::Reconfigure {
                    sample_rate,
                    channels,
                } => {
                    self.apply_reconfigure(sample_rate, channels);
                }
            }
        }

        // ── render audio ───────────────────────────────────────────────────
        self.generator.render_interleaved(output, self.channels);

        // ── compute peak for telemetry ─────────────────────────────────────
        let peak = output.iter().copied().fold(0.0f32, |a, s| a.max(s.abs()));
        self.telemetry
            .peak_bits
            .store(peak.to_bits(), Ordering::Release);

        // ── timing & underrun detection ────────────────────────────────────
        if let Some(t0) = t0 {
            let elapsed_ns = t0.elapsed().as_nanos() as u64;
            self.telemetry
                .last_callback_ns
                .store(elapsed_ns, Ordering::Release);

            // Update max callback duration (relaxed — approximate is fine).
            let prev_max = self.telemetry.max_callback_ns.load(Ordering::Relaxed);
            if elapsed_ns > prev_max {
                self.telemetry
                    .max_callback_ns
                    .store(elapsed_ns, Ordering::Relaxed);
            }

            if deadline_ns > 0 && elapsed_ns > deadline_ns {
                self.telemetry
                    .underrun_count
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Compute the callback budget in nanoseconds for the given buffer size.
    pub fn budget_ns(buffer_frames: u32, sample_rate: f32) -> u64 {
        if sample_rate <= 0.0 || buffer_frames == 0 {
            return 0;
        }
        (buffer_frames as f64 / sample_rate as f64 * 1_000_000_000.0) as u64
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy EngineControl shim
// ─────────────────────────────────────────────────────────────────────────────
// Kept for backward compatibility during migration. New code should use
// `EngineHandle` directly.

/// **Deprecated** — use `EngineHandle` for new code.
/// A lightweight atomic-only control pair used before the SPSC ring existed.
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
        self.frequency_bits
            .store(freq.clamp(20.0, 20_000.0).to_bits(), Ordering::Release);
    }

    pub fn get_frequency(&self) -> f32 {
        f32::from_bits(self.frequency_bits.load(Ordering::Acquire))
    }

    pub fn set_amplitude(&self, amp: f32) {
        self.amplitude_bits
            .store(amp.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }

    pub fn get_amplitude(&self) -> f32 {
        f32::from_bits(self.amplitude_bits.load(Ordering::Acquire))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SineGenerator ────────────────────────────────────────────────────────

    #[test]
    fn test_silence_when_stopped() {
        let mut sg = SineGenerator::new(440.0, 0.5, 48_000.0, 2);
        sg.set_playing(false);
        let mut buf = [1.0f32; 128];
        sg.render_interleaved(&mut buf, 2);
        assert!(buf.iter().all(|&s| s == 0.0), "expected silence");
    }

    #[test]
    fn test_zero_amplitude_produces_silence() {
        let mut sg = SineGenerator::new(440.0, 0.0, 48_000.0, 2);
        sg.set_playing(true);
        let mut buf = [1.0f32; 64];
        sg.render_interleaved(&mut buf, 2);
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "zero amplitude must be silent"
        );
    }

    #[test]
    fn test_playing_produces_nonzero_output() {
        let mut sg = SineGenerator::new(440.0, 0.2, 48_000.0, 2);
        sg.set_playing(true);
        let mut buf = [0.0f32; 128];
        sg.render_interleaved(&mut buf, 2);
        let max = buf.iter().copied().fold(0.0f32, f32::max);
        assert!(max > 0.0, "expected non-zero output");
        assert!(max <= 0.2001, "amplitude exceeds clamped value");
    }

    #[test]
    fn test_phase_continuity_across_buffers() {
        let mut sg = SineGenerator::new(440.0, 1.0, 48_000.0, 2);
        sg.set_playing(true);
        let mut b1 = [0.0f32; 64];
        let mut b2 = [0.0f32; 64];
        sg.render_interleaved(&mut b1, 2);
        sg.render_interleaved(&mut b2, 2);
        // Last stereo frame of b1 and first stereo frame of b2 should be close.
        let diff = (b2[0] - b1[62]).abs();
        assert!(diff < 0.2, "phase discontinuity between blocks: {diff}");
    }

    #[test]
    fn test_clamping_at_construction() {
        let sg = SineGenerator::new(-1.0, -1.0, 48_000.0, 2);
        assert_eq!(sg.frequency, 20.0);
        assert_eq!(sg.amplitude, 0.0);

        let mut gen2 = SineGenerator::new(440.0, 0.5, 48_000.0, 2);
        gen2.set_frequency(99_999.0);
        assert_eq!(gen2.frequency, 20_000.0);
        gen2.set_amplitude(5.0);
        assert_eq!(gen2.amplitude, 1.0);
    }

    #[test]
    fn test_various_sample_rates() {
        for &sr in &[22_050.0, 44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
            let mut sg = SineGenerator::new(1_000.0, 0.5, sr, 2);
            sg.set_playing(true);
            let mut buf = vec![0.0f32; 256];
            sg.render_interleaved(&mut buf, 2);
            let max = buf.iter().copied().fold(0.0f32, f32::max);
            assert!(max > 0.0 && max <= 0.5001, "sr={sr} failed: max={max}");
        }
    }

    #[test]
    fn test_mono_output() {
        let mut sg = SineGenerator::new(440.0, 0.5, 48_000.0, 1);
        sg.set_playing(true);
        let mut buf = vec![0.0f32; 128]; // 128 mono frames
        sg.render_interleaved(&mut buf, 1);
        let max = buf.iter().copied().fold(0.0f32, f32::max);
        assert!(max > 0.0 && max <= 0.5001, "mono failed: max={max}");
    }

    #[test]
    fn test_stereo_interleaving() {
        // All channels in a frame must share the same sample value.
        for &ch in &[1usize, 2, 4, 6] {
            let mut sg = SineGenerator::new(440.0, 0.4, 48_000.0, ch);
            sg.set_playing(true);
            let mut buf = vec![0.0f32; ch * 64];
            sg.render_interleaved(&mut buf, ch);
            for frame in 0..64 {
                let first = buf[frame * ch];
                for c in 1..ch {
                    assert_eq!(
                        buf[frame * ch + c],
                        first,
                        "ch={ch} frame={frame} channel mismatch"
                    );
                }
            }
        }
    }

    #[test]
    fn test_various_buffer_sizes() {
        let mut sg = SineGenerator::new(440.0, 0.5, 48_000.0, 2);
        sg.set_playing(true);
        for &frames in &[16usize, 32, 64, 128, 256, 512, 1024, 2048] {
            let mut buf = vec![0.0f32; frames * 2];
            sg.render_interleaved(&mut buf, 2);
            let max = buf.iter().copied().fold(0.0f32, f32::max);
            assert!(max > 0.0 && max <= 0.5001, "frames={frames} max={max}");
        }
    }

    // ── SPSC CommandRing ──────────────────────────────────────────────────────

    #[test]
    fn test_command_ring_empty_returns_none() {
        let ring = CommandRing::<8>::new();
        assert!(ring.pop().is_none());
    }

    #[test]
    fn test_command_ring_push_pop_roundtrip() {
        let ring = CommandRing::<8>::new();
        ring.push(EngineCommand::SetFrequency(880.0));
        ring.push(EngineCommand::SetAmplitude(0.5));
        ring.push(EngineCommand::Play);

        assert_eq!(ring.pop(), Some(EngineCommand::SetFrequency(880.0)));
        assert_eq!(ring.pop(), Some(EngineCommand::SetAmplitude(0.5)));
        assert_eq!(ring.pop(), Some(EngineCommand::Play));
        assert!(ring.pop().is_none());
    }

    #[test]
    fn test_command_ring_reconfigure_roundtrip() {
        let ring = CommandRing::<8>::new();
        ring.push(EngineCommand::Reconfigure {
            sample_rate: 44_100.0,
            channels: 1,
        });
        match ring.pop() {
            Some(EngineCommand::Reconfigure {
                sample_rate,
                channels,
            }) => {
                assert_eq!(sample_rate, 44_100.0);
                assert_eq!(channels, 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    // ── AudioEngine ──────────────────────────────────────────────────────────

    #[test]
    fn test_engine_starts_silent() {
        let (mut engine, _handle) = AudioEngine::new(48_000.0, 2);
        let mut buf = [0.0f32; 128];
        engine.process(&mut buf, 0);
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "engine must start silent (stopped)"
        );
    }

    #[test]
    fn test_engine_play_via_handle() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        let mut buf = [0.0f32; 256];
        engine.process(&mut buf, 0);
        let max = buf.iter().copied().fold(0.0f32, f32::max);
        assert!(max > 0.0, "engine should produce audio after Play command");
    }

    #[test]
    fn test_engine_stop_via_handle() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        // Process once so the Play command is consumed.
        let mut buf = [0.0f32; 128];
        engine.process(&mut buf, 0);
        handle.stop();
        engine.process(&mut buf, 0);
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "engine must be silent after Stop"
        );
    }

    #[test]
    fn test_engine_set_frequency_via_handle() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        handle.set_frequency(880.0);
        let mut buf = [0.0f32; 256];
        engine.process(&mut buf, 0);
        // Can't easily measure exact frequency in unit test, but engine must not panic
        // and must produce non-zero output.
        let max = buf.iter().copied().fold(0.0f32, f32::max);
        assert!(max > 0.0);
    }

    #[test]
    fn test_engine_reconfigure_44100_mono() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.send(EngineCommand::Reconfigure {
            sample_rate: 44_100.0,
            channels: 1,
        });
        handle.play();
        let mut buf = vec![0.0f32; 256]; // 256 mono frames
        engine.process(&mut buf, 0);
        let max = buf.iter().copied().fold(0.0f32, f32::max);
        assert!(
            max > 0.0,
            "reconfigured mono 44.1 kHz engine must produce audio"
        );
    }

    #[test]
    fn test_engine_reconfigure_96000_stereo() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.send(EngineCommand::Reconfigure {
            sample_rate: 96_000.0,
            channels: 2,
        });
        handle.play();
        let mut buf = [0.0f32; 512];
        engine.process(&mut buf, 0);
        let max = buf.iter().copied().fold(0.0f32, f32::max);
        assert!(max > 0.0, "96 kHz stereo must produce audio");
    }

    #[test]
    fn test_telemetry_peak_updated() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        handle.set_amplitude(0.4);
        let mut buf = [0.0f32; 256];
        engine.process(&mut buf, 0);
        let peak = handle.telemetry.peak();
        assert!(peak > 0.0 && peak <= 0.4001, "peak telemetry: {peak}");
    }

    #[test]
    fn test_telemetry_playing_flag() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        assert!(!handle.is_playing());
        handle.play();
        let mut buf = [0.0f32; 64];
        engine.process(&mut buf, 0);
        assert!(handle.is_playing());
        handle.stop();
        engine.process(&mut buf, 0);
        assert!(!handle.is_playing());
    }

    #[test]
    fn test_underrun_counter_increments_on_overrun() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        // Pass a deadline of 1 ns — guaranteed to overrun.
        let mut buf = [0.0f32; 256];
        engine.process(&mut buf, 1);
        assert!(
            handle.telemetry.underruns() > 0,
            "expected underrun with 1 ns deadline"
        );
    }

    #[test]
    fn test_budget_ns_calculation() {
        // 128 frames @ 48 kHz = 128/48000 s ≈ 2 666 666 ns
        let ns = AudioEngine::budget_ns(128, 48_000.0);
        assert!((ns as i64 - 2_666_666).abs() < 100, "budget_ns={ns}");
    }

    #[test]
    fn test_dsp_load_zero_on_no_timing() {
        let (_, handle) = AudioEngine::new(48_000.0, 2);
        let load = handle.telemetry.dsp_load(128, 48_000.0);
        assert_eq!(load, 0.0);
    }
}
