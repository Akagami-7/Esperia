//! Sample-accurate MIDI scheduler and stuck-note prevention.
//!
//! Converts musical-tick [`MidiTrack`] events into sample-positioned events via
//! [`TempoMap`], and provides [`ActiveNoteTracker`] to guarantee clean `NoteOff`
//! and sustain-pedal release on transport stop, pause, seek, or loop wrap.

use crate::midi_event::MidiMessage;
use crate::midi_track::MidiTrack;
use crate::musical_time::TempoMap;

/// A MIDI message scheduled at an absolute audio sample frame for a specific track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledMidiEvent {
    pub sample_frame: u64,
    pub track_id: u64,
    pub message: MidiMessage,
}

/// Immutable, sample-sorted schedule of MIDI events shared with the audio thread.
#[derive(Debug, Clone, Default)]
pub struct MidiSchedule {
    pub events: Vec<ScheduledMidiEvent>,
}

impl MidiSchedule {
    /// Build a sample-accurate [`MidiSchedule`] from all audible MIDI tracks.
    pub fn from_tracks(tracks: &[MidiTrack], tempo_map: &TempoMap, sample_rate: u32) -> Self {
        let any_solo = tracks.iter().any(|t| t.soloed);
        let mut events = Vec::new();

        for track in tracks {
            let audible = if any_solo {
                track.soloed && !track.muted
            } else {
                !track.muted
            };
            if !audible {
                continue;
            }

            // Ensure track's initial instrument/program is set at frame 0.
            events.push(ScheduledMidiEvent {
                sample_frame: 0,
                track_id: track.id,
                message: MidiMessage::ProgramChange {
                    channel: track.channel,
                    program: track.program,
                },
            });

            for clip in &track.clips {
                for timed in clip.to_timed_events() {
                    let sample_frame = tempo_map.tick_to_sample(timed.tick, sample_rate);
                    events.push(ScheduledMidiEvent {
                        sample_frame,
                        track_id: track.id,
                        message: timed.message,
                    });
                }
            }
        }

        // Sort by sample_frame, keeping NoteOff before NoteOn on the same frame.
        events.sort_by_key(|ev| {
            let order = match ev.message {
                MidiMessage::ProgramChange { .. } => 0u8,
                MidiMessage::ControlChange { .. } | MidiMessage::PitchBend { .. } => 1,
                MidiMessage::NoteOff { .. } => 2,
                MidiMessage::NoteOn { .. } => 3,
            };
            (ev.sample_frame, order)
        });

        Self { events }
    }

    /// Return the slice of events whose `sample_frame` lies in `[start_frame, end_frame)`.
    pub fn events_in_range(&self, start_frame: u64, end_frame: u64) -> &[ScheduledMidiEvent] {
        if start_frame >= end_frame || self.events.is_empty() {
            return &[];
        }
        let lo = self
            .events
            .partition_point(|e| e.sample_frame < start_frame);
        let hi = self.events.partition_point(|e| e.sample_frame < end_frame);
        &self.events[lo..hi]
    }
}

/// Preallocated, zero-allocation tracker of currently sounding `(channel, note)`
/// pairs and sustain pedal state.
///
/// Used on the real-time audio thread to emit `NoteOff` for any active notes
/// whenever playback stops, pauses, seeks, or wraps around a loop boundary.
pub struct ActiveNoteTracker {
    /// Active count per `(channel * 128 + note)`.
    active_counts: [u8; 16 * 128],
    /// Sustain pedal down state per channel (`0..16`).
    sustain_down: [bool; 16],
}

impl Default for ActiveNoteTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ActiveNoteTracker {
    pub const fn new() -> Self {
        Self {
            active_counts: [0; 16 * 128],
            sustain_down: [false; 16],
        }
    }

