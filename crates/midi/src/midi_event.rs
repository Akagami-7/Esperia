//! MIDI 1.0 message and event model.
//!
//! Supports Note On, Note Off, Pitch Bend, Modulation (CC 1), Control Change,
//! Sustain Pedal (CC 64), and Program Change, with lossless byte encoding/decoding
//! and structured domain errors.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Standard MIDI CC number for Modulation Wheel.
pub const CC_MODULATION: u8 = 1;
/// Standard MIDI CC number for Sustain / Damper Pedal.
pub const CC_SUSTAIN_PEDAL: u8 = 64;
/// Standard MIDI CC number for All Sound Off.
pub const CC_ALL_SOUND_OFF: u8 = 120;
/// Standard MIDI CC number for All Notes Off.
pub const CC_ALL_NOTES_OFF: u8 = 123;

/// Errors returned when parsing or constructing MIDI 1.0 messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiError {
    EmptyMessage,
    TruncatedMessage {
        status: u8,
        expected: usize,
        got: usize,
    },
    UnsupportedStatus(u8),
    InvalidDataByte(u8),
    DeviceUnavailable(String),
    PortNotFound(String),
    DeviceConnectionFailed(String),
}

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMessage => write!(f, "empty MIDI message"),
            Self::TruncatedMessage {
                status,
                expected,
                got,
            } => write!(
                f,
                "truncated MIDI message for status 0x{status:02X}: expected {expected} bytes, got {got}"
            ),
            Self::UnsupportedStatus(s) => write!(f, "unsupported MIDI status byte 0x{s:02X}"),
            Self::InvalidDataByte(b) => write!(f, "invalid MIDI data byte 0x{b:02X} (bit 7 set)"),
            Self::DeviceUnavailable(msg) => write!(f, "MIDI device unavailable: {msg}"),
            Self::PortNotFound(name) => write!(f, "MIDI port not found: {name}"),
            Self::DeviceConnectionFailed(msg) => write!(f, "MIDI connection failed: {msg}"),
        }
    }
}

impl std::error::Error for MidiError {}

/// A MIDI 1.0 channel voice message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiMessage {
    /// Note On (`0x90 | channel`, note `0..=127`, velocity `1..=127`).
    NoteOn { channel: u8, note: u8, velocity: u8 },
    /// Note Off (`0x80 | channel`, note `0..=127`, velocity `0..=127`).
    NoteOff { channel: u8, note: u8, velocity: u8 },
    /// 14-bit Pitch Bend centered at `0` (`-8192..=8191`).
    PitchBend { channel: u8, value: i16 },
    /// Control Change (`0xB0 | channel`, controller `0..=127`, value `0..=127`).
    /// Includes Modulation (CC 1) and Sustain Pedal (CC 64).
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    /// Program Change (`0xC0 | channel`, program `0..=127`).
    ProgramChange { channel: u8, program: u8 },
}

impl MidiMessage {
    /// Construct a Modulation Wheel CC (CC 1) message.
    #[inline]
    pub fn modulation(channel: u8, value: u8) -> Self {
        Self::ControlChange {
            channel: channel & 0x0F,
            controller: CC_MODULATION,
            value: value & 0x7F,
        }
    }

    /// Construct a Sustain Pedal CC (CC 64) message (`on = true` -> 127, `false` -> 0).
    #[inline]
    pub fn sustain_pedal(channel: u8, on: bool) -> Self {
        Self::ControlChange {
            channel: channel & 0x0F,
            controller: CC_SUSTAIN_PEDAL,
            value: if on { 127 } else { 0 },
        }
    }

    /// Return the MIDI channel (`0..=15`) of this message.
    #[inline]
    pub fn channel(&self) -> u8 {
        match *self {
            Self::NoteOn { channel, .. }
            | Self::NoteOff { channel, .. }
            | Self::PitchBend { channel, .. }
            | Self::ControlChange { channel, .. }
            | Self::ProgramChange { channel, .. } => channel,
        }
    }

    /// Returns `true` if this is a sustain pedal CC 64 message, returning its boolean state (`>= 64`).
    #[inline]
    pub fn as_sustain_pedal(&self) -> Option<bool> {
        match *self {
            Self::ControlChange {
                controller: CC_SUSTAIN_PEDAL,
                value,
                ..
            } => Some(value >= 64),
            _ => None,
        }
    }

    /// Returns `true` if this is a modulation wheel CC 1 message, returning normalized `0.0..=1.0`.
    #[inline]
    pub fn as_modulation(&self) -> Option<f32> {
        match *self {
            Self::ControlChange {
                controller: CC_MODULATION,
                value,
                ..
            } => Some(f32::from(value) / 127.0),
            _ => None,
        }
    }

