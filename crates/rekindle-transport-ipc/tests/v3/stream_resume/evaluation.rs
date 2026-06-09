use std::time::{Duration, Instant};
use rekindle_transport_ipc::v3::stream::resume::{
    ResumeState, ResumeRegistry, evaluate_resume_request, ResumeDecision, ResumeDenialReason,
};

fn setup_registry_with_state(
    transfer_id: uuid::Uuid,
    audit_link: [u8; 32],
    content_hash: [u8; 32],
    chunk: u32,
    byte_offset: u64,
) -> ResumeRegistry {
    let mut reg = ResumeRegistry::new();
    reg.register(ResumeState::new(
        transfer_id,
        byte_offset,
        chunk,
        audit_link,
        content_hash,
        Instant::now(),
        Duration::from_secs(300),
    ));
    reg
}

fn id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}

#[test]
fn valid_resume_accepted() {
    let tid = id(1);
    let audit_link = [0xAA; 32];
    let content_hash = [0xBB; 32];
    let reg = setup_registry_with_state(tid, audit_link, content_hash, 10, 10_485_760);

    let decision = evaluate_resume_request(
        &reg, tid, audit_link, content_hash,
    );
    match decision {
        ResumeDecision::Accepted { resume_from_chunk } => {
            assert_eq!(resume_from_chunk, 10);
        }
        other => panic!("Expected Accepted, got {other:?}"),
    }
}

#[test]
fn unknown_transfer_id_denied() {
    let reg = ResumeRegistry::new();
    let decision = evaluate_resume_request(
        &reg, id(999), [0xAA; 32], [0xBB; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::TransferIdUnknown)),
        "Expected TransferIdUnknown, got {decision:?}"
    );
}

#[test]
fn expired_window_denied() {
    let tid = id(1);
    let mut reg = ResumeRegistry::new();
    reg.register(ResumeState::new(
        tid, 0, 0, [0xAA; 32], [0xBB; 32],
        Instant::now() - Duration::from_secs(301),
        Duration::from_secs(300),
    ));

    let decision = evaluate_resume_request(
        &reg, tid, [0xAA; 32], [0xBB; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::ResumeWindowExpired)),
        "Expected ResumeWindowExpired, got {decision:?}"
    );
}

#[test]
fn audit_anchor_mismatch_denied() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    let decision = evaluate_resume_request(
        &reg, tid, [0xFF; 32], [0xBB; 32], // wrong audit link
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::AuditAnchorMismatch)),
        "Expected AuditAnchorMismatch, got {decision:?}"
    );
}

#[test]
fn content_anchor_mismatch_denied() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    let decision = evaluate_resume_request(
        &reg, tid, [0xAA; 32], [0xFF; 32], // wrong content hash
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::ContentAnchorMismatch)),
        "Expected ContentAnchorMismatch, got {decision:?}"
    );
}

#[test]
fn zero_byte_offset_resume_is_full_restart() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 0, 0);

    let decision = evaluate_resume_request(
        &reg, tid, [0xAA; 32], [0xBB; 32],
    );
    match decision {
        ResumeDecision::Accepted { resume_from_chunk } => {
            assert_eq!(resume_from_chunk, 0);
        }
        other => panic!("Expected Accepted at chunk 0, got {other:?}"),
    }
}

#[test]
fn resume_from_middle_of_transfer() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 50, 50 * 16_777_216);

    let decision = evaluate_resume_request(
        &reg, tid, [0xAA; 32], [0xBB; 32],
    );
    match decision {
        ResumeDecision::Accepted { resume_from_chunk } => {
            assert_eq!(resume_from_chunk, 50);
        }
        other => panic!("Expected Accepted at chunk 50, got {other:?}"),
    }
}

#[test]
fn resume_at_last_chunk() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 99, 99 * 16_777_216);

    let decision = evaluate_resume_request(
        &reg, tid, [0xAA; 32], [0xBB; 32],
    );
    match decision {
        ResumeDecision::Accepted { resume_from_chunk } => {
            assert_eq!(resume_from_chunk, 99);
        }
        other => panic!("Expected Accepted at chunk 99, got {other:?}"),
    }
}

