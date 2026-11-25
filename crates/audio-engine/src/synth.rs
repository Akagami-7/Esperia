//! Real-time-safe polyphonic synthesizer instrument for MIDI playback & live preview.
//!
//! Uses a fixed-size `[Voice; MAX_VOICES]` array so processing performs zero
//! heap allocations and zero blocking operations on the audio thread.

use esperia_midi::{
    CC_ALL_NOTES_OFF, CC_ALL_SOUND_OFF, CC_MODULATION, CC_SUSTAIN_PEDAL, InstrumentKind,
    MidiMessage,
};
use std::f32::consts::TAU;

const MAX_VOICES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    note: u8,
    channel: u8,
    velocity: f32,
    phase: f32,
    stage: EnvStage,
    env: f32,
    sustained_by_pedal: bool,
    age: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            note: 0,
            channel: 0,
            velocity: 0.0,
            phase: 0.0,
            stage: EnvStage::Idle,
            env: 0.0,
            sustained_by_pedal: false,
            age: 0,
        }
    }
}

/// Polyphonic synthesizer with built-in presets, pitch bend, modulation vibrato,
/// and sustain pedal support.
pub struct PolySynth {
    pub track_id: u64,
    pub instrument: InstrumentKind,
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    voice_seq: u64,
    pitch_bend_semitones: f32,
    modulation: f32,
    lfo_phase: f32,
    sustain_pedal: bool,
}

impl PolySynth {
    pub fn new(track_id: u64, instrument: InstrumentKind, sample_rate: f32) -> Self {
        Self {
            track_id,
            instrument,
            sample_rate: sample_rate.max(1.0),
            voices: [Voice::default(); MAX_VOICES],
            voice_seq: 0,
            pitch_bend_semitones: 0.0,
            modulation: 0.0,
            lfo_phase: 0.0,
            sustain_pedal: false,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
    }

    /// Process a single [`MidiMessage`] immediately. Real-time safe.
    pub fn handle_message(&mut self, msg: MidiMessage) {
        match msg {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } => {
                if velocity == 0 {
                    self.note_off(channel, note);
                } else {
                    self.note_on(channel, note, velocity);
                }
            }
            MidiMessage::NoteOff { channel, note, .. } => {
                self.note_off(channel, note);
            }
            MidiMessage::PitchBend { value, .. } => {
                // ±2 semitones standard pitch bend range
                self.pitch_bend_semitones = (f32::from(value) / 8192.0) * 2.0;
            }
            MidiMessage::ControlChange {
                controller, value, ..
            } => match controller {
                CC_MODULATION => {
                    self.modulation = f32::from(value) / 127.0;
                }
                CC_SUSTAIN_PEDAL => {
                    let down = value >= 64;
                    if self.sustain_pedal && !down {
                        // Release any voices that were only held by the sustain pedal
                        for v in &mut self.voices {
                            if v.active && v.sustained_by_pedal {
                                v.sustained_by_pedal = false;
                                v.stage = EnvStage::Release;
                            }
                        }
                    }
                    self.sustain_pedal = down;
                }
                CC_ALL_NOTES_OFF => self.all_notes_off(),
                CC_ALL_SOUND_OFF => self.all_sound_off(),
                _ => {}
            },
            MidiMessage::ProgramChange { program, .. } => {
                self.instrument = InstrumentKind::from_program(program);
            }
        }
    }

    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.voice_seq = self.voice_seq.saturating_add(1);
        let vel = (f32::from(velocity) / 127.0).clamp(0.05, 1.0);

        // Reuse existing voice for the same note, or an idle voice, or steal oldest.
        let mut target_idx = self
            .voices
            .iter()
            .position(|v| v.active && v.note == note && v.channel == channel);

