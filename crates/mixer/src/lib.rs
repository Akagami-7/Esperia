//! `esperia-mixer` — professional multitrack mixing & signal routing engine.
//!
//! # Architecture rules
//! * Zero external dependencies (`std` only).
//! * No heap allocations, mutexes, or I/O in `MixerGraph::process()`.
//! * Routing cycles are rejected deterministically at graph construction time.

pub mod bus;
pub mod channel;
pub mod channel_strip;
pub mod gain;
pub mod master;
pub mod meter;
pub mod panning;
pub mod routing;

pub use bus::{Bus, BusConfig};
pub use channel::ChannelConfig;
pub use channel_strip::ChannelStrip;
pub use gain::{Gain, MAX_GAIN_DB, MIN_GAIN_DB, db_to_linear, linear_to_db};
pub use master::MasterChannel;
pub use meter::{AtomicMeter, MeterReading};
pub use panning::StereoPan;
pub use routing::{RouteTarget, RoutingError, SendRoute, topological_bus_order};

use std::collections::HashMap;
use std::sync::Arc;

/// Maximum graph dimensions accepted by the real-time mixer.
pub const MAX_CHANNELS: usize = 4096;
pub const MAX_BUSES: usize = 512;
pub const MAX_SENDS: usize = 8192;
pub const MAX_BUFFER_FRAMES: usize = 16_384;

/// Snapshot of all meters in the mixer graph, cloneable for UI readout.
#[derive(Clone, Default)]
pub struct MixerMeters {
    pub channels: HashMap<u64, Arc<AtomicMeter>>,
    pub buses: HashMap<u64, Arc<AtomicMeter>>,
    pub master: Arc<AtomicMeter>,
}

/// Complete validated mixing & routing graph with preallocated buffers.
///
/// Built on the UI/control thread and evaluated lock-free on the audio thread
/// (or offline for deterministic rendering).
pub struct MixerGraph {
    channels: Vec<ChannelStrip>,
    channel_index: HashMap<u64, usize>,
    buses: Vec<Bus>,
    bus_index: HashMap<u64, usize>,
    bus_order: Vec<usize>,
    master: MasterChannel,
    max_frames: usize,
}

impl MixerGraph {
    /// Construct and validate a new `MixerGraph`.
    ///
    /// Rejects routing cycles (`RoutingError::CycleDetected`), self-routes, and
    /// references to unknown buses.
    pub fn new(
        channel_configs: &[ChannelConfig],
        bus_configs: &[BusConfig],
        master_gain_db: f32,
        max_frames: usize,
    ) -> Result<Self, RoutingError> {
        if channel_configs.len() > MAX_CHANNELS {
            return Err(RoutingError::ResourceLimit {
                resource: "channel",
                actual: channel_configs.len(),
                max: MAX_CHANNELS,
            });
        }
        if bus_configs.len() > MAX_BUSES {
            return Err(RoutingError::ResourceLimit {
                resource: "bus",
                actual: bus_configs.len(),
                max: MAX_BUSES,
            });
        }
        let send_count = channel_configs
            .iter()
            .map(|channel| channel.sends.len())
            .chain(bus_configs.iter().map(|bus| bus.sends.len()))
            .sum::<usize>();
        if send_count > MAX_SENDS {
            return Err(RoutingError::ResourceLimit {
                resource: "send",
                actual: send_count,
                max: MAX_SENDS,
            });
        }
        if max_frames > MAX_BUFFER_FRAMES {
            return Err(RoutingError::ResourceLimit {
                resource: "buffer frame",
                actual: max_frames,
                max: MAX_BUFFER_FRAMES,
            });
        }

        let cap = max_frames.max(64);
        let mut bus_ids = Vec::with_capacity(bus_configs.len());
        let mut bus_index = HashMap::with_capacity(bus_configs.len());
        let mut buses = Vec::with_capacity(bus_configs.len());

        for (idx, cfg) in bus_configs.iter().enumerate() {
            bus_ids.push(cfg.id);
            bus_index.insert(cfg.id, idx);
            buses.push(Bus::from_config(cfg, cap));
        }

        // Validate channel targets & sends against known buses.
        for ch in channel_configs {
            if let RouteTarget::Bus(bid) = ch.output
                && !bus_index.contains_key(&bid)
            {
                return Err(RoutingError::UnknownBus(bid));
            }
            for send in &ch.sends {
                if !bus_index.contains_key(&send.target_bus_id) {
                    return Err(RoutingError::UnknownBus(send.target_bus_id));
                }
            }
        }

        let bus_edges: Vec<(u64, RouteTarget, &[SendRoute])> = bus_configs
            .iter()
            .map(|b| (b.id, b.output, b.sends.as_slice()))
            .collect();
        let bus_order = topological_bus_order(&bus_ids, &bus_edges)?;

        let mut channel_index = HashMap::with_capacity(channel_configs.len());
        let mut channels = Vec::with_capacity(channel_configs.len());
        for (idx, cfg) in channel_configs.iter().enumerate() {
            channel_index.insert(cfg.id, idx);
            channels.push(ChannelStrip::from_config(cfg, cap));
        }

        let master = MasterChannel::new(master_gain_db, cap);

        Ok(Self {
            channels,
            channel_index,
            buses,
            bus_index,
            bus_order,
            master,
            max_frames: cap,
        })
    }

    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Return atomic meter handles for all channels, buses, and master.
    pub fn meters(&self) -> MixerMeters {
        let channels = self
            .channels
            .iter()
            .map(|c| (c.id, Arc::clone(&c.meter)))
            .collect();
        let buses = self
            .buses
            .iter()
            .map(|b| (b.id, Arc::clone(&b.meter)))
            .collect();
        MixerMeters {
            channels,
            buses,
            master: Arc::clone(&self.master.meter),
        }
    }

