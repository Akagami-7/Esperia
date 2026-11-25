//! Esperia MIDI & Musical Time crate.
//!
//! Provides MIDI 1.0 message parsing/encoding, integer-tick musical clock &
//! tempo map, MIDI clip & track models, quantization, sample-accurate
//! scheduling with stuck-note prevention, lock-free MIDI input ring & recorder,
//! and CoreMIDI hardware input on macOS.

pub mod midi_clip;
pub mod midi_device;
pub mod midi_event;
pub mod midi_quantization;
pub mod midi_recording;
pub mod midi_scheduler;
pub mod midi_track;
pub mod musical_time;

pub use midi_clip::{MidiCCEvent, MidiClip, MidiNote, MidiPitchBendEvent, MidiProgramChangeEvent};
pub use midi_device::{MidiInputConnection, MidiInputManager, MidiPortInfo, VirtualMidiInput};
pub use midi_event::{
    CC_ALL_NOTES_OFF, CC_ALL_SOUND_OFF, CC_MODULATION, CC_SUSTAIN_PEDAL, MidiError, MidiMessage,
    SampleMidiEvent, TimedMidiEvent,
};
pub use midi_quantization::{QuantizeGrid, quantize_notes, quantize_tick};
pub use midi_recording::{IncomingMidiEvent, MidiInputRing, MidiRecorder};
pub use midi_scheduler::{ActiveNoteTracker, MidiSchedule, ScheduledMidiEvent};
pub use midi_track::{InstrumentKind, MidiTrack};
pub use musical_time::{PPQN, TempoMap, TempoPoint, TimeSignature};
