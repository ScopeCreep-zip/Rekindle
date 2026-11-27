use super::*;

#[test]
fn debug_redacts_secret_and_key() {
    let state = GroupCallState {
        call_id: "abc".into(),
        group_id: None,
        initiator_pubkey: "deadbeef".into(),
        kind: 0,
        participants: vec!["p1".into(), "p2".into()],
        accepted: HashSet::new(),
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAB; 32]),
        status: GroupCallStatus::Outgoing,
    };
    let dbg = format!("{state:?}");
    // Must NOT contain the actual key bytes or any hex pattern that
    // would suggest a 32-byte key leaked through.
    assert!(!dbg.contains("AB"));
    assert!(dbg.contains("has_call_key: true"));
    assert!(dbg.contains("has_secret: true"));
    assert!(dbg.contains("participants_count: 2"));
}

#[test]
fn status_variants_present() {
    let _ = GroupCallStatus::Outgoing;
    let _ = GroupCallStatus::Incoming;
    let _ = GroupCallStatus::Active;
    let _ = GroupCallStatus::Ended;
}
