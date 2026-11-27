//! Signal routing targets, cycle detection, and topological evaluation order.

use std::collections::{HashMap, VecDeque};

/// Destination for a channel strip or bus output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteTarget {
    /// Route into a group/aux bus by ID.
    Bus(u64),
    /// Route directly into the master bus.
    Master,
}

/// An auxiliary send from a channel or bus into another bus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SendRoute {
    pub target_bus_id: u64,
    pub gain_db: f32,
    pub pre_fader: bool,
}

/// Errors produced when validating or updating the mixer routing graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    /// A routing cycle (feedback loop) was detected involving a bus.
    CycleDetected,
    /// A channel or bus targets a bus ID that does not exist.
    UnknownBus(u64),
    /// A bus attempted to route or send to itself.
    SelfRoute(u64),
    /// The audio callback buffer exceeded the mixer's preallocated capacity.
    BufferTooLarge {
        requested_frames: usize,
        max_frames: usize,
    },
    /// A project graph exceeded a fixed upper bound for callback work/storage.
    ResourceLimit {
        resource: &'static str,
        actual: usize,
        max: usize,
    },
}

impl std::fmt::Display for RoutingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CycleDetected => write!(f, "routing cycle detected in mixer graph"),
            Self::UnknownBus(id) => write!(f, "unknown target bus id: {id}"),
            Self::SelfRoute(id) => write!(f, "bus {id} cannot route to itself"),
            Self::BufferTooLarge {
                requested_frames,
                max_frames,
            } => write!(
                f,
                "audio buffer ({requested_frames} frames) exceeds mixer capacity ({max_frames})"
            ),
            Self::ResourceLimit {
                resource,
                actual,
                max,
            } => write!(f, "mixer {resource} limit exceeded ({actual} > {max})"),
        }
    }
}

impl std::error::Error for RoutingError {}

/// Validate bus-to-bus routing and compute a cycle-free topological order
/// (upstream buses first, downstream buses last).
pub fn topological_bus_order(
    bus_ids: &[u64],
    bus_edges: &[(u64, RouteTarget, &[SendRoute])],
) -> Result<Vec<usize>, RoutingError> {
    let mut id_to_idx: HashMap<u64, usize> = HashMap::with_capacity(bus_ids.len());
    for (idx, &id) in bus_ids.iter().enumerate() {
        id_to_idx.insert(id, idx);
    }

    let n = bus_ids.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<usize> = vec![0; n];

    for (src_id, output, sends) in bus_edges {
        let Some(&src_idx) = id_to_idx.get(src_id) else {
            continue;
        };

        if let RouteTarget::Bus(dst_id) = *output {
            if dst_id == *src_id {
                return Err(RoutingError::SelfRoute(*src_id));
            }
            let Some(&dst_idx) = id_to_idx.get(&dst_id) else {
                return Err(RoutingError::UnknownBus(dst_id));
            };
            adj[src_idx].push(dst_idx);
            in_degree[dst_idx] += 1;
        }

        for send in *sends {
            if send.target_bus_id == *src_id {
                return Err(RoutingError::SelfRoute(*src_id));
            }
            let Some(&dst_idx) = id_to_idx.get(&send.target_bus_id) else {
                return Err(RoutingError::UnknownBus(send.target_bus_id));
            };
            adj[src_idx].push(dst_idx);
            in_degree[dst_idx] += 1;
        }
    }

    let mut queue: VecDeque<usize> = VecDeque::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            queue.push_back(idx);
        }
    }

    let mut order = Vec::with_capacity(n);
    while let Some(u) = queue.pop_front() {
        order.push(u);
        for &v in &adj[u] {
            in_degree[v] -= 1;
            if in_degree[v] == 0 {
                queue.push_back(v);
            }
        }
    }

    if order.len() != n {
        return Err(RoutingError::CycleDetected);
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_routing_cycle() {
        let ids = [1, 2];
        let s1 = [SendRoute {
            target_bus_id: 2,
            gain_db: 0.0,
            pre_fader: false,
        }];
        let s2 = [SendRoute {
            target_bus_id: 1,
            gain_db: 0.0,
            pre_fader: false,
        }];
        let edges = [
            (1, RouteTarget::Master, s1.as_slice()),
            (2, RouteTarget::Master, s2.as_slice()),
        ];
        assert_eq!(
            topological_bus_order(&ids, &edges),
            Err(RoutingError::CycleDetected)
        );
    }

    #[test]
    fn orders_upstream_bus_before_downstream_bus() {
        let ids = [10, 20];
        let edges = [
            (10, RouteTarget::Bus(20), [].as_slice()),
            (20, RouteTarget::Master, [].as_slice()),
        ];
        let order = topological_bus_order(&ids, &edges).unwrap();
        assert_eq!(order, vec![0, 1]);
    }
}
