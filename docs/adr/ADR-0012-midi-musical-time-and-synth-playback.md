# ADR-0012 — MIDI 1.0, Musical Time System, and Polyphonic Synth Playback

**Date:** 2026-11-25  
**Status:** Accepted  
**Milestone:** M6 — MIDI & Musical Composition System  
**Authors:** Esperia Engineering

---

## Context

Milestone 6 adds MIDI composition, recording, quantization, a Piano Roll editor,
and instrument playback to Esperia.  Audio clips are naturally positioned in
audio sample frames, whereas musical notes, chords, and rhythmic divisions must
remain anchored to musical time (bars, beats, 1/4, 1/8, 1/16, triplets) even
when the project tempo changes.

In addition, real-time audio safety rules forbid heap allocations, blocking
locks, or hardware MIDI driver calls inside the audio callback.

---

## Decision

### 1. Integer Musical Clock at 960 PPQN + `TempoMap`

All MIDI note start positions and durations are stored in integer ticks at
`PPQN = 960` pulses per quarter note.  960 is evenly divisible by 2, 3, 4, 5, 6,
8, 12, 16, and 32, so straight notes and triplets (`1/4T = 640`, `1/8T = 320`,
`1/16T = 160`) are exact integers with zero floating-point drift.

`TempoMap` maintains sorted `TempoPoint`s and converts between musical ticks,
seconds, and audio sample frames at schedule build time (`MidiSchedule::from_tracks`)
and during MIDI recording (`TempoMap::sample_to_tick`).

### 2. Dedicated `esperia-midi` Crate with Zero External Driver Dependencies

`crates/midi/` implements:
- `midi_event.rs` — MIDI 1.0 message model (Note On/Off, Velocity, Pitch Bend,
  Modulation CC 1, Control Change, Sustain Pedal CC 64, Program Change).
- `musical_time.rs` — 960 PPQN clock, `TimeSignature`, and `TempoMap`.
- `midi_clip.rs` & `midi_track.rs` — MIDI clips, notes, controller events, and
  instrument tracks.
- `midi_quantization.rs` — straight & triplet rhythmic grid quantization.
- `midi_scheduler.rs` — sample-accurate `MidiSchedule` + `ActiveNoteTracker`.
- `midi_recording.rs` — lock-free `MidiInputRing` + `MidiRecorder`.
- `midi_device.rs` — macOS `CoreMIDI` FFI input enumeration/streaming plus
  `VirtualMidiInput` for UI preview and cross-platform testing.

### 3. Real-Time-Safe `PolySynth` and Stuck-Note Prevention

`crates/audio-engine/src/synth.rs` implements a 32-voice polyphonic soft synth
(`PolySynth`) backed by a fixed `[Voice; 32]` array with zero heap allocations.
It supports `ElectricPiano`, `SineSynth`, `TriangleSynth`, and `SawSynth`
presets, velocity, pitch bend (±2 semitones), modulation vibrato, sustain pedal,
and program changes.

`ActiveNoteTracker` uses a fixed `[u8; 16 * 128]` array on the audio thread to
track active notes and sustain state, flushing `Sustain Off` and `NoteOff` for
every sounding note on `Stop`, `Pause`, `Seek`, and loop wraps.  Similarly,
`MidiRecorder::stop_recording` automatically closes any held notes at the stop
tick so recorded clips never contain hanging notes.

---

## Consequences

- Tempo changes automatically re-map MIDI schedules to new sample frames without
  mutating note positions in `MidiClip`.
- Hardware MIDI keyboards on macOS stream into `MidiInputRing` without external
  crate dependencies (`midir` not required).
- The audio callback remains 100% allocation-free and lock-free.
