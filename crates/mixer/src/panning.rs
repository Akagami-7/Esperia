//! Constant-power stereo panning.

use std::f32::consts::FRAC_PI_2;

/// Stereo pan law using constant-power (`cos` / `sin`) curves.
///
/// * `pan = -1.0` -> hard left (`L = 1.0, R = 0.0`)
/// * `pan =  0.0` -> center (`L ≈ 0.7071, R ≈ 0.7071`, `L² + R² = 1.0`)
/// * `pan = +1.0` -> hard right (`L = 0.0, R = 1.0`)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StereoPan {
    pan: f32,
    left_gain: f32,
    right_gain: f32,
}

impl StereoPan {
    pub fn center() -> Self {
        Self::from_pan(0.0)
    }

    pub fn from_pan(pan: f32) -> Self {
        let clamped = if pan.is_finite() {
            pan.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let norm = (clamped + 1.0) * 0.5; // [0, 1]
        let angle = norm * FRAC_PI_2;
        Self {
            pan: clamped,
            left_gain: angle.cos(),
            right_gain: angle.sin(),
        }
    }

    #[inline]
    pub fn pan(&self) -> f32 {
        self.pan
    }

    #[inline]
    pub fn gains(&self) -> (f32, f32) {
        (self.left_gain, self.right_gain)
    }
}

impl Default for StereoPan {
    fn default() -> Self {
        Self::center()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_power_preserves_energy() {
        for p in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
            let (l, r) = StereoPan::from_pan(p).gains();
            let power = l * l + r * r;
            assert!((power - 1.0).abs() < 1e-5, "pan={p} power={power}");
        }
    }

    #[test]
    fn hard_left_and_right_bounds() {
        let (ll, lr) = StereoPan::from_pan(-1.0).gains();
        assert!((ll - 1.0).abs() < 1e-6);
        assert!(lr.abs() < 1e-6);

        let (rl, rr) = StereoPan::from_pan(1.0).gains();
        assert!(rl.abs() < 1e-6);
        assert!((rr - 1.0).abs() < 1e-6);
    }
}