    /// Update tracker state for an outgoing [`MidiMessage`].
    #[inline]
    pub fn track_message(&mut self, msg: MidiMessage) {
        match msg {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } => {
                let idx = usize::from(channel & 0x0F) * 128 + usize::from(note & 0x7F);
                if velocity > 0 {
                    self.active_counts[idx] = self.active_counts[idx].saturating_add(1);
                } else {
                    self.active_counts[idx] = self.active_counts[idx].saturating_sub(1);
                }
            }
            MidiMessage::NoteOff { channel, note, .. } => {
                let idx = usize::from(channel & 0x0F) * 128 + usize::from(note & 0x7F);
                self.active_counts[idx] = 0;
            }
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => {
                if controller == crate::midi_event::CC_SUSTAIN_PEDAL {
                    self.sustain_down[usize::from(channel & 0x0F)] = value >= 64;
                } else if controller == crate::midi_event::CC_ALL_NOTES_OFF
                    || controller == crate::midi_event::CC_ALL_SOUND_OFF
                {
                    let base = usize::from(channel & 0x0F) * 128;
                    self.active_counts[base..base + 128].fill(0);
                }
            }
            MidiMessage::PitchBend { .. } | MidiMessage::ProgramChange { .. } => {}
        }
    }

    /// Returns `true` if any note or sustain pedal is currently active.
    #[inline]
    pub fn has_active_notes(&self) -> bool {
        self.sustain_down.iter().any(|&s| s) || self.active_counts.iter().any(|&c| c > 0)
    }

    /// Emit `Sustain Off` (CC 64 = 0) and `NoteOff` for every currently active
    /// note without performing any heap allocations, and reset tracker state.
    pub fn flush_all_notes_off(&mut self, mut emit: impl FnMut(MidiMessage)) {
        for (ch, sustain) in self.sustain_down.iter_mut().enumerate() {
            if *sustain {
                *sustain = false;
                emit(MidiMessage::sustain_pedal(ch as u8, false));
            }
        }
        for ch in 0..16usize {
            let base = ch * 128;
            for note in 0..128usize {
                if self.active_counts[base + note] > 0 {
                    self.active_counts[base + note] = 0;
                    emit(MidiMessage::NoteOff {
                        channel: ch as u8,
                        note: note as u8,
                        velocity: 0,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_clip::MidiClip;
    use crate::musical_time::PPQN;

    #[test]
    fn schedule_converts_ticks_to_samples() {
        let mut track = MidiTrack::new(1, "Keys");
        let mut clip = MidiClip::new(10, "Pattern", 0, PPQN * 4);
        clip.add_note(PPQN, PPQN, 60, 100, 0); // beat 2 (tick 960 -> 24000 samples @ 120 BPM, 48kHz)
        track.clips.push(clip);

        let tempo = TempoMap::constant(120.0);
        let sched = MidiSchedule::from_tracks(&[track], &tempo, 48_000);

        let note_on = sched
            .events
            .iter()
            .find(|e| matches!(e.message, MidiMessage::NoteOn { note: 60, .. }))
            .unwrap();
        assert_eq!(note_on.sample_frame, 24_000);

        let note_off = sched
            .events
            .iter()
            .find(|e| matches!(e.message, MidiMessage::NoteOff { note: 60, .. }))
            .unwrap();
        assert_eq!(note_off.sample_frame, 48_000);
    }

    #[test]
    fn active_note_tracker_flushes_stuck_notes_and_sustain() {
        let mut tracker = ActiveNoteTracker::new();
        tracker.track_message(MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        });
        tracker.track_message(MidiMessage::NoteOn {
            channel: 0,
            note: 64,
            velocity: 90,
        });
        tracker.track_message(MidiMessage::sustain_pedal(0, true));

        let mut flushed = Vec::new();
        tracker.flush_all_notes_off(|m| flushed.push(m));

        assert!(!tracker.has_active_notes());
        assert_eq!(flushed.len(), 3); // 1 sustain off + 2 note offs
    }
}