    /// Clear all preallocated channel, bus, and master buffers for `frames` frames.
    /// Real-time safe: zero allocations.
    #[inline]
    pub fn clear_buffers(&mut self, frames: usize) {
        let f = frames.min(self.max_frames);
        for ch in &mut self.channels {
            ch.clear(f);
        }
        for bus in &mut self.buses {
            bus.clear(f);
        }
        self.master.clear(f);
    }

    /// Mutable stereo input buffer for channel `channel_id` (if present).
    /// Real-time safe: zero allocations.
    #[inline]
    pub fn channel_input_mut(&mut self, channel_id: u64, frames: usize) -> Option<&mut [f32]> {
        let idx = *self.channel_index.get(&channel_id)?;
        Some(self.channels[idx].input_buffer_mut(frames.min(self.max_frames)))
    }

    /// Fallback mutable stereo input buffer for the first channel (if any),
    /// or direct master accumulation when no channels exist.
    #[inline]
    pub fn accumulate_direct_master(&mut self, stereo_samples: &[f32], frames: usize) {
        self.master
            .accumulate(stereo_samples, frames.min(self.max_frames));
    }

    /// Evaluate the full mixing & routing graph for `frames` frames and write
    /// the final mix into `output` (`out_channels` interleaved).
    ///
    /// # Real-time safety
    /// Zero heap allocations, zero mutexes, zero I/O.
    pub fn process(&mut self, frames: usize, out_channels: usize, output: &mut [f32]) {
        let f = frames.min(self.max_frames);
        let any_channel_solo = self.channels.iter().any(|c| c.soloed);
        let any_bus_solo = self.buses.iter().any(|b| b.soloed);
        let any_solo = any_channel_solo || any_bus_solo;

        // 1. Evaluate channel strips and route pre/post-fader sends + main output.
        for ch_idx in 0..self.channels.len() {
            let ch_solo = self.channels[ch_idx].soloed;
            let ch_muted = self.channels[ch_idx].muted;
            let audible = if any_solo {
                ch_solo && !ch_muted
            } else {
                !ch_muted
            };

            // Pre-fader sends (only if channel is not muted).
            if !ch_muted {
                let send_count = self.channels[ch_idx].sends.len();
                for s_idx in 0..send_count {
                    let send = self.channels[ch_idx].sends[s_idx];
                    if send.pre_fader
                        && let Some(&bus_idx) = self.bus_index.get(&send.target_bus_id)
                    {
                        let scale = db_to_linear(send.gain_db);
                        let src = self.channels[ch_idx].buffer(f);
                        self.buses[bus_idx].accumulate_scaled(src, f, scale);
                    }
                }
            }

            self.channels[ch_idx].process_in_place(f, audible);

            // Post-fader sends + main output routing.
            let send_count = self.channels[ch_idx].sends.len();
            for s_idx in 0..send_count {
                let send = self.channels[ch_idx].sends[s_idx];
                if !send.pre_fader
                    && let Some(&bus_idx) = self.bus_index.get(&send.target_bus_id)
                {
                    let scale = db_to_linear(send.gain_db);
                    let src = self.channels[ch_idx].buffer(f);
                    self.buses[bus_idx].accumulate_scaled(src, f, scale);
                }
            }

            match self.channels[ch_idx].output {
                RouteTarget::Bus(bus_id) => {
                    if let Some(&bus_idx) = self.bus_index.get(&bus_id) {
                        let src = self.channels[ch_idx].buffer(f);
                        self.buses[bus_idx].accumulate_scaled(src, f, 1.0);
                    }
                }
                RouteTarget::Master => {
                    let src = self.channels[ch_idx].buffer(f);
                    self.master.accumulate(src, f);
                }
            }
        }

        // 2. Evaluate buses in cycle-free topological order.
        for order_i in 0..self.bus_order.len() {
            let bus_idx = self.bus_order[order_i];
            let bus_muted = self.buses[bus_idx].muted;
            let bus_solo = self.buses[bus_idx].soloed;
            // A bus is audible if not muted, and if bus-level solo is active it
            // must be soloed (channel-level solo already muted non-soloed inputs).
            let audible = if any_bus_solo {
                bus_solo && !bus_muted
            } else {
                !bus_muted
            };

            self.buses[bus_idx].process_in_place(f, audible);

            // Bus sends (to downstream buses only — guaranteed acyclic).
            let send_count = self.buses[bus_idx].sends.len();
            for s_idx in 0..send_count {
                let send = self.buses[bus_idx].sends[s_idx];
                if let Some(&dst_idx) = self.bus_index.get(&send.target_bus_id) {
                    let scale = db_to_linear(send.gain_db);
                    // Copy via split_at_mut or index since dst_idx != bus_idx.
                    if dst_idx != bus_idx {
                        let (src_slice, dst_bus) =
                            borrow_two_buses(&mut self.buses, bus_idx, dst_idx);
                        dst_bus.accumulate_scaled(src_slice.buffer(f), f, scale);
                    }
                }
            }

            match self.buses[bus_idx].output {
                RouteTarget::Bus(dst_id) => {
                    if let Some(&dst_idx) = self.bus_index.get(&dst_id)
                        && dst_idx != bus_idx
                    {
                        let (src_bus, dst_bus) =
                            borrow_two_buses(&mut self.buses, bus_idx, dst_idx);
                        dst_bus.accumulate_scaled(src_bus.buffer(f), f, 1.0);
                    }
                }
                RouteTarget::Master => {
                    self.master.accumulate(self.buses[bus_idx].buffer(f), f);
                }
            }
        }

        // 3. Evaluate master output.
        self.master.process_to_output(f, out_channels, output);
    }

