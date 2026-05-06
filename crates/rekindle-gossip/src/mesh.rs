//! Gossip mesh peer tracking helpers and fan-out selection rules.

use std::collections::HashMap;

/// Minimal mesh state that tracks selected peers and the broader online set.
#[derive(Debug, Clone, Default)]
pub struct GossipMesh<T> {
    pub peers: HashMap<String, T>,
    pub online_members: HashMap<String, T>,
    pub needs_initial_sync: bool,
}

impl<T> GossipMesh<T> {
    /// Create an empty mesh in the initial-sync state.
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            online_members: HashMap::new(),
            needs_initial_sync: true,
        }
    }
}

/// Compute the mesh fan-out degree for the current online population.
///
/// Architecture §3 line 315: `D = min(N, 6)` for `N ≤ 20`, `6` for
/// `21..=60`, `8` for `61+`. The dedup cache + 5-hop TTL guarantee
/// delivery without flooding even when most peers don't receive a
/// direct copy.
pub fn fanout_degree(online_count: usize) -> usize {
    match online_count {
        0 => 0,
        1..=20 => online_count.min(6),
        21..=60 => 6,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::fanout_degree;

    #[test]
    fn fanout_degree_matches_architecture_ranges() {
        assert_eq!(fanout_degree(0), 0);
        assert_eq!(fanout_degree(1), 1);
        assert_eq!(fanout_degree(5), 5);
        assert_eq!(fanout_degree(6), 6);
        assert_eq!(fanout_degree(7), 6);
        assert_eq!(fanout_degree(20), 6);
        assert_eq!(fanout_degree(21), 6);
        assert_eq!(fanout_degree(60), 6);
        assert_eq!(fanout_degree(61), 8);
    }
}
