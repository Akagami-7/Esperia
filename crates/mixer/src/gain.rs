//! Decibel and linear gain conversions for the Esperia mixer.

/// Gain values at or below this dB threshold are treated as exact digital silence.
pub const SILENCE_THRESHOLD_DB: f32 = -96.0;
/// Default minimum fader dB.
pub const MIN_GAIN_DB: f32 = -96.0;
/// Default maximum fader dB (+12 dB headroom).
pub const MAX_GAIN_DB: f32 = 12.0;

/// Convert a decibel gain value to a linear amplitude multiplier.
/// Values `<= -96.0 dB` map to `0.0`.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    if !db.is_finite() || db <= SILENCE_THRESHOLD_DB {
        0.0
    } else {
        10.0f32.powf(db.clamp(MIN_GAIN_DB, MAX_GAIN_DB) / 20.0)
    }
}

/// Convert a linear amplitude multiplier to decibels.
/// Values `<= 0.0` return `SILENCE_THRESHOLD_DB`.
#[inline]
pub fn linear_to_db(linear: f32) -> f32 {
    if !linear.is_finite() || linear <= 0.0 {
        SILENCE_THRESHOLD_DB
    } else {
        (20.0 * linear.log10()).max(SILENCE_THRESHOLD_DB)
    }
}

/// Channel or bus gain parameter storing both dB (UI) and cached linear (DSP) values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gain {
    db: f32,
    linear: f32,
}

impl Gain {
    pub fn unity() -> Self {
        Self {
            db: 0.0,
            linear: 1.0,
        }
    }

    pub fn from_db(db: f32) -> Self {
        let clamped = if db.is_finite() {
            db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)
        } else {
            0.0
        };
        Self {
            db: clamped,
            linear: db_to_linear(clamped),
        }
    }

    #[inline]
    pub fn set_db(&mut self, db: f32) {
        *self = Self::from_db(db);
    }

    #[inline]
    pub fn db(&self) -> f32 {
        self.db
    }

    #[inline]
    pub fn linear(&self) -> f32 {
        self.linear
    }
}

impl Default for Gain {
    fn default() -> Self {
        Self::unity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_and_silence_conversions() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
        assert_eq!(db_to_linear(-96.0), 0.0);
        assert_eq!(db_to_linear(-120.0), 0.0);
        assert!((db_to_linear(-6.0206) - 0.5).abs() < 1e-3);
        assert!((linear_to_db(1.0) - 0.0).abs() < 1e-6);
    }
}
