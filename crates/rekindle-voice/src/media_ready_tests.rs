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

#[test]
fn update_emits_exactly_once_per_change() {
    let mut t = MediaReadyTracker::new();
    // First update always emits (seed → some reason).
    let first = t.update("c", "ch", |i| i.local_caps_reported = true);
    assert_eq!(
        first,
        Some(MediaReadyTransition {
            ready: false,
            reason: "handshake-announced".into()
        })
    );
    // Same derived state → no emit, even though inputs changed.
    assert!(t
        .update("c", "ch", |i| i.session_config_emitted = true)
        .is_none());
    // Walk to ready: each step that changes the REASON emits.
    assert!(t
        .update("c", "ch", |i| i.handshake = JoinHandshake::Seen)
        .is_some());
    assert!(t
        .update("c", "ch", |i| i.handshake = JoinHandshake::Connected)
        .is_some());
    let ready = t.update("c", "ch", |i| i.roster_non_empty = true);
    assert_eq!(
        ready,
        Some(MediaReadyTransition {
            ready: true,
            reason: "ready".into()
        })
    );
    assert!(t.is_ready("c", "ch"));
    // Re-applying the same input → no spurious re-emit.
    assert!(t.update("c", "ch", |i| i.roster_non_empty = true).is_none());
}

#[test]
fn regression_re_emits_when_input_lost() {
    let mut t = MediaReadyTracker::new();
    t.update("c", "ch", |i| {
        *i = inputs(JoinHandshake::Connected, 7);
    });
    assert!(t.is_ready("c", "ch"));
    // Peer leaves → roster empties → must emit not-ready.
    let down = t.update("c", "ch", |i| i.roster_non_empty = false);
    assert_eq!(
        down,
        Some(MediaReadyTransition {
            ready: false,
            reason: "roster-empty".into()
        })
    );
}

#[test]
fn clear_emits_only_when_previously_ready_or_unemitted() {
    let mut t = MediaReadyTracker::new();
    // Clearing an unknown slot: nothing.
    assert!(t.clear("c", "ch").is_none());
    // Slot that last emitted not-ready: clear is silent.
    t.update("c", "ch", |i| i.local_caps_reported = true);
    assert!(t.clear("c", "ch").is_none());
    // Slot that was ready: clear emits left-channel.
    t.update("c", "ch", |i| {
        *i = inputs(JoinHandshake::Connected, 7);
    });
    assert_eq!(
        t.clear("c", "ch"),
        Some(MediaReadyTransition {
            ready: false,
            reason: "left-channel".into()
        })
    );
    assert_eq!(t.state("c", "ch"), (false, "not-in-voice".to_string()));
}
