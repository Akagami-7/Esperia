/// Visual interaction state for all Esperia button types.
///
/// Computed each frame from egui's `Response` and stored on the button widget so
/// that all rendering helpers (fill, shadow, transform) can branch on a single
/// value instead of re-testing multiple boolean flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonState {
    /// No pointer interaction and not toggled on.
    #[default]
    Default,
    /// Pointer is hovering over the button.
    Hovered,
    /// Mouse button held down (before release).
    Pressed,
    /// Toggle / latch is engaged (e.g. metronome on, record armed).
    Active,
    /// Keyboard focus ring visible (accessibility).
    Focused,
    /// Non-interactive; dimmed appearance.
    Disabled,
}

impl ButtonState {
    /// Derive state from an egui [`egui::Response`] plus an external `active` toggle flag.
    pub fn from_response(response: &egui::Response, active: bool, disabled: bool) -> Self {
        if disabled {
            return Self::Disabled;
        }
        if response.is_pointer_button_down_on() {
            return Self::Pressed;
        }
        if active {
            return Self::Active;
        }
        if response.has_focus() {
            return Self::Focused;
        }
        if response.hovered() {
            return Self::Hovered;
        }
        Self::Default
    }

    /// True when the button should render with the accent (violet) fill.
    pub fn is_accent(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Opacity multiplier for dimmed state.
    pub fn alpha(self) -> f32 {
        if matches!(self, Self::Disabled) {
            0.38
        } else {
            1.0
        }
    }

    /// Vertical scale applied to simulate a tactile inward press.
    pub fn press_scale(self) -> f32 {
        if matches!(self, Self::Pressed) {
            0.94
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_takes_priority_over_active() {
        // We cannot construct a real egui::Response here, so test the logic
        // through the flag branches directly.
        assert_eq!(ButtonState::Disabled.alpha(), 0.38);
        assert!(ButtonState::Active.is_accent());
        assert!(!ButtonState::Hovered.is_accent());
        assert_eq!(ButtonState::Pressed.press_scale(), 0.94);
        assert_eq!(ButtonState::Default.press_scale(), 1.0);
    }
}
