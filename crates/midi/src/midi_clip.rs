//! MIDI clip data model: notes, CC automation, pitch bend, and program changes.

use crate::midi_event::{MidiMessage, TimedMidiEvent};
use crate::musical_time::PPQN;
use serde::{Deserialize, Serialize};

/// A single MIDI note stored in clip-relative musical ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiNote {
    pub id: u64,
    /// Note start position relative to the clip's `start_tick`.
    pub start_tick: u64,
    /// Note duration in musical ticks (must be >= 1).
    pub duration_ticks: u64,
    /// MIDI note pitch (`0..=127`, where `60` = Middle C4).
    pub pitch: u8,
    /// Note-on velocity (`1..=127`).
    pub velocity: u8,
    /// MIDI channel (`0..=15`).
    pub channel: u8,
}

impl MidiNote {
    /// End tick relative to the clip's `start_tick`.
    #[inline]
    pub fn end_tick(&self) -> u64 {
        self.start_tick.saturating_add(self.duration_ticks.max(1))
    }
}

/// A Control Change event stored in clip-relative musical ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiCCEvent {
    pub id: u64,
    pub tick: u64,
    pub controller: u8,
    pub value: u8,
    pub channel: u8,
}

/// A Pitch Bend event stored in clip-relative musical ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiPitchBendEvent {
    pub id: u64,
    pub tick: u64,
    /// 14-bit signed bend (`-8192..=8191`).
    pub value: i16,
    pub channel: u8,
}

/// A Program Change event stored in clip-relative musical ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiProgramChangeEvent {
    pub id: u64,
    pub tick: u64,
    pub program: u8,
    pub channel: u8,
}

/// A musical MIDI clip positioned on a MIDI track's timeline in ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiClip {
    pub id: u64,
    pub name: String,
    /// Timeline start position in project ticks.
    pub start_tick: u64,
    /// Clip length in project ticks.
    pub length_ticks: u64,
    pub notes: Vec<MidiNote>,
    #[serde(default)]
    pub cc_events: Vec<MidiCCEvent>,
    #[serde(default)]
    pub pitch_bends: Vec<MidiPitchBendEvent>,
    #[serde(default)]
    pub program_changes: Vec<MidiProgramChangeEvent>,
    /// Monotonic local ID counter for notes/events inside this clip.
    #[serde(default = "default_next_note_id")]
    pub next_note_id: u64,
}

fn default_next_note_id() -> u64 {
    1
}

impl MidiClip {
    /// Create an empty MIDI clip of `length_ticks` (defaults to 4 bars if `0`).
    pub fn new(id: u64, name: impl Into<String>, start_tick: u64, length_ticks: u64) -> Self {
        Self {
            id,
            name: name.into(),
            start_tick,
            length_ticks: if length_ticks == 0 {
                PPQN * 16
            } else {
                length_ticks
            },
            notes: Vec::new(),
            cc_events: Vec::new(),
            pitch_bends: Vec::new(),
            program_changes: Vec::new(),
            next_note_id: 1,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_note_id.max(1);
        self.next_note_id = id.saturating_add(1);
        id
    }

    /// Add a note to the clip and return its assigned note ID.
    pub fn add_note(
        &mut self,
        start_tick: u64,
        duration_ticks: u64,
        pitch: u8,
        velocity: u8,
        channel: u8,
    ) -> u64 {
        let id = self.alloc_id();
        self.notes.push(MidiNote {
            id,
            start_tick,
            duration_ticks: duration_ticks.max(1),
            pitch: pitch.min(127),
            velocity: velocity.clamp(1, 127),
            channel: channel & 0x0F,
        });
        id
    }

    /// Move an existing note to a new clip-relative start tick and pitch.
    pub fn move_note(&mut self, note_id: u64, new_start_tick: u64, new_pitch: u8) -> bool {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == note_id) {
            n.start_tick = new_start_tick;
            n.pitch = new_pitch.min(127);
            true
        } else {
            false
        }
    }

