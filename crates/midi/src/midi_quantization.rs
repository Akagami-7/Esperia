//! Rhythmic quantization for MIDI notes (straight divisions and triplets).

use crate::midi_clip::MidiNote;
use crate::musical_time::PPQN;
use serde::{Deserialize, Serialize};

/// Standard musical grid divisions for snapping and quantization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum QuantizeGrid {
    Whole,
    Half,
    Quarter,
    Eighth,
    #[default]
    Sixteenth,
    ThirtySecond,
    QuarterTriplet,
    EighthTriplet,
    SixteenthTriplet,
}

impl QuantizeGrid {
    pub const ALL: [Self; 9] = [
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::ThirtySecond,
        Self::QuarterTriplet,
        Self::EighthTriplet,
        Self::SixteenthTriplet,
    ];

    /// Grid step size in integer ticks at `PPQN = 960`.
    #[inline]
    pub const fn ticks(self) -> u64 {
        match self {
            Self::Whole => PPQN * 4,                // 3840
            Self::Half => PPQN * 2,                 // 1920
            Self::Quarter => PPQN,                  // 960
            Self::Eighth => PPQN / 2,               // 480
            Self::Sixteenth => PPQN / 4,            // 240
            Self::ThirtySecond => PPQN / 8,         // 120
            Self::QuarterTriplet => (PPQN * 2) / 3, // 640
            Self::EighthTriplet => PPQN / 3,        // 320
            Self::SixteenthTriplet => PPQN / 6,     // 160
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Whole => "1/1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
            Self::ThirtySecond => "1/32",
            Self::QuarterTriplet => "1/4T",
            Self::EighthTriplet => "1/8T",
            Self::SixteenthTriplet => "1/16T",
        }
    }
}

/// Snap a tick position toward the nearest `grid` line with `strength` in `0.0..=1.0`.
#[inline]
pub fn quantize_tick(tick: u64, grid: QuantizeGrid, strength: f32) -> u64 {
    let step = grid.ticks().max(1);
    let s = strength.clamp(0.0, 1.0) as f64;
    let rem = tick % step;
    let nearest = if rem * 2 >= step {
        tick.saturating_add(step - rem)
    } else {
        tick - rem
    };
    let delta = nearest as f64 - tick as f64;
    (tick as f64 + delta * s).round().max(0.0) as u64
}

/// Quantize a slice of [`MidiNote`]s in place.
///
/// When `quantize_length` is `true`, note durations are also snapped to the nearest
/// non-zero grid multiple.
pub fn quantize_notes(
    notes: &mut [MidiNote],
    grid: QuantizeGrid,
    strength: f32,
    quantize_length: bool,
) {
    let step = grid.ticks().max(1);
    for note in notes {
        note.start_tick = quantize_tick(note.start_tick, grid, strength);
        if quantize_length {
            let snapped_dur = quantize_tick(note.duration_ticks, grid, strength).max(step);
            note.duration_ticks = snapped_dur;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_ticks_are_exact_integers() {
        assert_eq!(QuantizeGrid::Quarter.ticks(), 960);
        assert_eq!(QuantizeGrid::Eighth.ticks(), 480);
        assert_eq!(QuantizeGrid::Sixteenth.ticks(), 240);
        assert_eq!(QuantizeGrid::EighthTriplet.ticks(), 320);
        assert_eq!(QuantizeGrid::SixteenthTriplet.ticks(), 160);
    }

    #[test]
    fn quantize_snaps_to_nearest_sixteenth() {
        assert_eq!(quantize_tick(230, QuantizeGrid::Sixteenth, 1.0), 240);
        assert_eq!(quantize_tick(250, QuantizeGrid::Sixteenth, 1.0), 240);
        assert_eq!(quantize_tick(120, QuantizeGrid::Sixteenth, 1.0), 240);
        assert_eq!(quantize_tick(119, QuantizeGrid::Sixteenth, 1.0), 0);
    }

    #[test]
    fn partial_strength_interpolates() {
        // 200 toward 240 at 50% strength = 220
        assert_eq!(quantize_tick(200, QuantizeGrid::Sixteenth, 0.5), 220);
    }
}
