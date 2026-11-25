//! Real-time-safe MIDI input ring buffer and musical-clock MIDI recorder.
//!
//! Hardware MIDI callbacks push incoming messages into [`MidiInputRing`] without
//! locks or allocations.  [`MidiRecorder`] drains those events, maps them onto
//! the project's [`TempoMap`], pairs `NoteOn` with `NoteOff`, and guarantees that
//! any held notes are cleanly closed when recording stops (no stuck notes).

use crate::midi_clip::MidiClip;
use crate::midi_event::MidiMessage;
use crate::musical_time::TempoMap;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

const RING_CAPACITY: usize = 2048;

/// An incoming MIDI event pushed by a MIDI input port or virtual keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncomingMidiEvent {
    pub message: MidiMessage,
}

/// Lock-free single-producer / single-consumer ring buffer for MIDI input events.
pub struct MidiInputRing {
    buf: Box<[UnsafeCell<IncomingMidiEvent>; RING_CAPACITY]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// SAFETY: Single-producer (MIDI callback or UI) writes to `tail`, single-consumer
// (UI/recorder or audio thread) reads from `head`.
unsafe impl Sync for MidiInputRing {}
unsafe impl Send for MidiInputRing {}

impl Default for MidiInputRing {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiInputRing {
    pub fn new() -> Self {
        let default_ev = IncomingMidiEvent {
            message: MidiMessage::NoteOff {
                channel: 0,
                note: 0,
                velocity: 0,
            },
        };
        let vec: Vec<UnsafeCell<IncomingMidiEvent>> = (0..RING_CAPACITY)
            .map(|_| UnsafeCell::new(default_ev))
            .collect();
        let boxed: Box<[UnsafeCell<IncomingMidiEvent>; RING_CAPACITY]> = vec
            .into_boxed_slice()
            .try_into()
            .unwrap_or_else(|_| unreachable!("exact length"));
        Self {
            buf: boxed,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Push a MIDI message without blocking or allocating. Returns `false` if full.
    #[inline]
    pub fn push(&self, message: MidiMessage) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let next = (tail + 1) % RING_CAPACITY;
        if next == self.head.load(Ordering::Acquire) {
            return false;
        }
        // SAFETY: `tail` slot is exclusively owned by producer until `tail` advances.
        unsafe {
            *self.buf[tail].get() = IncomingMidiEvent { message };
        }
        self.tail.store(next, Ordering::Release);
        true
    }

    /// Pop the oldest MIDI message if available.
    #[inline]
    pub fn pop(&self) -> Option<IncomingMidiEvent> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: `head` slot is exclusively owned by consumer until `head` advances.
        let ev = unsafe { *self.buf[head].get() };
        self.head
            .store((head + 1) % RING_CAPACITY, Ordering::Release);
        Some(ev)
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingNote {
    start_tick: u64,
    pitch: u8,
    velocity: u8,
    channel: u8,
}

/// Stateful MIDI recorder that captures live MIDI into a [`MidiClip`].
pub struct MidiRecorder {
    armed_track_id: Option<u64>,
    is_recording: bool,
    start_tick: u64,
    active_clip: Option<MidiClip>,
    pending_notes: Vec<PendingNote>,
    next_clip_id: u64,
}

impl Default for MidiRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiRecorder {
    pub fn new() -> Self {
        Self {
            armed_track_id: None,
            is_recording: false,
            start_tick: 0,
            active_clip: None,
            pending_notes: Vec::new(),
            next_clip_id: 1000,
        }
    }

    pub fn arm_track(&mut self, track_id: Option<u64>) {
        self.armed_track_id = track_id;
    }

    pub fn armed_track_id(&self) -> Option<u64> {
        self.armed_track_id
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    /// Begin recording MIDI onto the armed track at `start_sample_frame`.
    pub fn start_recording(
        &mut self,
        start_sample_frame: u64,
        tempo_map: &TempoMap,
        sample_rate: u32,
    ) -> bool {
        if self.armed_track_id.is_none() || self.is_recording {
            return false;
        }
        let start_tick = tempo_map.sample_to_tick(start_sample_frame, sample_rate);
        let clip_id = self.next_clip_id;
        self.next_clip_id = self.next_clip_id.saturating_add(1);
        self.start_tick = start_tick;
        self.active_clip = Some(MidiClip::new(clip_id, "Recorded MIDI", start_tick, 1));
        self.pending_notes.clear();
        self.is_recording = true;
        true
    }

    /// Record a single incoming [`MidiMessage`] at `sample_frame`.
    pub fn record_message(
        &mut self,
        message: MidiMessage,
        sample_frame: u64,
        tempo_map: &TempoMap,
        sample_rate: u32,
    ) {
        if !self.is_recording {
            return;
        }
        let abs_tick = tempo_map.sample_to_tick(sample_frame, sample_rate);
        let rel_tick = abs_tick.saturating_sub(self.start_tick);

        match message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => {
                self.pending_notes.push(PendingNote {
                    start_tick: rel_tick,
                    pitch: note,
                    velocity,
                    channel,
                });
            }
            MidiMessage::NoteOn { channel, note, .. }
            | MidiMessage::NoteOff { channel, note, .. } => {
                if let Some(pos) = self
                    .pending_notes
                    .iter()
                    .position(|n| n.pitch == note && n.channel == channel)
                {
                    let pending = self.pending_notes.remove(pos);
                    let dur = rel_tick.saturating_sub(pending.start_tick).max(1);
                    if let Some(clip) = self.active_clip.as_mut() {
                        clip.add_note(
                            pending.start_tick,
                            dur,
                            pending.pitch,
                            pending.velocity,
                            pending.channel,
                        );
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => {
                if let Some(clip) = self.active_clip.as_mut() {
                    clip.add_cc(rel_tick, controller, value, channel);
                }
            }
            MidiMessage::PitchBend { channel, value } => {
                if let Some(clip) = self.active_clip.as_mut() {
                    clip.add_pitch_bend(rel_tick, value, channel);
                }
            }
            MidiMessage::ProgramChange { channel, program } => {
                if let Some(clip) = self.active_clip.as_mut() {
                    clip.add_program_change(rel_tick, program, channel);
                }
            }
        }
    }

    /// Stop recording, cleanly closing any hanging notes at `stop_sample_frame`
    /// so no stuck or zero-length notes remain, and return `(track_id, MidiClip)`.
    pub fn stop_recording(
        &mut self,
        stop_sample_frame: u64,
        tempo_map: &TempoMap,
        sample_rate: u32,
    ) -> Option<(u64, MidiClip)> {
        if !self.is_recording {
            return None;
        }
        self.is_recording = false;
        let track_id = self.armed_track_id?;
        let stop_abs_tick = tempo_map.sample_to_tick(stop_sample_frame, sample_rate);
        let stop_rel_tick = stop_abs_tick.saturating_sub(self.start_tick).max(1);

        let mut clip = self.active_clip.take()?;
        // Close any still-held notes at stop_rel_tick (prevents stuck notes!)
        for pending in self.pending_notes.drain(..) {
            let dur = stop_rel_tick.saturating_sub(pending.start_tick).max(1);
            clip.add_note(
                pending.start_tick,
                dur,
                pending.pitch,
                pending.velocity,
                pending.channel,
            );
        }

        // Size clip to cover all recorded notes (minimum 1 beat = 960 ticks).
        let max_note_end = clip.notes.iter().map(|n| n.end_tick()).max().unwrap_or(0);
        clip.length_ticks = stop_rel_tick
            .max(max_note_end)
            .max(crate::musical_time::PPQN);

        Some((track_id, clip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::musical_time::PPQN;

    #[test]
    fn midi_recorder_pairs_notes_and_closes_stuck_notes_on_stop() {
        let tempo = TempoMap::constant(120.0);
        let sr = 48_000;
        let mut rec = MidiRecorder::new();
        rec.arm_track(Some(7));
        assert!(rec.start_recording(0, &tempo, sr));

        // Note 60 pressed at t=0, released at 24000 samples (1 beat = 960 ticks)
        rec.record_message(
            MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
            0,
            &tempo,
            sr,
        );
        rec.record_message(
            MidiMessage::NoteOff {
                channel: 0,
                note: 60,
                velocity: 0,
            },
            24_000,
            &tempo,
            sr,
        );

        // Note 64 pressed at 24000 samples, NEVER released before stop!
        rec.record_message(
            MidiMessage::NoteOn {
                channel: 0,
                note: 64,
                velocity: 88,
            },
            24_000,
            &tempo,
            sr,
        );

        let (tid, clip) = rec.stop_recording(48_000, &tempo, sr).unwrap();
        assert_eq!(tid, 7);
        assert_eq!(clip.notes.len(), 2);

        let n1 = clip.notes.iter().find(|n| n.pitch == 60).unwrap();
        assert_eq!(n1.start_tick, 0);
        assert_eq!(n1.duration_ticks, PPQN);

        let n2 = clip.notes.iter().find(|n| n.pitch == 64).unwrap();
        assert_eq!(n2.start_tick, PPQN);
        assert_eq!(n2.duration_ticks, PPQN); // Closed cleanly at stop!
    }
}
