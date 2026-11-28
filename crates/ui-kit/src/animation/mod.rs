//! Reusable animation, easing, and spring-physics primitives for Esperia UI controls.

mod spring;
mod transition;

pub use spring::{SpringNode, SpringParams};
pub use transition::{animate_towards, ease_in_out_cubic, ease_out_cubic, lerp_color};
