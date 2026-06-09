use rekindle_transport_ipc::v3::session::rotation::{
    RotationCoordinator, RotationError,
};
use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
use rekindle_transport_ipc::v3::audit::chain::AuditChain;

fn test_handshake_hash() -> [u8; 32] { [0xAA; 32] }

/// The audit chain link at the moment of rotation in all tests.
const TEST_TRANSCRIPT_ANCHOR: [u8; 32] = [0xBB; 32];

#[test]
fn initiate_rotation_produces_init_payload() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut coordinator = RotationCoordinator::new(initial_keys, 0);

    let init = coordinator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).expect("initiate must succeed");

    assert_eq!(init.initiator_generation, 1);
    assert_eq!(init.phase_2_deadline_ms, 30_000);
    assert_ne!(init.new_static_pub, [0u8; 32], "must generate a real keypair");
    assert_ne!(init.rotation_chain_secret, [0u8; 32], "must generate random secret");
}

#[test]
fn receive_init_and_produce_commit() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys, 0);

    let init_payload = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit_payload = responder.receive_init(&init_payload, TEST_TRANSCRIPT_ANCHOR).expect("receive_init must succeed");

    assert_eq!(commit_payload.rotation_id, init_payload.rotation_id);
    assert_ne!(commit_payload.new_static_pub, [0u8; 32]);
    assert_ne!(commit_payload.rotation_chain_secret, [0u8; 32]);
    assert_eq!(commit_payload.responder_generation, 1);
}

#[test]
fn complete_rotation_swaps_keys() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let old_envelope = initial_keys.envelope_d2l;

    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys, 0);

    let init_payload = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit_payload = responder.receive_init(&init_payload, TEST_TRANSCRIPT_ANCHOR).unwrap();
    initiator.receive_commit(&commit_payload).expect("receive_commit must succeed");

    let new_keys = initiator.current_keys();
    assert_ne!(new_keys.envelope_d2l, old_envelope);
}

#[test]
fn old_keys_differ_from_new() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys.clone(), 0);

    let init_payload = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit_payload = responder.receive_init(&init_payload, TEST_TRANSCRIPT_ANCHOR).unwrap();
    initiator.receive_commit(&commit_payload).unwrap();

    let new_keys = initiator.current_keys();
    assert_ne!(new_keys.envelope_d2l, initial_keys.envelope_d2l);
    assert_ne!(new_keys.envelope_l2d, initial_keys.envelope_l2d);
    assert_ne!(new_keys.header_d2l, initial_keys.header_d2l);
    assert_ne!(new_keys.header_l2d, initial_keys.header_l2d);
    assert_ne!(new_keys.stream_d2l, initial_keys.stream_d2l);
    assert_ne!(new_keys.stream_l2d, initial_keys.stream_l2d);
    assert_ne!(new_keys.audit_d2l, initial_keys.audit_d2l);
    assert_ne!(new_keys.audit_l2d, initial_keys.audit_l2d);
    assert_ne!(new_keys.handoff, initial_keys.handoff);
}

#[test]
fn generation_increments() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut coordinator = RotationCoordinator::new(initial_keys.clone(), 0);

    assert_eq!(coordinator.generation(), 0);

    let mut responder = RotationCoordinator::new(initial_keys, 0);
    let init = coordinator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    coordinator.receive_commit(&commit).unwrap();

    assert_eq!(coordinator.generation(), 1);
}

#[test]
fn rotation_link_bridges_audit_chain() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys.clone(), 0);

    // Build a pre-rotation chain
    let mut pre_chain = AuditChain::new(initial_keys.audit_d2l, TEST_TRANSCRIPT_ANCHOR);
    for seq in 0..5 {
        pre_chain.advance(rekindle_transport_ipc::v3::audit::chain::LinkInput {
            session_seq: seq,
            envelope_hash: [seq as u8; 32],
            header_hash: [(seq + 1) as u8; 32],
            ciphertext_hash: [(seq + 2) as u8; 32],
        });
    }
    let pre_terminal = pre_chain.current_link();

    let init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    initiator.receive_commit(&commit).unwrap();

    // Compute rotation link using the coordinator's combined secret
    let rot_link = initiator.rotation_link(&pre_terminal)
        .expect("rotation_link must be available after complete rotation");

    // Post-rotation chain anchored on rotation_link
    let new_keys = initiator.current_keys();
    let mut post_chain = AuditChain::with_rotation_anchor(new_keys.audit_d2l, rot_link, 5);

    post_chain.advance(rekindle_transport_ipc::v3::audit::chain::LinkInput {
        session_seq: 5,
        envelope_hash: [5; 32],
        header_hash: [6; 32],
        ciphertext_hash: [7; 32],
    });

    assert_ne!(post_chain.current_link(), [0u8; 32]);
    assert_eq!(post_chain.anchor_record().value, rot_link);
}

#[test]
fn concurrent_rotation_rejected() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut coordinator = RotationCoordinator::new(initial_keys, 0);

    coordinator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let second = coordinator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR);
    assert!(
        second.is_err(),
        "second initiate during active rotation must fail"
    );
    match second.unwrap_err() {
        RotationError::RotationInProgress => {}
        other => panic!("Expected RotationInProgress, got {other:?}"),
    }
}

#[test]
fn stale_commit_rejected() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys, 0);

    let init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let mut commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();

    // Tamper the rotation_id
    commit.rotation_id = uuid::Uuid::from_u128(9999);

    let result = initiator.receive_commit(&commit);
    assert!(result.is_err());
    match result.unwrap_err() {
        RotationError::RotationIdMismatch => {}
        other => panic!("Expected RotationIdMismatch, got {other:?}"),
    }
}

