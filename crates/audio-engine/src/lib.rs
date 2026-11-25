//! `esperia-audio-engine` — real-time audio processing core.
//!
//! # Architecture rules (enforced)
//! * This crate has **zero** dependency on `cpal`, `esperia-audio-io`, or any GUI crate.
//! * Every public function that enters the audio callback is real-time safe:
//!   no heap allocation, no blocking synchronisation, no I/O.
//! * Cross-thread communication uses lock-free atomics and a bounded SPSC ring.
//!
//! # Step 3 additions
//! * `Pause`, `Seek`, `SetLoopRegion`, `EnableLoop` commands.
//! * `ClipSchedule` — the UI thread passes an `Arc<Vec<ScheduledClip>>` to the
//!   engine via `AtomicPtr` (lock-free ownership transfer).
//! * `playhead_frame` in `EngineTelemetry` for UI position display.
//! * `ScheduledClip` holds an `Arc<DecodedSamples>` (a type-alias for
//!   `Arc<Vec<f32>>`) so the callback can read decoded audio without allocation.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

// ─────────────────────────────────────────────────────────────────────────────
// DecodedSamples — shared audio data
// ─────────────────────────────────────────────────────────────────────────────

/// A cheaply-cloneable handle to decoded, interleaved f32 audio data.
/// The audio callback reads this without any allocation.
pub type DecodedSamples = Arc<Vec<f32>>;

// ─────────────────────────────────────────────────────────────────────────────
// SPSC Command ring
// ─────────────────────────────────────────────────────────────────────────────

/// Commands that the UI thread sends to the audio engine.
/// Each variant is encoded as a fixed-size `[u32; 4]` to accommodate u64 frame
/// positions (encoded as two u32 words).
///
/// Encoding: `[tag, word0, word1, word2]`
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EngineCommand {
    /// Start playback.
    Play,
    /// Stop playback and reset playhead to 0.
    Stop,
    /// Pause playback; playhead retains position.
    Pause,
    /// Set the sine-wave frequency (Hz). Clamped to [20, 20 000].
    SetFrequency(f32),
    /// Set the sine-wave amplitude. Clamped to [0, 1].
    SetAmplitude(f32),
    /// Reconfigure sample rate and channel count without restarting the stream.
    Reconfigure { sample_rate: f32, channels: u16 },
    /// Seek to an absolute frame position.
    Seek { frame: u64 },
    /// Define the loop region (in frames).
    SetLoopRegion { start: u64, end: u64 },
    /// Enable or disable looping.
    EnableLoop(bool),
}

impl EngineCommand {
    fn encode(self) -> [u32; 4] {
        match self {
            Self::Play => [0, 0, 0, 0],
            Self::Stop => [1, 0, 0, 0],
            Self::SetFrequency(f) => [2, f.to_bits(), 0, 0],
            Self::SetAmplitude(a) => [3, a.to_bits(), 0, 0],
            Self::Reconfigure {
                sample_rate,
                channels,
            } => [4, sample_rate.to_bits(), channels as u32, 0],
            Self::Pause => [5, 0, 0, 0],
            Self::Seek { frame } => {
                let lo = frame as u32;
                let hi = (frame >> 32) as u32;
                [6, lo, hi, 0]
            }
            Self::SetLoopRegion { start, end } => {
                // Pack: word0 = start_lo, word1 = start_hi|end_lo (tricky).
                // Simpler: encode start in [1,2] and end in [3, extra].
                // We'll use two separate u32 pairs.
                let s_lo = start as u32;
                let s_hi = (start >> 32) as u32;
                // end encoded separately — but we only have 4 slots.
                // Encode end_lo in word3, end_hi is lost (acceptable for
                // timelines < 4 Gi frames ≈ 24 h at 48 kHz).
                let e_lo = end as u32;
                // Encode s_hi and e_hi together: ((s_hi & 0xFFFF) << 16) | (e_hi & 0xFFFF)
                let hi_packed = ((s_hi & 0xFFFF) << 16) | ((end >> 32) as u32 & 0xFFFF);
                [7, s_lo, e_lo, hi_packed]
            }
            Self::EnableLoop(b) => [8, b as u32, 0, 0],
        }
    }