    /// Parse a single MIDI 1.0 message from raw bytes.
    ///
    /// Note On with velocity 0 is normalized to `MidiMessage::NoteOff` per MIDI 1.0 spec.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MidiError> {
        let &status = bytes.first().ok_or(MidiError::EmptyMessage)?;
        if status < 0x80 {
            return Err(MidiError::UnsupportedStatus(status));
        }

        let kind = status & 0xF0;
        let channel = status & 0x0F;

        let check_data = |idx: usize| -> Result<u8, MidiError> {
            let b = *bytes.get(idx).ok_or(MidiError::TruncatedMessage {
                status,
                expected: if kind == 0xC0 { 2 } else { 3 },
                got: bytes.len(),
            })?;
            if b & 0x80 != 0 {
                return Err(MidiError::InvalidDataByte(b));
            }
            Ok(b)
        };

        match kind {
            0x80 => {
                let note = check_data(1)?;
                let velocity = check_data(2)?;
                Ok(Self::NoteOff {
                    channel,
                    note,
                    velocity,
                })
            }
            0x90 => {
                let note = check_data(1)?;
                let velocity = check_data(2)?;
                if velocity == 0 {
                    Ok(Self::NoteOff {
                        channel,
                        note,
                        velocity: 0,
                    })
                } else {
                    Ok(Self::NoteOn {
                        channel,
                        note,
                        velocity,
                    })
                }
            }
            0xB0 => {
                let controller = check_data(1)?;
                let value = check_data(2)?;
                Ok(Self::ControlChange {
                    channel,
                    controller,
                    value,
                })
            }
            0xC0 => {
                let program = check_data(1)?;
                Ok(Self::ProgramChange { channel, program })
            }
            0xE0 => {
                let lsb = check_data(1)? as i16;
                let msb = check_data(2)? as i16;
                let raw_14bit = (msb << 7) | lsb;
                Ok(Self::PitchBend {
                    channel,
                    value: raw_14bit - 8192,
                })
            }
            _ => Err(MidiError::UnsupportedStatus(status)),
        }
    }

    /// Encode this message into MIDI 1.0 wire bytes (`([b0, b1, b2], len)`).
    pub fn to_bytes(&self) -> ([u8; 3], usize) {
        match *self {
            Self::NoteOff {
                channel,
                note,
                velocity,
            } => ([0x80 | (channel & 0x0F), note & 0x7F, velocity & 0x7F], 3),
            Self::NoteOn {
                channel,
                note,
                velocity,
            } => ([0x90 | (channel & 0x0F), note & 0x7F, velocity & 0x7F], 3),
            Self::ControlChange {
                channel,
                controller,
                value,
            } => (
                [0xB0 | (channel & 0x0F), controller & 0x7F, value & 0x7F],
                3,
            ),
            Self::ProgramChange { channel, program } => {
                ([0xC0 | (channel & 0x0F), program & 0x7F, 0], 2)
            }
            Self::PitchBend { channel, value } => {
                let raw = (value.clamp(-8192, 8191) + 8192) as u16;
                let lsb = (raw & 0x7F) as u8;
                let msb = ((raw >> 7) & 0x7F) as u8;
                ([0xE0 | (channel & 0x0F), lsb, msb], 3)
            }
        }
    }
}

/// A MIDI message stamped with an integer musical tick position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedMidiEvent {
    pub tick: u64,
    pub message: MidiMessage,
}

/// A MIDI message stamped with an audio-buffer-relative sample offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleMidiEvent {
    pub sample_offset: usize,
    pub message: MidiMessage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_midi_messages() {
        let msgs = [
            MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
            MidiMessage::NoteOff {
                channel: 2,
                note: 64,
                velocity: 64,
            },
            MidiMessage::PitchBend {
                channel: 1,
                value: -4096,
            },
            MidiMessage::PitchBend {
                channel: 1,
                value: 8191,
            },
            MidiMessage::modulation(3, 95),
            MidiMessage::sustain_pedal(0, true),
            MidiMessage::ProgramChange {
                channel: 5,
                program: 42,
            },
        ];

        for msg in msgs {
            let (bytes, len) = msg.to_bytes();
            let parsed = MidiMessage::from_bytes(&bytes[..len]).expect("parse valid bytes");
            assert_eq!(parsed, msg);
        }
    }

    #[test]
    fn note_on_velocity_zero_becomes_note_off() {
        let parsed = MidiMessage::from_bytes(&[0x92, 60, 0]).unwrap();
        assert_eq!(
            parsed,
            MidiMessage::NoteOff {
                channel: 2,
                note: 60,
                velocity: 0
            }
        );
    }
}
