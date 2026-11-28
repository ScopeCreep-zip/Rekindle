use super::*;

const ALL_HANDSHAKES: [JoinHandshake; 3] = [
    JoinHandshake::Announced,
    JoinHandshake::Seen,
    JoinHandshake::Connected,
];

/// Build inputs from a 3-bit flag mask: bit0 = roster, bit1 = caps,
/// bit2 = config (7 = all set).
fn inputs(handshake: JoinHandshake, bits: u8) -> MediaReadyInputs {
    MediaReadyInputs {
        handshake,
        roster_non_empty: bits & 1 != 0,
        local_caps_reported: bits & 2 != 0,
        session_config_emitted: bits & 4 != 0,
    }
}

/// Exhaustive truth table: 3 handshake states × 2³ flags = 24
/// combinations. `ready()` ⇔ Connected ∧ all three flags, and the
/// reason is "ready" exactly when ready.
#[test]
fn ready_iff_connected_and_all_flags() {
    for hs in ALL_HANDSHAKES {
        for bits in 0..8u8 {
            let i = inputs(hs, bits);
            let expect = hs == JoinHandshake::Connected && bits == 7;
            assert_eq!(i.ready(), expect, "inputs: {i:?}");
            assert_eq!(i.block_reason() == "ready", expect, "inputs: {i:?}");
        }
    }
}

/// Reason names the FIRST blocker in bring-up order.
#[test]
fn block_reason_priority_order() {
    assert_eq!(
        inputs(JoinHandshake::Announced, 7).block_reason(),
        "handshake-announced"
    );
    assert_eq!(
        inputs(JoinHandshake::Seen, 0).block_reason(),
        "handshake-seen"
    );
    assert_eq!(
        inputs(JoinHandshake::Connected, 0).block_reason(),
        "roster-empty"
    );
    assert_eq!(
        inputs(JoinHandshake::Connected, 1).block_reason(),
        "caps-unreported"
    );
    assert_eq!(
        inputs(JoinHandshake::Connected, 1 | 2).block_reason(),
        "config-pending"
    );
    assert_eq!(inputs(JoinHandshake::Connected, 7).block_reason(), "ready");
}
