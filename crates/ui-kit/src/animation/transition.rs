use egui::Color32;

/// Cubic ease-out (`0.0..=1.0` -> `0.0..=1.0`).
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Cubic ease-in-out (`0.0..=1.0` -> `0.0..=1.0`).
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Exponentially smoothed frame-rate-independent approach toward `target`.
pub fn animate_towards(current: f32, target: f32, dt: f32, duration_secs: f32) -> f32 {
    if duration_secs <= f32::EPSILON {
        return target;
    }
    let alpha = 1.0 - (-dt.max(0.0) / (duration_secs * 0.35)).exp();
    current + (target - current) * alpha.clamp(0.0, 1.0)
}

/// Linearly interpolate between two sRGB colors.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    Color32::from_rgba_unmultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_preserve_endpoints() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_out_cubic(0.0), 0.0);
        assert_eq!(ease_in_out_cubic(1.0), 1.0);
    }

    #[test]
    fn lerp_color_midpoints() {
        let c = lerp_color(Color32::BLACK, Color32::WHITE, 0.5);
        assert!((c.r() as i32 - 128).abs() <= 1);
    }
}