    /// Resize an existing note's duration.
    pub fn resize_note(&mut self, note_id: u64, new_duration_ticks: u64) -> bool {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == note_id) {
            n.duration_ticks = new_duration_ticks.max(1);
            true
        } else {
            false
        }
    }

    /// Set an existing note's velocity (`1..=127`).
    pub fn set_note_velocity(&mut self, note_id: u64, velocity: u8) -> bool {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == note_id) {
            n.velocity = velocity.clamp(1, 127);
            true
        } else {
            false
        }
    }

    /// Delete a note by ID.
    pub fn delete_note(&mut self, note_id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != note_id);
        self.notes.len() != before
    }

    /// Add a Control Change event (including Modulation CC 1 or Sustain CC 64).
    pub fn add_cc(&mut self, tick: u64, controller: u8, value: u8, channel: u8) -> u64 {
        let id = self.alloc_id();
        self.cc_events.push(MidiCCEvent {
            id,
            tick,
            controller: controller & 0x7F,
            value: value & 0x7F,
            channel: channel & 0x0F,
        });
        id
    }

    /// Add a Pitch Bend event (`-8192..=8191`).
    pub fn add_pitch_bend(&mut self, tick: u64, value: i16, channel: u8) -> u64 {
        let id = self.alloc_id();
        self.pitch_bends.push(MidiPitchBendEvent {
            id,
            tick,
            value: value.clamp(-8192, 8191),
            channel: channel & 0x0F,
        });
        id
    }

    /// Add a Program Change event (`0..=127`).
    pub fn add_program_change(&mut self, tick: u64, program: u8, channel: u8) -> u64 {
        let id = self.alloc_id();
        self.program_changes.push(MidiProgramChangeEvent {
            id,
            tick,
            program: program & 0x7F,
            channel: channel & 0x0F,
        });
        id
    }

    /// Expand all clip notes and controller events into absolute project-tick [`TimedMidiEvent`]s,
    /// sorted by tick (with `NoteOff` ordered before `NoteOn` at identical ticks to prevent
    /// re-triggered note clipping).
    pub fn to_timed_events(&self) -> Vec<TimedMidiEvent> {
        let mut events = Vec::with_capacity(
            self.notes.len() * 2
                + self.cc_events.len()
                + self.pitch_bends.len()
                + self.program_changes.len(),
        );

        let clip_end = self.start_tick.saturating_add(self.length_ticks);

        for pc in &self.program_changes {
            let abs_tick = self.start_tick.saturating_add(pc.tick);
            if abs_tick < clip_end {
                events.push(TimedMidiEvent {
                    tick: abs_tick,
                    message: MidiMessage::ProgramChange {
                        channel: pc.channel,
                        program: pc.program,
                    },
                });
            }
        }

        for cc in &self.cc_events {
            let abs_tick = self.start_tick.saturating_add(cc.tick);
            if abs_tick < clip_end {
                events.push(TimedMidiEvent {
                    tick: abs_tick,
                    message: MidiMessage::ControlChange {
                        channel: cc.channel,
                        controller: cc.controller,
                        value: cc.value,
                    },
                });
            }
        }

        for pb in &self.pitch_bends {
            let abs_tick = self.start_tick.saturating_add(pb.tick);
            if abs_tick < clip_end {
                events.push(TimedMidiEvent {
                    tick: abs_tick,
                    message: MidiMessage::PitchBend {
                        channel: pb.channel,
                        value: pb.value,
                    },
                });
            }
        }

        for note in &self.notes {
            let on_tick = self.start_tick.saturating_add(note.start_tick);
            if on_tick >= clip_end {
                continue;
            }
            let off_tick = self
                .start_tick
                .saturating_add(note.end_tick())
                .min(clip_end);
            events.push(TimedMidiEvent {
                tick: on_tick,
                message: MidiMessage::NoteOn {
                    channel: note.channel,
                    note: note.pitch,
                    velocity: note.velocity,
                },
            });
            events.push(TimedMidiEvent {
                tick: off_tick,
                message: MidiMessage::NoteOff {
                    channel: note.channel,
                    note: note.pitch,
                    velocity: 0,
                },
            });
        }

        // Sort by tick, placing NoteOff before NoteOn at the same tick.
        events.sort_by_key(|ev| {
            let order = match ev.message {
                MidiMessage::ProgramChange { .. } => 0u8,
                MidiMessage::ControlChange { .. } | MidiMessage::PitchBend { .. } => 1,
                MidiMessage::NoteOff { .. } => 2,
                MidiMessage::NoteOn { .. } => 3,
            };
            (ev.tick, order)
        });

        events
    }
}
