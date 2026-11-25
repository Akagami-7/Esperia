//! Integration tests for M6: MIDI recording, quantization, musical time,
//! scheduling, and stuck-note prevention.

use esperia_midi::{
    ActiveNoteTracker, MidiClip, MidiInputRing, MidiMessage, MidiRecorder, MidiSchedule, MidiTrack,
    PPQN, QuantizeGrid, TempoMap, TempoPoint, TimeSignature, VirtualMidiInput, quantize_notes,
};
use std::sync::Arc;

#[test]
fn record_quantize_and_schedule_end_to_end() {
    let ring = Arc::new(MidiInputRing::new());
    let v_in = VirtualMidiInput::new(Arc::clone(&ring));
    let mut recorder = MidiRecorder::new();
    let tempo = TempoMap::constant(120.0);
    let sr = 48_000;

    recorder.arm_track(Some(1));
    assert!(recorder.start_recording(0, &tempo, sr));

    // Simulate slightly off-grid playing around beat 1 (tick 25 -> ~625 samples) and beat 2 (tick 945 -> ~23625 samples)
    v_in.send(MidiMessage::NoteOn {
        channel: 0,
        note: 60,
        velocity: 110,
    });
    while let Some(ev) = ring.pop() {
        recorder.record_message(ev.message, 625, &tempo, sr);
    }

    v_in.send(MidiMessage::NoteOff {
        channel: 0,
        note: 60,
        velocity: 0,
    });
    while let Some(ev) = ring.pop() {
        recorder.record_message(ev.message, 12_000, &tempo, sr);
    }

    v_in.send(MidiMessage::NoteOn {
        channel: 0,
        note: 64,
        velocity: 96,
    });
    while let Some(ev) = ring.pop() {
        recorder.record_message(ev.message, 23_625, &tempo, sr);
    }

    // Stop recording while note 64 is still held -> must auto-close without stuck note
    let (_tid, mut clip) = recorder.stop_recording(48_000, &tempo, sr).unwrap();
    assert_eq!(clip.notes.len(), 2);

    // Quantize to 1/16 grid
    quantize_notes(&mut clip.notes, QuantizeGrid::Sixteenth, 1.0, false);
    assert_eq!(clip.notes[0].start_tick, 0);
    assert_eq!(clip.notes[1].start_tick, PPQN);

    // Schedule into audio samples
    let mut track = MidiTrack::new(1, "Piano");
    track.clips.push(clip);
    let sched = MidiSchedule::from_tracks(&[track], &tempo, sr);

    let c4_on = sched
        .events
        .iter()
        .find(|e| matches!(e.message, MidiMessage::NoteOn { note: 60, .. }))
        .unwrap();
    let e4_on = sched
        .events
        .iter()
        .find(|e| matches!(e.message, MidiMessage::NoteOn { note: 64, .. }))
        .unwrap();
    assert_eq!(c4_on.sample_frame, 0);
    assert_eq!(e4_on.sample_frame, 24_000);
}

#[test]
fn tempo_change_preserves_musical_ticks_and_updates_samples() {
    let mut clip = MidiClip::new(1, "Arp", 0, PPQN * 4);
    clip.add_note(PPQN * 2, PPQN, 67, 100, 0);
    let mut track = MidiTrack::new(1, "Synth");
    track.clips.push(clip);

    let tempo_120 = TempoMap::constant(120.0);
    let tempo_60 = TempoMap::constant(60.0);

    let s120 = MidiSchedule::from_tracks(&[track.clone()], &tempo_120, 48_000);
    let s60 = MidiSchedule::from_tracks(&[track], &tempo_60, 48_000);

    let on_120 = s120
        .events
        .iter()
        .find(|e| matches!(e.message, MidiMessage::NoteOn { note: 67, .. }))
        .unwrap()
        .sample_frame;
    let on_60 = s60
        .events
        .iter()
        .find(|e| matches!(e.message, MidiMessage::NoteOn { note: 67, .. }))
        .unwrap()
        .sample_frame;

    assert_eq!(on_120, 48_000); // 1.0s at 120 BPM
    assert_eq!(on_60, 96_000); // 2.0s at 60 BPM
}

#[test]
fn triplet_quantization_and_bar_beat_conversion() {
    let mut clip = MidiClip::new(1, "Triplets", 0, PPQN * 4);
    let id = clip.add_note(310, 300, 62, 100, 0);
    quantize_notes(&mut clip.notes, QuantizeGrid::EighthTriplet, 1.0, true);
    let n = clip.notes.iter().find(|n| n.id == id).unwrap();
    assert_eq!(n.start_tick, 320); // Exact 1/8T tick at 960 PPQN
    assert_eq!(n.duration_ticks, 320);

    let map = TempoMap::from_points(vec![TempoPoint {
        tick: 0,
        bpm: 120.0,
    }]);
    assert_eq!(map.bpm_at_tick(320), 120.0);
    assert_eq!(
        TempoMap::tick_to_bar_beat_tick(PPQN * 4 + 320, TimeSignature::default()),
        (2, 1, 320)
    );
}

#[test]
fn stuck_note_tracker_clears_all_channels_and_sustain() {
    let mut tracker = ActiveNoteTracker::new();
    tracker.track_message(MidiMessage::NoteOn {
        channel: 0,
        note: 60,
        velocity: 100,
    });
    tracker.track_message(MidiMessage::NoteOn {
        channel: 3,
        note: 72,
        velocity: 80,
    });
    tracker.track_message(MidiMessage::sustain_pedal(3, true));
    assert!(tracker.has_active_notes());

    let mut msgs = Vec::new();
    tracker.flush_all_notes_off(|m| msgs.push(m));
    assert!(!tracker.has_active_notes());
    assert!(msgs.contains(&MidiMessage::sustain_pedal(3, false)));
    assert!(msgs.contains(&MidiMessage::NoteOff {
        channel: 0,
        note: 60,
        velocity: 0
    }));
    assert!(msgs.contains(&MidiMessage::NoteOff {
        channel: 3,
        note: 72,
        velocity: 0
    }));
}
