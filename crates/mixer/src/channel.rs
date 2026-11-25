//! Channel configuration and input descriptor.

use crate::routing::{RouteTarget, SendRoute};

/// Configuration for a mixer input channel (track strip).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelConfig {
    pub id: u64,
    pub name: String,
    pub gain_db: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub output: RouteTarget,
    pub sends: Vec<SendRoute>,
}

impl ChannelConfig {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            gain_db: 0.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            output: RouteTarget::Master,
            sends: Vec::new(),
        }
    }
}
