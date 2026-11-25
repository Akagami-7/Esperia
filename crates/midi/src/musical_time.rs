//! Musical time system: integer ticks (960 PPQN), time signatures, and tempo map.
//!
//! All musical note positions and durations are stored in integer ticks so tempo
//! edits never drift or corrupt rhythmic alignment.  The [`TempoMap`] converts
//! cleanly between musical ticks, wall-clock seconds, and audio sample frames.

use serde::{Deserialize, Serialize};

/// Pulses (ticks) per quarter note.
///
/// 960 is divisible by 2, 3, 4, 5, 6, 8, 12, 16, 24, 32, 48, 64, 96, 120, 160,
/// 192, 240, 320, 480, and 960 — so standard divisions and triplets are exact integers.
pub const PPQN: u64 = 960;

/// Musical time signature (e.g. 4/4, 3/4, 6/8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            numerator: 4,
            denominator: 4,
        }
    }
}

impl TimeSignature {
    /// Ticks per beat (where one beat is `4 / denominator` quarter notes).
    #[inline]
    pub fn ticks_per_beat(self) -> u64 {
        let denom = u64::from(self.denominator.max(1));
        (PPQN * 4) / denom
    }

    /// Ticks per bar/measure.
    #[inline]
    pub fn ticks_per_bar(self) -> u64 {
        u64::from(self.numerator.max(1)) * self.ticks_per_beat()
    }
}

/// A tempo change event at a specific musical tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TempoPoint {
    pub tick: u64,
    pub bpm: f64,
}

/// Piecewise-constant tempo map converting between musical ticks and audio samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TempoMap {
    points: Vec<TempoPoint>,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self::constant(120.0)
    }
}

impl TempoMap {
    /// Create a constant-tempo map starting at tick 0.
    pub fn constant(bpm: f64) -> Self {
        let clamped = if bpm.is_finite() && bpm > 0.0 {
            bpm.clamp(20.0, 400.0)
        } else {
            120.0
        };
        Self {
            points: vec![TempoPoint {
                tick: 0,
                bpm: clamped,
            }],
        }
    }

    /// Build a tempo map from arbitrary points, ensuring tick 0 exists and points are sorted.
    pub fn from_points(mut points: Vec<TempoPoint>) -> Self {
        points.retain(|p| p.bpm.is_finite() && p.bpm > 0.0);
        points.sort_by_key(|p| p.tick);
        // Deduplicate same-tick entries keeping the last one
        let mut deduped: Vec<TempoPoint> = Vec::with_capacity(points.len().max(1));
        for p in points {
            let clamped = TempoPoint {
                tick: p.tick,
                bpm: p.bpm.clamp(20.0, 400.0),
            };
            if let Some(last) = deduped.last_mut()
                && last.tick == clamped.tick
            {
                *last = clamped;
            } else {
                deduped.push(clamped);
            }
        }
        if deduped.first().is_none_or(|p| p.tick != 0) {
            let initial_bpm = deduped.first().map_or(120.0, |p| p.bpm);
            deduped.insert(
                0,
                TempoPoint {
                    tick: 0,
                    bpm: initial_bpm,
                },
            );
        }
        Self { points: deduped }
    }

    /// Return the sorted tempo points.
    pub fn points(&self) -> &[TempoPoint] {
        &self.points
    }

    /// Return the active BPM at `tick`.
    pub fn bpm_at_tick(&self, tick: u64) -> f64 {
        let mut active = self.points[0].bpm;
        for p in &self.points {
            if p.tick <= tick {
                active = p.bpm;
            } else {
                break;
            }
        }
        active
    }

    /// Convert a musical `tick` position into elapsed seconds from tick 0.
    pub fn tick_to_seconds(&self, tick: u64) -> f64 {
        let mut seconds = 0.0f64;
        for (idx, p) in self.points.iter().enumerate() {
            if p.tick >= tick {
                break;
            }
            let next_tick = self.points.get(idx + 1).map_or(tick, |n| n.tick.min(tick));
            let delta_ticks = next_tick.saturating_sub(p.tick) as f64;
            let secs_per_tick = 60.0 / (p.bpm * PPQN as f64);
            seconds += delta_ticks * secs_per_tick;
        }
        seconds
    }