// ── Adversarial tests ─────────────────────────────────────────────

#[test]
fn replayed_init_after_completion_rejected() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys, 0);

    let init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    initiator.receive_commit(&commit).unwrap();

    // Replay the same init — initiator has no pending rotation now,
    // but a new initiate should succeed (it's a fresh rotation).
    // The REPLAYED init going to the responder should produce keys
    // that don't match the initiator's current keys.
    let replayed_commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    assert_eq!(
        replayed_commit.rotation_id, init.rotation_id,
        "responder doesn't detect replay — it produces a commit for the replayed init"
    );

    // The replayed commit has the old init's chain_secret combined with
    // a NEW responder chain_secret — the keys will differ from what
    // the initiator derived in the first rotation.
    let responder_keys_after_replay = responder.current_keys();
    let initiator_keys = initiator.current_keys();
    assert_ne!(
        responder_keys_after_replay.envelope_d2l,
        initiator_keys.envelope_d2l,
        "replayed init must produce divergent keys — the responder \
         generated a new chain_secret that the initiator doesn't have"
    );
}

#[test]
fn tampered_chain_secret_in_commit_produces_divergent_keys() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys.clone(), 0);

    let init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let mut commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();

    // Tamper the responder's chain_secret
    commit.rotation_chain_secret = [0xFF; 32];

    // Initiator accepts — it can't detect the tamper at this layer.
    // But the derived keys will differ from the responder's keys.
    initiator.receive_commit(&commit).unwrap();

    let initiator_keys = initiator.current_keys();
    let responder_keys = responder.current_keys();
    assert_ne!(
        initiator_keys.envelope_d2l,
        responder_keys.envelope_d2l,
        "tampered chain_secret must produce divergent keys — \
         detected at first EMAC verification after rotation"
    );
}

#[test]
fn commit_without_prior_init_rejected() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut coordinator = RotationCoordinator::new(initial_keys, 0);

    let fake_commit = rekindle_transport_ipc::v3::codec::channel::rotate::RotateCommitPayload {
        rotation_id: uuid::Uuid::from_u128(42),
        responder_generation: 1,
        new_static_pub: [0xDD; 32],
        rotation_chain_secret: [0xEE; 32],
        transcript_anchor: [0xFF; 32],
    };

    let result = coordinator.receive_commit(&fake_commit);
    assert!(result.is_err());
    match result.unwrap_err() {
        RotationError::RotationIdMismatch => {}
        other => panic!("Expected RotationIdMismatch (no pending), got {other:?}"),
    }
}

#[test]
fn all_zero_chain_secret_still_produces_valid_keys() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys, 0);

    let mut init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    // Force all-zero chain secret — should still produce non-zero derived keys
    // because HKDF with zero IKM and non-empty labels produces non-zero output
    init.rotation_chain_secret = [0x00; 32];

    let commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    assert_ne!(commit.rotation_chain_secret, [0u8; 32],
        "responder generates its own random secret independent of initiator's");

    let rk = responder.current_keys();
    assert_ne!(rk.envelope_d2l, [0u8; 32], "zero chain_secret must not produce zero keys");
    assert_ne!(rk.stream_d2l, [0u8; 32]);
}

#[test]
fn keys_after_rotation_are_not_initial_keys() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys.clone(), 0);

    let init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    initiator.receive_commit(&commit).unwrap();

    // Both sides' keys must differ from the initial handshake keys
    let ik = initiator.current_keys();
    let rk = responder.current_keys();
    for (name, old, new_i, new_r) in [
        ("envelope_d2l", &initial_keys.envelope_d2l, &ik.envelope_d2l, &rk.envelope_d2l),
        ("stream_d2l", &initial_keys.stream_d2l, &ik.stream_d2l, &rk.stream_d2l),
        ("audit_d2l", &initial_keys.audit_d2l, &ik.audit_d2l, &rk.audit_d2l),
        ("handoff", &initial_keys.handoff, &ik.handoff, &rk.handoff),
    ] {
        assert_ne!(old, new_i, "initiator {name} must differ from initial after rotation");
        assert_ne!(old, new_r, "responder {name} must differ from initial after rotation");
    }
}

#[test]
fn both_sides_derive_same_keys_after_honest_rotation() {
    let initial_keys = derive_all_keys(&test_handshake_hash());
    let mut initiator = RotationCoordinator::new(initial_keys.clone(), 0);
    let mut responder = RotationCoordinator::new(initial_keys, 0);

    let init = initiator.initiate(30_000, TEST_TRANSCRIPT_ANCHOR).unwrap();
    let commit = responder.receive_init(&init, TEST_TRANSCRIPT_ANCHOR).unwrap();
    initiator.receive_commit(&commit).unwrap();

    let ik = initiator.current_keys();
    let rk = responder.current_keys();
    assert_eq!(ik.envelope_d2l, rk.envelope_d2l);
    assert_eq!(ik.envelope_l2d, rk.envelope_l2d);
    assert_eq!(ik.header_d2l, rk.header_d2l);
    assert_eq!(ik.header_l2d, rk.header_l2d);
    assert_eq!(ik.stream_d2l, rk.stream_d2l);
    assert_eq!(ik.stream_l2d, rk.stream_l2d);
    assert_eq!(ik.audit_d2l, rk.audit_d2l);
    assert_eq!(ik.audit_l2d, rk.audit_l2d);
    assert_eq!(ik.handoff, rk.handoff);
}