#[test]
fn window_checked_before_anchors() {
    let tid = id(1);
    let mut reg = ResumeRegistry::new();
    reg.register(ResumeState::new(
        tid, 0, 0, [0xAA; 32], [0xBB; 32],
        Instant::now() - Duration::from_secs(301),
        Duration::from_secs(300),
    ));

    // Both anchors wrong AND expired — should get ResumeWindowExpired, not anchor mismatch
    let decision = evaluate_resume_request(
        &reg, tid, [0xFF; 32], [0xFF; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::ResumeWindowExpired)),
        "Window must be checked before anchors. Got {decision:?}"
    );
}

#[test]
fn audit_link_verified_before_content_hash() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    // Both anchors wrong — should get AuditAnchorMismatch (checked first)
    let decision = evaluate_resume_request(
        &reg, tid, [0xFF; 32], [0xFF; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::AuditAnchorMismatch)),
        "Audit link must be checked before content hash. Got {decision:?}"
    );
}

#[test]
fn resume_preserves_registry_entry() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    let decision = evaluate_resume_request(
        &reg, tid, [0xAA; 32], [0xBB; 32],
    );
    assert!(matches!(decision, ResumeDecision::Accepted { .. }));

    // Entry still in registry — caller removes after STREAM_FIN/ACK
    assert!(reg.lookup(tid).is_some());
}

// ── Adversarial tests ─────────────────────────────────────────────

#[test]
fn concurrent_resume_for_same_transfer_both_see_same_state() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    let d1 = evaluate_resume_request(&reg, tid, [0xAA; 32], [0xBB; 32]);
    let d2 = evaluate_resume_request(&reg, tid, [0xAA; 32], [0xBB; 32]);

    // Both see the same state — both accepted. The application layer
    // must serialize actual stream creation to prevent double-resume.
    assert!(matches!(d1, ResumeDecision::Accepted { resume_from_chunk: 10 }));
    assert!(matches!(d2, ResumeDecision::Accepted { resume_from_chunk: 10 }));
}

#[test]
fn resume_after_gc_returns_unknown() {
    let tid = id(1);
    let mut reg = ResumeRegistry::new();
    reg.register(ResumeState::new(
        tid, 0, 0, [0xAA; 32], [0xBB; 32],
        Instant::now() - Duration::from_secs(301),
        Duration::from_secs(300),
    ));
    reg.gc_expired();

    let decision = evaluate_resume_request(&reg, tid, [0xAA; 32], [0xBB; 32]);
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::TransferIdUnknown)),
        "After gc, transfer_id must be unknown. Got {decision:?}"
    );
}

#[test]
fn valid_audit_link_from_wrong_chain_position_denied() {
    let tid = id(1);
    // Registry has audit_link [0xAA], attacker sends a valid-looking but
    // different link [0x11] — this is a link from a different chain position
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    let decision = evaluate_resume_request(
        &reg, tid, [0x11; 32], [0xBB; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::AuditAnchorMismatch)),
        "Wrong-position audit link must be rejected. Got {decision:?}"
    );
}

#[test]
fn all_zero_anchors_do_not_bypass_check() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    let decision = evaluate_resume_request(
        &reg, tid, [0x00; 32], [0x00; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(_)),
        "All-zero anchors must not bypass verification. Got {decision:?}"
    );
}

#[test]
fn resume_with_swapped_anchors_denied() {
    let tid = id(1);
    let reg = setup_registry_with_state(tid, [0xAA; 32], [0xBB; 32], 10, 0);

    // Swap: send content_hash as audit_link and vice versa
    let decision = evaluate_resume_request(
        &reg, tid, [0xBB; 32], [0xAA; 32],
    );
    assert!(
        matches!(decision, ResumeDecision::Denied(ResumeDenialReason::AuditAnchorMismatch)),
        "Swapped anchors must be rejected at audit check. Got {decision:?}"
    );
}