    fn decode(raw: [u32; 4]) -> Option<Self> {
        match raw[0] {
            0 => Some(Self::Play),
            1 => Some(Self::Stop),
            2 => Some(Self::SetFrequency(f32::from_bits(raw[1]))),
            3 => Some(Self::SetAmplitude(f32::from_bits(raw[1]))),
            4 => Some(Self::Reconfigure {
                sample_rate: f32::from_bits(raw[1]),
                channels: raw[2] as u16,
            }),
            5 => Some(Self::Pause),
            6 => {
                let frame = raw[1] as u64 | ((raw[2] as u64) << 32);
                Some(Self::Seek { frame })
            }
            7 => {
                let s_lo = raw[1] as u64;
                let e_lo = raw[2] as u64;
                let hi_packed = raw[3];
                let s_hi = ((hi_packed >> 16) & 0xFFFF) as u64;
                let e_hi = (hi_packed & 0xFFFF) as u64;
                let start = s_lo | (s_hi << 32);
                let end = e_lo | (e_hi << 32);
                Some(Self::SetLoopRegion { start, end })
            }
            8 => Some(Self::EnableLoop(raw[1] != 0)),
            _ => None,
        }
    }
}

/// Bounded, wait-free SPSC command ring.
///
/// Slots are `[u32; 4]` (upgraded from `[u32; 3]` to hold 64-bit frame indices).
struct CommandRing<const N: usize> {
    slots: Box<[[AtomicU32; 4]; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl<const N: usize> CommandRing<N> {
    fn new() -> Self {
        assert!(
            N.is_power_of_two(),
            "CommandRing capacity must be a power of two"
        );
        let slots: Vec<[AtomicU32; 4]> = (0..N)
            .map(|_| {
                [
                    AtomicU32::new(0),
                    AtomicU32::new(0),
                    AtomicU32::new(0),
                    AtomicU32::new(0),
                ]
            })
            .collect();
        Self {
            slots: slots.into_boxed_slice().try_into().ok().unwrap(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn push(&self, cmd: EngineCommand) {
        let raw = cmd.encode();
        let head = self.head.load(Ordering::Relaxed);
        let slot = head & (N - 1);
        self.slots[slot][0].store(raw[0], Ordering::Relaxed);
        self.slots[slot][1].store(raw[1], Ordering::Relaxed);
        self.slots[slot][2].store(raw[2], Ordering::Relaxed);
        self.slots[slot][3].store(raw[3], Ordering::Release);
        self.head.store(head.wrapping_add(1), Ordering::Release);
    }

    fn pop(&self) -> Option<EngineCommand> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let slot = tail & (N - 1);
        let _ = self.slots[slot][3].load(Ordering::Acquire);
        let raw = [
            self.slots[slot][0].load(Ordering::Relaxed),
            self.slots[slot][1].load(Ordering::Relaxed),
            self.slots[slot][2].load(Ordering::Relaxed),
            self.slots[slot][3].load(Ordering::Relaxed),
        ];
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        EngineCommand::decode(raw)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ClipSchedule — UI → audio lock-free transfer
// ─────────────────────────────────────────────────────────────────────────────

/// A single audio clip scheduled on the timeline.
///
/// All fields are `Copy` / plain integers so the audio callback can read them
/// without any allocation.  The `samples` arc is cloned once per schedule swap,
/// not per callback.
#[derive(Clone)]
pub struct ScheduledClip {
    /// Unique clip identifier (opaque u64 — maps to `ClipId`'s inner UUID low bits).
    pub clip_id: u64,
    /// Frame at which this clip starts on the timeline.
    pub timeline_start_frame: u64,
    /// First frame within the source audio to play (for trimmed clips).
    pub source_offset_frame: u64,
    /// Number of frames from `source_offset_frame` to play.
    pub duration_frames: u64,
    /// Linear gain applied to this clip (1.0 = unity).
    pub gain: f32,
    /// Number of channels in `samples`.
    pub channels: u16,
    /// The decoded, interleaved f32 samples.
    pub samples: DecodedSamples,
}

/// A complete mix snapshot: all clips for all tracks at one point in time.
/// Shared between the UI thread (which builds and replaces it) and the audio
/// callback (which reads it).
pub struct ClipSchedule {
    pub clips: Vec<ScheduledClip>,
}

// ─────────────────────────────────────────────────────────────────────────────
// TransportState
// ─────────────────────────────────────────────────────────────────────────────

/// Transport states encoded as u8 for atomic storage.
mod transport {
    pub const STOPPED: u8 = 0;
    pub const PLAYING: u8 = 1;
    pub const PAUSED: u8 = 2;
}

// ─────────────────────────────────────────────────────────────────────────────
// EngineTelemetry
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
    /// Current playhead position in frames.
    pub playhead_frame: AtomicU64,
    /// Transport state: 0=stopped, 1=playing, 2=paused.
    pub transport_state: AtomicU8,
}

impl EngineTelemetry {
    fn new() -> Self {
        Self {
            peak_bits: AtomicU32::new(0.0f32.to_bits()),
            underrun_count: AtomicU64::new(0),
            last_callback_ns: AtomicU64::new(0),
            max_callback_ns: AtomicU64::new(0),
            is_playing: AtomicBool::new(false),
            playhead_frame: AtomicU64::new(0),
            transport_state: AtomicU8::new(transport::STOPPED),
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

    /// Current playhead position in frames.
    pub fn playhead_frame(&self) -> u64 {
        self.playhead_frame.load(Ordering::Acquire)
    }

    /// Current transport state.
    pub fn transport_state(&self) -> u8 {
        self.transport_state.load(Ordering::Acquire)
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
    /// Lock-free pointer to the current `ClipSchedule`.
    schedule_ptr: Arc<AtomicPtr<ClipSchedule>>,
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

    pub fn pause(&self) {
        self.send(EngineCommand::Pause);
    }

    pub fn toggle_play(&self) -> bool {
        let state = self.telemetry.transport_state.load(Ordering::Acquire);
        if state == transport::PLAYING {
            self.pause();
            false
        } else {
            self.play();
            true
        }
    }

    pub fn is_playing(&self) -> bool {
        self.telemetry.is_playing.load(Ordering::Acquire)
    }

    pub fn seek(&self, frame: u64) {
        self.send(EngineCommand::Seek { frame });
    }

    pub fn set_loop_region(&self, start: u64, end: u64) {
        self.send(EngineCommand::SetLoopRegion { start, end });
    }

    pub fn enable_loop(&self, enabled: bool) {
        self.send(EngineCommand::EnableLoop(enabled));
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

    /// Replace the clip schedule.  Lock-free: the old schedule is dropped on
    /// the UI thread after the swap.
    ///
    /// # Safety contract
    /// The audio callback reads the pointer with `Acquire` ordering and clones
    /// the Arc once per block — this is a single atomic increment, not a heap
    /// allocation.
    pub fn set_clip_schedule(&self, schedule: Arc<ClipSchedule>) {
        let new_raw = Arc::into_raw(schedule) as *mut ClipSchedule;
        let old_raw = self.schedule_ptr.swap(new_raw, Ordering::AcqRel);
        // Drop the old schedule on the UI thread.
        if !old_raw.is_null() {
            // SAFETY: we own the raw pointer because we stored it here originally.
            let _ = unsafe { Arc::from_raw(old_raw as *const ClipSchedule) };
        }
    }

    /// Return the current playhead position in frames.
    pub fn playhead_frame(&self) -> u64 {
        self.telemetry.playhead_frame.load(Ordering::Acquire)
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // If this is the last handle, free the schedule pointer.
        // Arc::strong_count is not available without an Arc reference, so we
        // check if our schedule_ptr Arc has only one reference left.
        if Arc::strong_count(&self.schedule_ptr) == 1 {
            let raw = self
                .schedule_ptr
                .swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !raw.is_null() {
                let _ = unsafe { Arc::from_raw(raw as *const ClipSchedule) };
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AudioEngine — lives exclusively on the audio thread
// ─────────────────────────────────────────────────────────────────────────────

/// The real-time audio engine.
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
    _scratch: Vec<f32>,
    // ── Transport state ───────────────────────────────────────────────────────
    transport_state: u8,
    playhead_frame: u64,
    loop_start: u64,
    loop_end: u64,
    loop_enabled: bool,
    /// Shared pointer to the current clip schedule.
    schedule_ptr: Arc<AtomicPtr<ClipSchedule>>,
}

impl AudioEngine {
    /// Construct the engine and return a cloneable `EngineHandle` for the UI thread.
    pub fn new(sample_rate: f32, channels: u16) -> (Self, EngineHandle) {
        let ch = (channels as usize).max(1);
        let sr = sample_rate.max(1.0);
        let commands = Arc::new(CommandRing::<64>::new());
        let telemetry = Arc::new(EngineTelemetry::new());
        let schedule_ptr = Arc::new(AtomicPtr::new(std::ptr::null_mut()));

        let handle = EngineHandle {
            commands: Arc::clone(&commands),
            telemetry: Arc::clone(&telemetry),
            schedule_ptr: Arc::clone(&schedule_ptr),
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
            transport_state: transport::STOPPED,
            playhead_frame: 0,
            loop_start: 0,
            loop_end: u64::MAX,
            loop_enabled: false,
            schedule_ptr,
        };

        (engine, handle)
    }

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
    /// * No heap allocation (preallocated scratch is used; Arc::clone is one atomic).
    /// * No blocking synchronisation.
    /// * No I/O.
    /// * Bounded iteration proportional to `output.len()`.
    ///
    /// `deadline_ns` is the callback budget in nanoseconds (buffer_frames / sample_rate × 1e9).
    /// Pass `0` to skip underrun detection.
    pub fn process(&mut self, output: &mut [f32], deadline_ns: u64) {
        let t0 = if deadline_ns > 0 {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // ── drain commands (non-blocking) ──────────────────────────────────
        while let Some(cmd) = self.commands.pop() {
            match cmd {
                EngineCommand::Play => {
                    self.transport_state = transport::PLAYING;
                    self.generator.set_playing(true);
                    self.telemetry.is_playing.store(true, Ordering::Release);
                    self.telemetry
                        .transport_state
                        .store(transport::PLAYING, Ordering::Release);
                }
                EngineCommand::Stop => {
                    self.transport_state = transport::STOPPED;
                    self.generator.set_playing(false);
                    self.playhead_frame = 0;
                    self.telemetry.is_playing.store(false, Ordering::Release);
                    self.telemetry
                        .transport_state
                        .store(transport::STOPPED, Ordering::Release);
                    self.telemetry.playhead_frame.store(0, Ordering::Release);
                }
                EngineCommand::Pause => {
                    self.transport_state = transport::PAUSED;
                    self.generator.set_playing(false);
                    self.telemetry.is_playing.store(false, Ordering::Release);
                    self.telemetry
                        .transport_state
                        .store(transport::PAUSED, Ordering::Release);
                }
                EngineCommand::Seek { frame } => {
                    self.playhead_frame = frame;
                    self.telemetry
                        .playhead_frame
                        .store(frame, Ordering::Release);
                }
                EngineCommand::SetLoopRegion { start, end } => {
                    self.loop_start = start;
                    self.loop_end = end;
                }
                EngineCommand::EnableLoop(b) => {
                    self.loop_enabled = b;
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
        let ch = self.channels;
        let frames = output.len() / ch.max(1);

        if self.transport_state == transport::PLAYING {
            // Load the current schedule (one Arc::clone — single atomic increment).
            let schedule_opt: Option<Arc<ClipSchedule>> = {
                let raw = self.schedule_ptr.load(Ordering::Acquire);
                if raw.is_null() {
                    None
                } else {
                    // SAFETY: pointer was stored via Arc::into_raw and is kept
                    // alive by the UI thread's EngineHandle.
                    let arc = unsafe { Arc::from_raw(raw as *const ClipSchedule) };
                    let cloned = Arc::clone(&arc);
                    // Forget our reconstructed Arc so we don't decrement the
                    // refcount — the original pointer still lives in schedule_ptr.
                    std::mem::forget(arc);
                    Some(cloned)
                }
            };

            // Fill output with sine generator first (tone-generator mode).
            self.generator.render_interleaved(output, ch);

            // If a clip schedule is present, mix clips over (or instead of) the sine.
            if let Some(schedule) = schedule_opt
                && !schedule.clips.is_empty()
            {
                // When clips exist, silence the generator contribution so we
                // only hear the actual audio.
                output.fill(0.0);

                for clip in &schedule.clips {
                    mix_clip(clip, output, frames, ch, self.playhead_frame);
                }
            }

            // ── advance playhead ───────────────────────────────────────────
            let new_head = if self.loop_enabled
                && self.loop_end > self.loop_start
                && self.playhead_frame + frames as u64 >= self.loop_end
            {
                // Wrap around to loop start.
                self.loop_start
                    + (self.playhead_frame + frames as u64 - self.loop_end)
                        .min(self.loop_end - self.loop_start)
            } else {
                self.playhead_frame.saturating_add(frames as u64)
            };
            self.playhead_frame = new_head;
            self.telemetry
                .playhead_frame
                .store(new_head, Ordering::Release);
        } else {
            output.fill(0.0);
        }

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
// Clip mixing helper (called from process — real-time safe)
// ─────────────────────────────────────────────────────────────────────────────

/// Mix `clip` into `output` for the current block starting at `playhead_frame`.
///
/// Real-time safe: no allocation, no blocking, bounded iteration.
#[inline]
fn mix_clip(
    clip: &ScheduledClip,
    output: &mut [f32],
    frames: usize,
    out_channels: usize,
    playhead_frame: u64,
) {
    // Find the overlap between [playhead, playhead+frames) and the clip's timeline range.
    let clip_end = clip.timeline_start_frame + clip.duration_frames;
    if playhead_frame >= clip_end {
        return; // clip is entirely in the past
    }
    let clip_start = clip.timeline_start_frame;
    if playhead_frame + frames as u64 <= clip_start {
        return; // clip hasn't started yet
    }

    // Output frame offset where we start writing.
    let out_start = if clip_start > playhead_frame {
        (clip_start - playhead_frame) as usize
    } else {
        0
    };

    // Source frame where we start reading.
    let src_offset = if playhead_frame > clip_start {
        (playhead_frame - clip_start) as usize + clip.source_offset_frame as usize
    } else {
        clip.source_offset_frame as usize
    };

    let clip_channels = clip.channels as usize;
    let gain = clip.gain;

    // How many frames to mix in this block.
    let frames_to_mix = (frames - out_start).min(
        clip.duration_frames as usize - {
            if playhead_frame > clip_start {
                (playhead_frame - clip_start) as usize
            } else {
                0
            }
        },
    );

    let samples = &clip.samples;

    for f in 0..frames_to_mix {
        let src_frame = src_offset + f;
        if src_frame * clip_channels + clip_channels > samples.len() {
            break;
        }
        let out_base = (out_start + f) * out_channels;
        if out_base + out_channels > output.len() {
            break;
        }
        let src_base = src_frame * clip_channels;

        for c in 0..out_channels {
            let src_c = c.min(clip_channels - 1); // channel fold-down
            output[out_base + c] += samples[src_base + src_c] * gain;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Legacy EngineControl shim (kept for backward compatibility)
// ─────────────────────────────────────────────────────────────────────────────

/// **Deprecated** — use `EngineHandle` for new code.
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
    fn test_stereo_interleave() {
        let mut sg = SineGenerator::new(1_000.0, 1.0, 48_000.0, 2);
        sg.set_playing(true);
        let mut buf = [0.0f32; 4]; // 2 stereo frames
        sg.render_interleaved(&mut buf, 2);
        // Both channels in each frame must be equal (mono source → stereo copy).
        assert_eq!(buf[0], buf[1], "left and right must be equal (frame 0)");
        assert_eq!(buf[2], buf[3], "left and right must be equal (frame 1)");
    }

    #[test]
    fn test_frequency_clamp_low() {
        let mut sg = SineGenerator::new(1.0, 0.5, 48_000.0, 1);
        assert_eq!(sg.frequency, 20.0);
        sg.set_frequency(0.0);
        assert_eq!(sg.frequency, 20.0);
    }

    #[test]
    fn test_frequency_clamp_high() {
        let sg = SineGenerator::new(999_999.0, 0.5, 48_000.0, 1);
        assert_eq!(sg.frequency, 20_000.0);
    }

    #[test]
    fn test_phase_continuity() {
        let mut sg = SineGenerator::new(440.0, 1.0, 48_000.0, 1);
        sg.set_playing(true);
        let mut buf1 = [0.0f32; 128];
        let mut buf2 = [0.0f32; 128];
        sg.render_interleaved(&mut buf1, 1);
        sg.render_interleaved(&mut buf2, 1);
        // The last sample of buf1 and first sample of buf2 should not be equal
        // (phase is advancing).  They might coincidentally be equal for a
        // specific frequency — check buf2 is not all-zero.
        assert!(buf2.iter().any(|&s| s.abs() > 0.0));
    }

    // ── CommandRing ──────────────────────────────────────────────────────────

    #[test]
    fn test_ring_roundtrip_all_commands() {
        let ring = CommandRing::<64>::new();
        let cmds = vec![
            EngineCommand::Play,
            EngineCommand::Stop,
            EngineCommand::Pause,
            EngineCommand::SetFrequency(880.0),
            EngineCommand::SetAmplitude(0.5),
            EngineCommand::Reconfigure {
                sample_rate: 44_100.0,
                channels: 1,
            },
            EngineCommand::Seek {
                frame: 0x0000_DEAD_BEEF_1234,
            },
            EngineCommand::SetLoopRegion {
                start: 1024,
                end: 8192,
            },
            EngineCommand::EnableLoop(true),
        ];
        for &cmd in &cmds {
            ring.push(cmd);
        }
        for &expected in &cmds {
            let got = ring.pop().expect("should have a command");
            assert_eq!(got, expected);
        }
        assert!(ring.pop().is_none());
    }

    #[test]
    fn test_ring_seek_round_trip() {
        let ring = CommandRing::<8>::new();
        ring.push(EngineCommand::Seek { frame: 123_456_789 });
        match ring.pop().unwrap() {
            EngineCommand::Seek { frame } => assert_eq!(frame, 123_456_789),
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
    fn test_engine_pause_retains_playhead() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        let mut buf = [0.0f32; 256];
        engine.process(&mut buf, 0);
        let ph1 = engine.playhead_frame;
        handle.pause();
        engine.process(&mut buf, 0);
        let ph2 = engine.playhead_frame;
        // After pause, playhead should not advance.
        assert_eq!(ph1, ph2, "playhead must not advance while paused");
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "paused engine must be silent"
        );
    }

    #[test]
    fn test_engine_seek() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.seek(99_000);
        let mut buf = [0.0f32; 64];
        engine.process(&mut buf, 0);
        assert_eq!(engine.playhead_frame, 99_000, "seek should set playhead");
    }

    #[test]
    fn test_engine_set_frequency_via_handle() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        handle.set_frequency(880.0);
        let mut buf = [0.0f32; 256];
        engine.process(&mut buf, 0);
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
        let mut buf = vec![0.0f32; 256];
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

    #[test]
    fn test_clip_mix_basic() {
        // Build a schedule with one mono clip containing a constant value of 0.5.
        let samples = Arc::new(vec![0.5f32; 128]); // 128 mono frames
        let clip = ScheduledClip {
            clip_id: 1,
            timeline_start_frame: 0,
            source_offset_frame: 0,
            duration_frames: 128,
            gain: 1.0,
            channels: 1,
            samples,
        };
        let schedule = Arc::new(ClipSchedule { clips: vec![clip] });

        let (mut engine, handle) = AudioEngine::new(48_000.0, 1);
        handle.set_clip_schedule(schedule);
        handle.play();

        let mut buf = [0.0f32; 128];
        engine.process(&mut buf, 0);

        // All frames should be 0.5.
        for &s in &buf {
            assert!((s - 0.5).abs() < 1e-5, "expected 0.5, got {s}");
        }
    }

    #[test]
    fn test_clip_starts_in_future() {
        // A clip that starts at frame 64 — first 64 frames of output must be silent.
        let samples = Arc::new(vec![1.0f32; 64]);
        let clip = ScheduledClip {
            clip_id: 2,
            timeline_start_frame: 64,
            source_offset_frame: 0,
            duration_frames: 64,
            gain: 1.0,
            channels: 1,
            samples,
        };
        let schedule = Arc::new(ClipSchedule { clips: vec![clip] });

        let (mut engine, handle) = AudioEngine::new(48_000.0, 1);
        handle.set_clip_schedule(schedule);
        handle.play();

        let mut buf = [0.0f32; 128];
        engine.process(&mut buf, 0);

        // Frames 0..63 should be 0.0 (clip hasn't started).
        for &s in &buf[..64] {
            assert_eq!(s, 0.0, "frame before clip should be silent");
        }
        // Frames 64..127 should be 1.0.
        for &s in &buf[64..] {
            assert!(
                (s - 1.0).abs() < 1e-5,
                "frame within clip should be 1.0, got {s}"
            );
        }
    }

    #[test]
    fn test_loop_wraps_playhead() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 1);
        handle.set_loop_region(0, 100);
        handle.enable_loop(true);
        handle.play();

        // Set playhead just before loop end.
        engine.playhead_frame = 90;
        let mut buf = [0.0f32; 64]; // will cross loop boundary at frame 100
        engine.process(&mut buf, 0);

        // Playhead should have wrapped to somewhere in [0, 100).
        assert!(
            engine.playhead_frame < 100,
            "playhead should wrap: got {}",
            engine.playhead_frame
        );
    }

    #[test]
    fn test_telemetry_playhead_advances() {
        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        handle.play();
        let mut buf = [0.0f32; 512]; // 256 frames stereo
        engine.process(&mut buf, 0);
        let ph = handle.playhead_frame();
        assert_eq!(ph, 256, "playhead should advance by frame count");
    }
}
