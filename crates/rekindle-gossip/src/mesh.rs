//! Gossip mesh peer tracking helpers and fan-out selection rules.

// `GossipMesh<T>` lived here and was never used — not by this crate, not
// re-exported from its `lib.rs`, not imported anywhere. It is a leftover
// from plan item 4.8, which converged the gossip *primitives*
// (`fanout_degree`, `DEFAULT_TTL`, `LamportClock`, `TokenBucket`) into
// this crate and left the struct behind. The live mesh is
// `rekindle_transport::gossip::GossipMesh`, which carries
// `community_id`, the clock and the rate limiter rather than a bare
// peer map, and which imports this crate's functions.

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