    /// Convert elapsed `seconds` from tick 0 into the nearest musical tick.
    pub fn seconds_to_tick(&self, seconds: f64) -> u64 {
        if !seconds.is_finite() || seconds <= 0.0 {
            return 0;
        }
        let mut remaining = seconds;
        for (idx, p) in self.points.iter().enumerate() {
            let secs_per_tick = 60.0 / (p.bpm * PPQN as f64);
            if let Some(next) = self.points.get(idx + 1) {
                let seg_ticks = next.tick.saturating_sub(p.tick);
                let seg_secs = seg_ticks as f64 * secs_per_tick;
                if remaining <= seg_secs {
                    return p.tick + (remaining / secs_per_tick).round() as u64;
                }
                remaining -= seg_secs;
            } else {
                return p.tick + (remaining / secs_per_tick).round() as u64;
            }
        }
        0
    }

    /// Convert a musical `tick` into an audio sample frame at `sample_rate`.
    #[inline]
    pub fn tick_to_sample(&self, tick: u64, sample_rate: u32) -> u64 {
        let secs = self.tick_to_seconds(tick);
        (secs * f64::from(sample_rate.max(1))).round() as u64
    }

    /// Convert an audio sample frame at `sample_rate` into a musical tick.
    #[inline]
    pub fn sample_to_tick(&self, sample: u64, sample_rate: u32) -> u64 {
        let secs = sample as f64 / f64::from(sample_rate.max(1));
        self.seconds_to_tick(secs)
    }

    /// Convert `tick` into `(bar_1based, beat_1based, sub_tick)`.
    pub fn tick_to_bar_beat_tick(tick: u64, time_sig: TimeSignature) -> (u64, u64, u64) {
        let tpb_bar = time_sig.ticks_per_bar().max(1);
        let tpb_beat = time_sig.ticks_per_beat().max(1);
        let bar = tick / tpb_bar + 1;
        let rem = tick % tpb_bar;
        let beat = rem / tpb_beat + 1;
        let sub_tick = rem % tpb_beat;
        (bar, beat, sub_tick)
    }

    /// Convert `(bar_1based, beat_1based, sub_tick)` into an absolute musical tick.
    pub fn bar_beat_tick_to_tick(
        bar: u64,
        beat: u64,
        sub_tick: u64,
        time_sig: TimeSignature,
    ) -> u64 {
        let bar0 = bar.saturating_sub(1);
        let beat0 = beat.saturating_sub(1);
        bar0 * time_sig.ticks_per_bar() + beat0 * time_sig.ticks_per_beat() + sub_tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_120_bpm_quarter_note_is_half_second() {
        let map = TempoMap::constant(120.0);
        let secs = map.tick_to_seconds(PPQN);
        assert!((secs - 0.5).abs() < 1e-9);
        assert_eq!(map.tick_to_sample(PPQN, 48_000), 24_000);
        assert_eq!(map.sample_to_tick(24_000, 48_000), PPQN);
    }

    #[test]
    fn tempo_map_multi_segment_roundtrip() {
        // Bar 1 (4 beats = 3840 ticks) at 120 BPM (= 2.0 s), then 60 BPM (1 beat/s)
        let map = TempoMap::from_points(vec![
            TempoPoint {
                tick: 0,
                bpm: 120.0,
            },
            TempoPoint {
                tick: PPQN * 4,
                bpm: 60.0,
            },
        ]);
        // At tick 5*PPQN: 2.0s + 1.0s = 3.0s
        let secs = map.tick_to_seconds(PPQN * 5);
        assert!((secs - 3.0).abs() < 1e-6);
        assert_eq!(map.seconds_to_tick(3.0), PPQN * 5);
    }

    #[test]
    fn bar_beat_tick_roundtrip() {
        let ts = TimeSignature::default();
        let tick = TempoMap::bar_beat_tick_to_tick(3, 2, 240, ts);
        assert_eq!(TempoMap::tick_to_bar_beat_tick(tick, ts), (3, 2, 240));
    }
}
