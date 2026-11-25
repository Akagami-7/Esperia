//! MIDI track model and built-in instrument selector.

use crate::midi_clip::MidiClip;
use serde::{Deserialize, Serialize};

/// Built-in polyphonic synthesizer instrument presets for MIDI tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum InstrumentKind {
    #[default]
    ElectricPiano,
    SineSynth,
    TriangleSynth,
    SawSynth,
}

impl InstrumentKind {
    pub const ALL: [Self; 4] = [
        Self::ElectricPiano,
        Self::SineSynth,
        Self::TriangleSynth,
        Self::SawSynth,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ElectricPiano => "Electric Piano",
            Self::SineSynth => "Sine Synth",
            Self::TriangleSynth => "Triangle Synth",
            Self::SawSynth => "Saw Synth",
        }
    }

    pub fn from_program(program: u8) -> Self {
        match program % 4 {
            0 => Self::ElectricPiano,
            1 => Self::SineSynth,
            2 => Self::TriangleSynth,
            _ => Self::SawSynth,
        }
    }
}

/// A MIDI/Instrument track holding MIDI clips, routing channel, and instrument preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiTrack {
    pub id: u64,
    pub name: String,
    /// MIDI channel (`0..=15`) used for outgoing notes and live input.
    pub channel: u8,
    /// MIDI Program number (`0..=127`).
    pub program: u8,
    pub instrument: InstrumentKind,
    pub gain_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub record_armed: bool,
    pub clips: Vec<MidiClip>,
}

impl MidiTrack {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            channel: 0,
            program: 0,
            instrument: InstrumentKind::ElectricPiano,
            gain_db: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            record_armed: false,
            clips: Vec::new(),
        }
    }

    pub fn find_clip(&self, clip_id: u64) -> Option<&MidiClip> {
        self.clips.iter().find(|c| c.id == clip_id)
    }

    pub fn find_clip_mut(&mut self, clip_id: u64) -> Option<&mut MidiClip> {
        self.clips.iter_mut().find(|c| c.id == clip_id)
    }
}
