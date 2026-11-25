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

pub mod synth;

use esperia_midi::{ActiveNoteTracker, InstrumentKind, MidiInputRing, MidiMessage, MidiSchedule};
use esperia_mixer::{MixerGraph, MixerMeters};
use std::cell::UnsafeCell;
use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};
use synth::PolySynth;

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
    /// Enable or disable live input monitoring pass-through.
    EnableInputMonitoring(bool),
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
            Self::EnableInputMonitoring(b) => [9, b as u32, 0, 0],
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
            9 => Some(Self::EnableInputMonitoring(raw[1] != 0)),
            _ => None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MonitorRing — lock-free SPSC ring for live input monitoring
// ─────────────────────────────────────────────────────────────────────────────

/// Preallocated, lock-free SPSC ring buffer for routing live input samples
/// from the input callback thread to the output `AudioEngine::process` callback.
pub struct MonitorRing {
    buffer: Box<[UnsafeCell<f32>]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// SAFETY: SPSC contract (input callback pushes, output callback pops).
unsafe impl Send for MonitorRing {}
unsafe impl Sync for MonitorRing {}

impl MonitorRing {
    pub fn new(min_capacity_samples: usize) -> Self {
        let cap = min_capacity_samples.max(256).next_power_of_two();
        let buffer: Vec<UnsafeCell<f32>> = (0..cap).map(|_| UnsafeCell::new(0.0)).collect();
        Self {
            buffer: buffer.into_boxed_slice(),
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Push captured samples from the input callback (real-time safe, drops oldest
    /// excess if full rather than blocking).
    #[inline]
    pub fn push_samples(&self, samples: &[f32]) {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        let free = (self.mask + 1).saturating_sub(used);
        let to_write = samples.len().min(free);
        for (i, &s) in samples.iter().take(to_write).enumerate() {
            let idx = (head.wrapping_add(i)) & self.mask;
            unsafe {
                *self.buffer[idx].get() = s;
            }
        }
        self.head
            .store(head.wrapping_add(to_write), Ordering::Release);
    }

    /// Pop up to `dst.len()` samples into `dst` (output callback, real-time safe).
    #[inline]
    pub fn pop_samples(&self, dst: &mut [f32]) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let avail = head.wrapping_sub(tail);
        let to_read = dst.len().min(avail);
        for (i, slot) in dst.iter_mut().take(to_read).enumerate() {
            let idx = (tail.wrapping_add(i)) & self.mask;
            *slot = unsafe { *self.buffer[idx].get() };
        }
        self.tail
            .store(tail.wrapping_add(to_read), Ordering::Release);
        to_read
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
    /// Track / mixer channel ID that this clip routes into.
    pub track_id: u64,
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
    /// Lock-free pointer to the current `MidiSchedule`.
    midi_schedule_ptr: Arc<AtomicPtr<MidiSchedule>>,
    /// Lock-free pointer for pushing a newly-built `MixerGraph` to the audio thread.
    pending_mixer_ptr: Arc<AtomicPtr<MixerGraph>>,
    /// Lock-free pointer for returning a retired `MixerGraph` to the UI thread for drop.
    retired_mixer_ptr: Arc<AtomicPtr<MixerGraph>>,
    /// Lock-free ring for live input monitoring (written by input callback).
    monitor_ring: Arc<MonitorRing>,
    /// Lock-free ring for live MIDI keyboard / Piano Roll preview input.
    live_midi_ring: Arc<MidiInputRing>,
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

    pub fn set_input_monitoring(&self, enabled: bool) {
        self.send(EngineCommand::EnableInputMonitoring(enabled));
    }

    pub fn monitor_ring(&self) -> Arc<MonitorRing> {
        Arc::clone(&self.monitor_ring)
    }

    pub fn live_midi_ring(&self) -> Arc<MidiInputRing> {
        Arc::clone(&self.live_midi_ring)
    }

    /// Push a live MIDI message (e.g. from a MIDI keyboard or Piano Roll preview)
    /// into the audio engine without blocking.
    pub fn send_live_midi(&self, msg: MidiMessage) {
        let _ = self.live_midi_ring.push(msg);
    }

    /// Replace the MIDI schedule lock-free.
    pub fn set_midi_schedule(&self, schedule: Arc<MidiSchedule>) {
        let new_raw = Arc::into_raw(schedule) as *mut MidiSchedule;
        let old_raw = self.midi_schedule_ptr.swap(new_raw, Ordering::AcqRel);
        if !old_raw.is_null() {
            // SAFETY: we own the raw pointer because we stored it here originally.
            let _ = unsafe { Arc::from_raw(old_raw as *const MidiSchedule) };
        }
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

    /// Replace the mixer routing graph lock-free.
    ///
    /// Returns the `MixerMeters` snapshot for reading channel, bus, and master
    /// peak/RMS/clip values from the UI thread. Any previously retired graph
    /// is reclaimed and dropped on the UI thread (never on the audio thread).
    pub fn set_mixer_graph(&self, graph: MixerGraph) -> MixerMeters {
        let meters = graph.meters();
        // Reclaim any graph retired by the audio thread first.
        let retired = self
            .retired_mixer_ptr
            .swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !retired.is_null() {
            let _ = unsafe { Box::from_raw(retired) };
        }

        let new_raw = Box::into_raw(Box::new(graph));
        let prev_pending = self.pending_mixer_ptr.swap(new_raw, Ordering::AcqRel);
        if !prev_pending.is_null() {
            // Audio thread hadn't picked up the previous pending graph yet; drop it here.
            let _ = unsafe { Box::from_raw(prev_pending) };
        }
        meters
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
        if Arc::strong_count(&self.midi_schedule_ptr) == 1 {
            let raw = self
                .midi_schedule_ptr
                .swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !raw.is_null() {
                let _ = unsafe { Arc::from_raw(raw as *const MidiSchedule) };
            }
        }
        if Arc::strong_count(&self.pending_mixer_ptr) == 1 {
            let raw = self
                .pending_mixer_ptr
                .swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !raw.is_null() {
                let _ = unsafe { Box::from_raw(raw) };
            }
        }
        if Arc::strong_count(&self.retired_mixer_ptr) == 1 {
            let raw = self
                .retired_mixer_ptr
                .swap(std::ptr::null_mut(), Ordering::AcqRel);
            if !raw.is_null() {
                let _ = unsafe { Box::from_raw(raw) };
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
    input_monitoring_enabled: bool,
    /// Shared pointer to the current clip schedule.
    schedule_ptr: Arc<AtomicPtr<ClipSchedule>>,
    /// Shared pointer to the current MIDI schedule.
    midi_schedule_ptr: Arc<AtomicPtr<MidiSchedule>>,
    /// Active mixer routing graph (owned exclusively by audio thread).
    mixer: Option<MixerGraph>,
    pending_mixer_ptr: Arc<AtomicPtr<MixerGraph>>,
    retired_mixer_ptr: Arc<AtomicPtr<MixerGraph>>,
    /// Preallocated SPSC ring for live input monitoring.
    monitor_ring: Arc<MonitorRing>,
    /// Preallocated scratch buffer for draining monitor samples.
    monitor_scratch: Vec<f32>,
    /// Lock-free ring for live MIDI preview / keyboard input.
    live_midi_ring: Arc<MidiInputRing>,
    /// Built-in polyphonic synthesizer for MIDI playback & preview.
    synth: PolySynth,
    /// Zero-allocation tracker preventing stuck notes on stop/pause/seek/loop.
    active_note_tracker: ActiveNoteTracker,
    /// Preallocated stereo scratch buffer for MIDI synth rendering.
    midi_scratch: Vec<f32>,
}

impl AudioEngine {
    /// Construct the engine and return a cloneable `EngineHandle` for the UI thread.
    pub fn new(sample_rate: f32, channels: u16) -> (Self, EngineHandle) {
        let ch = (channels as usize).max(1);
        let sr = sample_rate.max(1.0);
        let commands = Arc::new(CommandRing::<64>::new());
        let telemetry = Arc::new(EngineTelemetry::new());
        let schedule_ptr = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let midi_schedule_ptr = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let pending_mixer_ptr = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let retired_mixer_ptr = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let monitor_ring = Arc::new(MonitorRing::new(16_384));
        let live_midi_ring = Arc::new(MidiInputRing::new());

        let handle = EngineHandle {
            commands: Arc::clone(&commands),
            telemetry: Arc::clone(&telemetry),
            schedule_ptr: Arc::clone(&schedule_ptr),
            midi_schedule_ptr: Arc::clone(&midi_schedule_ptr),
            pending_mixer_ptr: Arc::clone(&pending_mixer_ptr),
            retired_mixer_ptr: Arc::clone(&retired_mixer_ptr),
            monitor_ring: Arc::clone(&monitor_ring),
            live_midi_ring: Arc::clone(&live_midi_ring),
        };

        // Preallocate for the largest expected buffer (2048 frames × channels).
        let scratch = vec![0.0f32; 2048 * ch];
        let monitor_scratch = vec![0.0f32; 4096 * ch];
        let midi_scratch = vec![0.0f32; 4096 * 2];

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
            input_monitoring_enabled: false,
            schedule_ptr,
            midi_schedule_ptr,
            mixer: None,
            pending_mixer_ptr,
            retired_mixer_ptr,
            monitor_ring,
            monitor_scratch,
            live_midi_ring,
            synth: PolySynth::new(0, InstrumentKind::ElectricPiano, sr),
            active_note_tracker: ActiveNoteTracker::new(),
            midi_scratch,
        };

        (engine, handle)
    }

    fn flush_midi_notes(&mut self) {
        let synth = &mut self.synth;
        self.active_note_tracker.flush_all_notes_off(|msg| {
            synth.handle_message(msg);
        });
        synth.all_notes_off();
    }

    fn apply_reconfigure(&mut self, sample_rate: f32, channels: u16) {
        let ch = (channels as usize).max(1);
        self.sample_rate = sample_rate.max(1.0);
        self.channels = ch;
        self.generator.set_sample_rate(self.sample_rate);
        self.generator.set_channels(ch);
        self.synth.set_sample_rate(self.sample_rate);
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
                    self.flush_midi_notes();
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
                    self.flush_midi_notes();
                    self.telemetry.is_playing.store(false, Ordering::Release);
                    self.telemetry
                        .transport_state
                        .store(transport::PAUSED, Ordering::Release);
                }
                EngineCommand::Seek { frame } => {
                    self.flush_midi_notes();
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
                EngineCommand::EnableInputMonitoring(b) => {
                    self.input_monitoring_enabled = b;
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

        // ── swap pending mixer graph if UI published a new one ─────────────
        let incoming_mixer = self
            .pending_mixer_ptr
            .swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !incoming_mixer.is_null() {
            let new_mixer = unsafe { *Box::from_raw(incoming_mixer) };
            if let Some(old_mixer) = self.mixer.replace(new_mixer) {
                let old_raw = Box::into_raw(Box::new(old_mixer));
                let _ = self.retired_mixer_ptr.swap(old_raw, Ordering::AcqRel);
            }
        }

        // ── drain live MIDI input ring (zero alloc, non-blocking) ─────────
        while let Some(ev) = self.live_midi_ring.pop() {
            self.active_note_tracker.track_message(ev.message);
            self.synth.handle_message(ev.message);
        }

        // ── render audio ───────────────────────────────────────────────────
        let ch = self.channels;
        let frames = output.len() / ch.max(1);

        if self.transport_state == transport::PLAYING {
            // Load the current clip schedule (one Arc::clone — single atomic increment).
            let schedule_opt: Option<Arc<ClipSchedule>> = {
                let raw = self.schedule_ptr.load(Ordering::Acquire);
                if raw.is_null() {
                    None
                } else {
                    let arc = unsafe { Arc::from_raw(raw as *const ClipSchedule) };
                    let cloned = Arc::clone(&arc);
                    std::mem::forget(arc);
                    Some(cloned)
                }
            };

            // Load the current MIDI schedule (one Arc::clone — single atomic increment).
            let midi_schedule_opt: Option<Arc<MidiSchedule>> = {
                let raw = self.midi_schedule_ptr.load(Ordering::Acquire);
                if raw.is_null() {
                    None
                } else {
                    let arc = unsafe { Arc::from_raw(raw as *const MidiSchedule) };
                    let cloned = Arc::clone(&arc);
                    std::mem::forget(arc);
                    Some(cloned)
                }
            };

            let block_start = self.playhead_frame;
            let block_end = block_start.saturating_add(frames as u64);

            let has_audio_clips = schedule_opt.as_ref().is_some_and(|s| !s.clips.is_empty());
            let has_midi_events = midi_schedule_opt
                .as_ref()
                .is_some_and(|m| !m.events.is_empty());

            // Trigger scheduled MIDI events in [block_start, block_end).
            if let Some(ref midi_sched) = midi_schedule_opt {
                for ev in midi_sched.events_in_range(block_start, block_end) {
                    self.active_note_tracker.track_message(ev.message);
                    self.synth.handle_message(ev.message);
                }
            }

            if has_audio_clips || has_midi_events || self.synth.has_active_voices() {
                output.fill(0.0);

                // Render synth into preallocated stereo scratch buffer.
                let synth_frames = frames.min(self.midi_scratch.len() / 2);
                let synth_len = synth_frames * 2;
                if self.synth.has_active_voices() {
                    self.midi_scratch[..synth_len].fill(0.0);
                    self.synth
                        .render_stereo_add(&mut self.midi_scratch[..synth_len], synth_frames);
                }

                if let Some(ref mut mixer) = self.mixer {
                    mixer.clear_buffers(frames);
                    if let Some(ref schedule) = schedule_opt {
                        for clip in &schedule.clips {
                            if let Some(ch_buf) = mixer.channel_input_mut(clip.track_id, frames) {
                                mix_clip_stereo(clip, ch_buf, frames, self.playhead_frame);
                            } else {
                                mix_clip(clip, output, frames, ch, self.playhead_frame);
                            }
                        }
                    }
                    if self.synth.has_active_voices() {
                        mixer.accumulate_direct_master(
                            &self.midi_scratch[..synth_len],
                            synth_frames,
                        );
                    }
                    mixer.process(frames, ch, output);
                } else {
                    if let Some(ref schedule) = schedule_opt {
                        for clip in &schedule.clips {
                            mix_clip(clip, output, frames, ch, self.playhead_frame);
                        }
                    }
                    if self.synth.has_active_voices() {
                        mix_stereo_into_output(&self.midi_scratch[..synth_len], output, frames, ch);
                    }
                }
            } else {
                // Tone-generator fallback mode when project has no audio or MIDI content.
                self.generator.render_interleaved(output, ch);
            }

            // ── advance playhead ───────────────────────────────────────────
            let new_head = if self.loop_enabled
                && self.loop_end > self.loop_start
                && self.playhead_frame + frames as u64 >= self.loop_end
            {
                self.flush_midi_notes();
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
            if self.synth.has_active_voices() {
                let synth_frames = frames.min(self.midi_scratch.len() / 2);
                let synth_len = synth_frames * 2;
                self.midi_scratch[..synth_len].fill(0.0);
                self.synth
                    .render_stereo_add(&mut self.midi_scratch[..synth_len], synth_frames);
                mix_stereo_into_output(&self.midi_scratch[..synth_len], output, frames, ch);
            }
        }

        // ── mix live input monitoring (real-time safe, zero alloc) ─────────
        if self.input_monitoring_enabled {
            let max_drain = output.len().min(self.monitor_scratch.len());
            let read = self
                .monitor_ring
                .pop_samples(&mut self.monitor_scratch[..max_drain]);
            for i in 0..read {
                output[i] += self.monitor_scratch[i];
            }
        } else {
            // Drain stale monitor samples silently so buffer stays fresh.
            let max_drain = output.len().min(self.monitor_scratch.len());
            let _ = self
                .monitor_ring
                .pop_samples(&mut self.monitor_scratch[..max_drain]);
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

/// Mix `clip` into an interleaved stereo channel buffer (`out_channels == 2`).
#[inline]
fn mix_clip_stereo(
    clip: &ScheduledClip,
    stereo_output: &mut [f32],
    frames: usize,
    playhead_frame: u64,
) {
    mix_clip(clip, stereo_output, frames, 2, playhead_frame);
}

/// Add a stereo scratch buffer into an interleaved `out_channels` output buffer.
#[inline]
fn mix_stereo_into_output(stereo: &[f32], output: &mut [f32], frames: usize, out_channels: usize) {
    let f_max = frames
        .min(stereo.len() / 2)
        .min(output.len() / out_channels.max(1));
    if out_channels == 2 {
        for i in 0..(f_max * 2) {
            output[i] += stereo[i];
        }
    } else if out_channels == 1 {
        for f in 0..f_max {
            output[f] += 0.5 * (stereo[f * 2] + stereo[f * 2 + 1]);
        }
    } else if out_channels > 2 {
        for f in 0..f_max {
            let base = f * out_channels;
            output[base] += stereo[f * 2];
            output[base + 1] += stereo[f * 2 + 1];
        }
    }
}

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
            track_id: 1,
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
            track_id: 1,
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

    #[test]
    fn test_midi_schedule_plays_synth_and_stop_prevents_stuck_notes() {
        use esperia_midi::{MidiClip, MidiTrack, PPQN, TempoMap};

        let (mut engine, handle) = AudioEngine::new(48_000.0, 2);
        let mut track = MidiTrack::new(1, "Keys");
        let mut clip = MidiClip::new(10, "Chord", 0, PPQN * 4);
        clip.add_note(0, PPQN * 2, 60, 100, 0);
        track.clips.push(clip);

        let sched = MidiSchedule::from_tracks(&[track], &TempoMap::constant(120.0), 48_000);
        handle.set_midi_schedule(Arc::new(sched));
        handle.play();

        let mut buf = [0.0f32; 1024]; // 512 stereo frames
        engine.process(&mut buf, 0);
        let peak: f32 = buf.iter().copied().fold(0.0, |a, s| a.max(s.abs()));
        assert!(
            peak > 0.01,
            "MIDI synth should produce audible output: peak={peak}"
        );

        // Stop transport while note is still active -> must release all notes cleanly
        handle.stop();
        for _ in 0..40 {
            engine.process(&mut buf, 0);
        }
        assert!(
            !engine.active_note_tracker.has_active_notes(),
            "active note tracker should have no stuck notes after stop"
        );
        let tail_peak: f32 = buf.iter().copied().fold(0.0, |a, s| a.max(s.abs()));
        assert!(
            tail_peak < 1e-4,
            "synth voices should decay to silence after stop: tail_peak={tail_peak}"
        );
    }
}