    /// Deterministic offline rendering helper: renders `total_frames` in
    /// fixed `block_frames` chunks using a caller-supplied per-block input filler.
    pub fn render_offline<F>(
        &mut self,
        total_frames: usize,
        block_frames: usize,
        out_channels: usize,
        mut fill_inputs: F,
    ) -> Result<Vec<f32>, RoutingError>
    where
        F: FnMut(&mut Self, usize, usize),
    {
        let blk = block_frames.max(1);
        if blk > self.max_frames {
            return Err(RoutingError::BufferTooLarge {
                requested_frames: blk,
                max_frames: self.max_frames,
            });
        }
        let ch = out_channels.max(1);
        let mut out = vec![0.0f32; total_frames * ch];
        let mut cursor = 0usize;

        while cursor < total_frames {
            let n = (total_frames - cursor).min(blk);
            self.clear_buffers(n);
            fill_inputs(self, cursor, n);
            let slice = &mut out[cursor * ch..(cursor + n) * ch];
            self.process(n, ch, slice);
            cursor += n;
        }

        Ok(out)
    }
}

#[inline]
fn borrow_two_buses(buses: &mut [Bus], a: usize, b: usize) -> (&Bus, &mut Bus) {
    if a < b {
        let (left, right) = buses.split_at_mut(b);
        (&left[a], &mut right[0])
    } else {
        let (left, right) = buses.split_at_mut(a);
        (&right[0], &mut left[b])
    }
}
