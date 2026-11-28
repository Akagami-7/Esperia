/// Parameters for a critically-damped or underdamped 1D spring.
#[derive(Clone, Copy, Debug)]
pub struct SpringParams {
    /// Spring stiffness (higher = snappier response).
    pub stiffness: f32,
    /// Viscous damping coefficient (higher = less overshoot).
    pub damping: f32,
}

impl Default for SpringParams {
    fn default() -> Self {
        Self {
            stiffness: 420.0,
            damping: 28.0,
        }
    }
}

/// 1D spring-damper state used by tactile controls and elastic gliders.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpringNode {
    pub position: f32,
    pub velocity: f32,
}

impl SpringNode {
    pub fn at(position: f32) -> Self {
        Self {
            position,
            velocity: 0.0,
        }
    }

    /// Advance the spring by `dt` seconds toward `target`.
    pub fn step(&mut self, target: f32, dt: f32, params: SpringParams) {
        let accel = (target - self.position) * params.stiffness - self.velocity * params.damping;
        self.velocity += accel * dt;
        self.position += self.velocity * dt;
    }

    /// Returns `true` when the node has settled near `target`.
    pub fn is_settled(&self, target: f32, epsilon: f32) -> bool {
        (target - self.position).abs() < epsilon && self.velocity.abs() < epsilon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_to_target() {
        let mut node = SpringNode::at(0.0);
        let params = SpringParams::default();
        for _ in 0..240 {
            node.step(100.0, 1.0 / 120.0, params);
        }
        assert!(node.is_settled(100.0, 0.05));
    }
}
