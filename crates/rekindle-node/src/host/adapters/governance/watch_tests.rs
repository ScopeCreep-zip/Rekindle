//! The ring's rebuild trigger (plan E3.2 S7).

use rekindle_types::dht_layout::member_record::{FRONTIER, SUBKEY_COUNT};

use super::rebuilds_on;

/// A ring change on each subkey of `M` rebuilds only for the frontier
/// (`M`/3), the writer's commit point; the op-log head alone, a beat,
/// an outbox or any other subkey never does (register 2026-10-24,
/// "S8 sequencing"). `on_watched_change` itself is driven in the
/// netsim (`ring_frontier`).
#[test]
fn only_a_frontier_change_rebuilds() {
    for subkey in 0..u32::from(SUBKEY_COUNT) {
        assert_eq!(
            rebuilds_on(&[subkey]),
            subkey == FRONTIER,
            "a ring change on subkey {subkey}"
        );
    }
    assert!(
        rebuilds_on(&[2, 3]),
        "a change naming the frontier rebuilds"
    );
    assert!(
        !rebuilds_on(&[1, 2, 4]),
        "a head without its frontier does not"
    );
}
