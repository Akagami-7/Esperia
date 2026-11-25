//! Integration tests for the `esperia-mixer` crate.
//!
//! All tests use `MixerGraph::render_offline` so they exercise the full
//! processing graph without needing a real-time audio device.

use esperia_mixer::{
    BusConfig, ChannelConfig, MixerGraph, RouteTarget, SendRoute,
    gain::{db_to_linear, linear_to_db},
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build a stereo sine burst at 440 Hz, 48 kHz, `frames` long.
fn sine_burst(frames: usize) -> Vec<f32> {
    (0..frames * 2)
        .map(|i| {
            let frame = i / 2;
            (2.0 * std::f32::consts::PI * 440.0 * frame as f32 / 48_000.0).sin() * 0.5
        })
        .collect()
}

fn channel_cfg(id: u64) -> ChannelConfig {
    ChannelConfig {
        id,
        name: format!("ch{id}"),
        gain_db: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        output: RouteTarget::Master,
        sends: vec![],
    }
}

/// Helper: render a graph where each channel id maps to a fixed input buffer.
fn render_with_inputs(
    graph: &mut MixerGraph,
    inputs: Vec<(u64, Vec<f32>)>,
    frames: usize,
) -> Vec<f32> {
    graph
        .render_offline(frames, frames, 2, |g, _cursor, blk| {
            for (id, buf) in &inputs {
                if let Some(ch) = g.channel_input_mut(*id, blk) {
                    let len = ch.len().min(buf.len());
                    ch[..len].copy_from_slice(&buf[..len]);
                }
            }
        })
        .expect("render_offline failed")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// A single channel strip at unity gain must pass its input through to master.
#[test]
fn single_channel_unity_gain() {
    let frames = 512;
    let cfgs = [channel_cfg(1)];
    let sig = sine_burst(frames);
    let mut graph = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();

    let output = render_with_inputs(&mut graph, vec![(1, sig.clone())], frames);

    for (o, i) in output.iter().zip(sig.iter()) {
        let diff = (o - i).abs();
        assert!(
            diff < 1e-5,
            "unity-gain output diverged: got {o}, expected {i}"
        );
    }
}

/// Two channels at unity gain into master should produce summed output.
#[test]
fn two_channels_sum_to_master() {
    let frames = 512;
    let cfgs = [channel_cfg(1), channel_cfg(2)];
    let sig = sine_burst(frames);
    let mut graph = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();

    let output = render_with_inputs(&mut graph, vec![(1, sig.clone()), (2, sig.clone())], frames);

    for (o, i) in output.iter().zip(sig.iter()) {
        let expected = i * 2.0;
        let diff = (o - expected).abs();
        assert!(diff < 1e-4, "sum mismatch: got {o}, expected {expected}");
    }
}

/// A muted channel contributes zero samples to the master.
#[test]
fn muted_channel_produces_silence() {
    let frames = 256;
    let mut cfg = channel_cfg(1);
    cfg.muted = true;
    let sig = sine_burst(frames);
    let mut graph = MixerGraph::new(&[cfg], &[], 0.0, frames).unwrap();

    let output = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    for &s in &output {
        assert!(s.abs() < 1e-6, "muted channel leaked: {s}");
    }
}

/// Gain at -6 dBFS should halve the linear amplitude (≈ 0.5×).
#[test]
fn channel_gain_minus_6db() {
    let frames = 256;
    let mut cfg = channel_cfg(1);
    let gain_db = -6.0206;
    cfg.gain_db = gain_db; // exactly 0.5× linear
    // DC signal at 1.0 makes it easy to check the output level
    let sig = vec![1.0f32; frames * 2];
    let mut graph = MixerGraph::new(&[cfg], &[], 0.0, frames).unwrap();

    let output = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    for &s in &output {
        let diff = (s - 0.5).abs();
        assert!(diff < 2e-3, "gain {gain_db}: got {s}");
    }
}

/// A solo'd channel causes all other non-solo'd channels to be silenced.
#[test]
fn solo_silences_non_soloed_channels() {
    let frames = 256;
    let mut cfg1 = channel_cfg(1);
    let mut cfg2 = channel_cfg(2);
    cfg1.soloed = true;
    cfg2.soloed = false;

    let sig = vec![1.0f32; frames * 2];
    let mut graph = MixerGraph::new(&[cfg1, cfg2], &[], 0.0, frames).unwrap();

    let output = render_with_inputs(&mut graph, vec![(1, sig.clone()), (2, sig.clone())], frames);

    // Output should equal the solo'd channel only (1.0 DC)
    for &s in &output {
        let diff = (s - 1.0).abs();
        assert!(diff < 1e-4, "solo leaked other channel: got {s}");
    }
}

/// Two channels → vocal bus → master (3-node routing graph).
#[test]
fn bus_groups_channels_to_master() {
    let frames = 256;
    const BUS_ID: u64 = 42;

    let mut cfg1 = channel_cfg(1);
    let mut cfg2 = channel_cfg(2);
    cfg1.output = RouteTarget::Bus(BUS_ID);
    cfg2.output = RouteTarget::Bus(BUS_ID);

    let bus_cfg = BusConfig {
        id: BUS_ID,
        name: "Vocal Bus".to_string(),
        gain_db: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        output: RouteTarget::Master,
        sends: vec![],
    };

    let sig = vec![0.5f32; frames * 2];
    let mut graph = MixerGraph::new(&[cfg1, cfg2], &[bus_cfg], 0.0, frames).unwrap();

    let output = render_with_inputs(&mut graph, vec![(1, sig.clone()), (2, sig.clone())], frames);

    // Both channels sum at bus (0.5+0.5 = 1.0 DC), then through master
    for &s in &output {
        let diff = (s - 1.0).abs();
        assert!(diff < 1e-4, "bus sum incorrect: got {s}");
    }
}

/// A routing cycle (A→B→A) must be rejected at graph construction.
#[test]
fn cycle_detection_rejects_a_to_b_to_a() {
    const BUS_A: u64 = 10;
    const BUS_B: u64 = 11;

    let bus_a = BusConfig {
        id: BUS_A,
        name: "Bus A".to_string(),
        gain_db: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        output: RouteTarget::Bus(BUS_B),
        sends: vec![],
    };
    let bus_b = BusConfig {
        id: BUS_B,
        name: "Bus B".to_string(),
        gain_db: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        output: RouteTarget::Bus(BUS_A), // cycle!
        sends: vec![],
    };

    let result = MixerGraph::new(&[], &[bus_a, bus_b], 0.0, 256);
    assert!(result.is_err(), "cycle must be rejected");
}

/// Self-route (a bus routing to itself) must be rejected.
#[test]
fn self_route_rejected() {
    const BUS_ID: u64 = 99;
    let bus = BusConfig {
        id: BUS_ID,
        name: "Self Bus".to_string(),
        gain_db: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        output: RouteTarget::Bus(BUS_ID), // self!
        sends: vec![],
    };
    let result = MixerGraph::new(&[], &[bus], 0.0, 256);
    assert!(result.is_err(), "self-route must be rejected");
}

/// `render_offline` is deterministic: identical inputs produce identical outputs.
#[test]
fn offline_render_is_deterministic() {
    let frames = 512;
    let cfgs = [channel_cfg(1), channel_cfg(2)];
    let sig = sine_burst(frames);

    let mut graph_a = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();
    let mut graph_b = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();

    let out_a = render_with_inputs(
        &mut graph_a,
        vec![(1, sig.clone()), (2, sig.clone())],
        frames,
    );
    let out_b = render_with_inputs(
        &mut graph_b,
        vec![(1, sig.clone()), (2, sig.clone())],
        frames,
    );

    assert_eq!(out_a, out_b, "render_offline is not deterministic");
}

/// Peak meter reads the correct maximum absolute sample value.
#[test]
fn peak_meter_reads_correctly() {
    let frames = 256;
    let cfgs = [channel_cfg(1)];
    // DC signal at exactly 0.7
    let sig = vec![0.7f32; frames * 2];
    let mut graph = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();
    let meters = graph.meters();

    let _ = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    let reading = meters.channels[&1].read();
    let peak = reading.peak_left.max(reading.peak_right);
    assert!(
        (peak - 0.7).abs() < 0.01,
        "peak meter incorrect: got {peak}"
    );
}

/// RMS meter reads a sensible value for a full-scale sine.
#[test]
fn rms_meter_reads_correctly_for_sine() {
    let frames = 1024; // enough cycles for stable RMS
    let cfgs = [channel_cfg(1)];
    // Full-scale sine at amplitude 1.0 — theoretical RMS = 1/√2 ≈ 0.707
    let sig: Vec<f32> = (0..frames * 2)
        .map(|i| {
            let frame = i / 2;
            (2.0 * std::f32::consts::PI * 440.0 * frame as f32 / 48_000.0).sin()
        })
        .collect();
    let mut graph = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();
    let meters = graph.meters();

    let _ = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    let reading = meters.channels[&1].read();
    let rms = reading.rms_left.max(reading.rms_right);
    // Expect within 10% of theoretical 0.707
    assert!(
        (rms - 0.707).abs() < 0.08,
        "RMS meter inaccurate: got {rms}, expected ≈ 0.707"
    );
}

/// Clipping is detected when samples exceed 0 dBFS.
#[test]
fn clip_indicator_set_on_overload() {
    let frames = 64;
    let cfgs = [channel_cfg(1)];
    // Signal clipping at 2.0 (well above 1.0 full-scale)
    let sig = vec![2.0f32; frames * 2];
    let mut graph = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();
    let meters = graph.meters();

    let _ = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    let reading = meters.channels[&1].read();
    assert!(
        reading.clipped,
        "clip indicator should be set for 2.0 signal"
    );
}

/// `clear_clip` resets the clip indicator.
#[test]
fn clear_clip_resets_indicator() {
    let frames = 64;
    let cfgs = [channel_cfg(1)];
    let sig = vec![2.0f32; frames * 2];
    let mut graph = MixerGraph::new(&cfgs, &[], 0.0, frames).unwrap();
    let meters = graph.meters();

    let _ = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    // Confirm clip first
    assert!(meters.channels[&1].read().clipped);

    // Clear and confirm reset
    meters.channels[&1].clear_clip();
    assert!(
        !meters.channels[&1].read().clipped,
        "clear_clip did not reset clip flag"
    );
}

/// Send route: channel sends a copy to an aux bus while routing primary output to master.
#[test]
fn send_route_delivers_to_aux_bus() {
    let frames = 256;
    const AUX_ID: u64 = 20;

    let mut cfg = channel_cfg(1);
    cfg.sends = vec![SendRoute {
        target_bus_id: AUX_ID,
        gain_db: 0.0,
        pre_fader: false,
    }];
    // Primary output still goes to master
    cfg.output = RouteTarget::Master;

    let aux = BusConfig {
        id: AUX_ID,
        name: "Reverb Send".to_string(),
        gain_db: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        output: RouteTarget::Master,
        sends: vec![],
    };

    let sig = vec![0.5f32; frames * 2];
    let mut graph = MixerGraph::new(&[cfg], &[aux], 0.0, frames).unwrap();
    let meters = graph.meters();

    let _ = render_with_inputs(&mut graph, vec![(1, sig)], frames);

    // The aux bus meter should show activity from the send
    let aux_reading = meters
        .buses
        .get(&AUX_ID)
        .expect("aux bus meter missing")
        .read();
    let aux_peak = aux_reading.peak_left.max(aux_reading.peak_right);
    assert!(
        aux_peak > 0.1,
        "send route did not deliver signal to aux bus: peak={aux_peak}"
    );
}

// ── Gain utility unit tests ───────────────────────────────────────────────────

#[test]
fn db_to_linear_round_trips() {
    for db in [-60.0f32, -20.0, -6.0, 0.0, 6.0] {
        let lin = db_to_linear(db);
        let back = linear_to_db(lin);
        assert!((back - db).abs() < 0.001, "round-trip failed for {db} dB");
    }
}

#[test]
fn db_to_linear_known_values() {
    assert!(
        (db_to_linear(0.0) - 1.0).abs() < 1e-6,
        "0 dB should be 1.0 linear"
    );
    assert!(
        (db_to_linear(-6.0206) - 0.5).abs() < 1e-3,
        "-6 dBFS should be ~0.5"
    );
}