        if target_idx.is_none() {
            target_idx = self.voices.iter().position(|v| !v.active);
        }
        if target_idx.is_none() {
            target_idx = self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.age)
                .map(|(i, _)| i);
        }

        if let Some(idx) = target_idx {
            self.voices[idx] = Voice {
                active: true,
                note,
                channel,
                velocity: vel,
                phase: 0.0,
                stage: EnvStage::Attack,
                env: 0.0,
                sustained_by_pedal: false,
                age: self.voice_seq,
            };
        }
    }

    fn note_off(&mut self, channel: u8, note: u8) {
        for v in &mut self.voices {
            if v.active && v.note == note && v.channel == channel {
                if self.sustain_pedal {
                    v.sustained_by_pedal = true;
                } else {
                    v.stage = EnvStage::Release;
                }
            }
        }
    }

    /// Release all currently sounding notes cleanly (prevents stuck notes).
    pub fn all_notes_off(&mut self) {
        self.sustain_pedal = false;
        for v in &mut self.voices {
            if v.active {
                v.sustained_by_pedal = false;
                v.stage = EnvStage::Release;
            }
        }
    }

    /// Silence all voices immediately.
    pub fn all_sound_off(&mut self) {
        self.sustain_pedal = false;
        for v in &mut self.voices {
            *v = Voice::default();
        }
    }

    /// Returns `true` if any voice is currently sounding.
    pub fn has_active_voices(&self) -> bool {
        self.voices.iter().any(|v| v.active)
    }

    /// Render `frames` of interleaved stereo audio and **add** it into `stereo_out`.
    pub fn render_stereo_add(&mut self, stereo_out: &mut [f32], frames: usize) {
        let actual_frames = frames.min(stereo_out.len() / 2);
        if actual_frames == 0 || !self.has_active_voices() {
            return;
        }

        let sr = self.sample_rate;
        let (attack_rate, decay_rate, sustain_level, release_rate) = match self.instrument {
            InstrumentKind::ElectricPiano => (
                1.0 / (0.004 * sr),
                1.0 / (0.45 * sr),
                0.35f32,
                1.0 / (0.25 * sr),
            ),
            InstrumentKind::SineSynth => (
                1.0 / (0.010 * sr),
                1.0 / (0.15 * sr),
                0.75f32,
                1.0 / (0.12 * sr),
            ),
            InstrumentKind::TriangleSynth => (
                1.0 / (0.008 * sr),
                1.0 / (0.20 * sr),
                0.65f32,
                1.0 / (0.15 * sr),
            ),
            InstrumentKind::SawSynth => (
                1.0 / (0.012 * sr),
                1.0 / (0.25 * sr),
                0.60f32,
                1.0 / (0.18 * sr),
            ),
        };

        let lfo_inc = 5.5 / sr;
        let vib_depth = self.modulation * 0.5; // up to ±0.5 semitones vibrato

        for f in 0..actual_frames {
            self.lfo_phase = (self.lfo_phase + lfo_inc) % 1.0;
            let lfo_semi = (self.lfo_phase * TAU).sin() * vib_depth;
            let total_bend = self.pitch_bend_semitones + lfo_semi;

            let mut sum = 0.0f32;
            for v in &mut self.voices {
                if !v.active {
                    continue;
                }

                // Update ADSR envelope
                match v.stage {
                    EnvStage::Idle => {
                        v.active = false;
                        continue;
                    }
                    EnvStage::Attack => {
                        v.env += attack_rate;
                        if v.env >= 1.0 {
                            v.env = 1.0;
                            v.stage = EnvStage::Decay;
                        }
                    }
                    EnvStage::Decay => {
                        v.env -= decay_rate;
                        if v.env <= sustain_level {
                            v.env = sustain_level;
                            v.stage = EnvStage::Sustain;
                        }
                    }
                    EnvStage::Sustain => {
                        v.env = sustain_level;
                    }
                    EnvStage::Release => {
                        v.env -= release_rate;
                        if v.env <= 0.0 {
                            v.env = 0.0;
                            v.active = false;
                            v.stage = EnvStage::Idle;
                            continue;
                        }
                    }
                }

                let midi_note = f32::from(v.note) + total_bend;
                let freq = 440.0 * 2.0f32.powf((midi_note - 69.0) / 12.0);
                let phase_inc = freq / sr;
                v.phase = (v.phase + phase_inc) % 1.0;

                let osc = match self.instrument {
                    InstrumentKind::SineSynth => (v.phase * TAU).sin(),
                    InstrumentKind::TriangleSynth => 1.0 - 4.0 * (v.phase - 0.5).abs(),
                    InstrumentKind::SawSynth => 2.0 * v.phase - 1.0,
                    InstrumentKind::ElectricPiano => {
                        // Fundamental + soft 2nd & 3rd harmonic tine
                        let p = v.phase * TAU;
                        0.72 * p.sin() + 0.20 * (2.0 * p).sin() + 0.08 * (3.0 * p).sin()
                    }
                };

                sum += osc * v.env * v.velocity * 0.25;
            }

            let idx = f * 2;
            stereo_out[idx] += sum;
            stereo_out[idx + 1] += sum;
        }
    }
}
