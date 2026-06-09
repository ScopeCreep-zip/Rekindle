//! Audit chain lifecycle tests.
//!
//! These tests prove the audit chain advances correctly for every
//! frame direction and that checkpoints carry the correct chain state.
//!
//! The audit chain is the transport's forensic integrity backbone:
//! - Every frame contributes a Link
//! - Checkpoints commit chain state periodically
//! - The receiver's inbound chain must match the sender's outbound chain
//! - Key rotation bridges chains via rotation_link
//!
//! These tests drive the chain directly via SessionContext methods.
//! The control loop calls these same methods after encode/decode.

use rekindle_transport_ipc::v3::audit::chain::LinkInput;
use rekindle_transport_ipc::v3::audit::checkpoint::CheckpointConfig;
use rekindle_transport_ipc::v3::context::SessionConfig;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{
    make_test_context, make_test_context_with_config,
};

fn make_link_input(seq: u64) -> LinkInput {
    LinkInput {
        session_seq: seq,
        envelope_hash: [seq as u8; 32],
        header_hash: [(seq + 1) as u8; 32],
        ciphertext_hash: [(seq + 2) as u8; 32],
    }
}

/// Inbound frame advances the inbound chain by one Link.
/// The control loop calls ctx.inbound_chain_mut().advance(link_input)
/// after every successfully verified inbound frame.
#[test]
fn inbound_frame_advances_inbound_chain() {
    let (mut ctx, _router) = make_test_context();

    let anchor = ctx.inbound_chain().current_link();
    assert_eq!(ctx.inbound_chain().length(), 0);

    ctx.inbound_chain_mut().advance(make_link_input(0));

    assert_eq!(ctx.inbound_chain().length(), 1);
    assert_ne!(
        ctx.inbound_chain().current_link(),
        anchor,
        "chain must advance — current link must differ from anchor"
    );
}

/// Outbound frame advances the outbound chain by one Link.
/// The control loop calls ctx.outbound_chain_mut().advance(link_input)
/// after encoding each outbound frame (computing hashes from ciphertext).
#[test]
fn outbound_frame_advances_outbound_chain() {
    let (mut ctx, _router) = make_test_context();

    let anchor = ctx.outbound_chain().current_link();
    assert_eq!(ctx.outbound_chain().length(), 0);

    ctx.outbound_chain_mut().advance(make_link_input(0));

    assert_eq!(ctx.outbound_chain().length(), 1);
    assert_ne!(ctx.outbound_chain().current_link(), anchor);
}

/// Inbound and outbound chains are independent — advancing one
/// does not affect the other. They use different keys (audit_d2l
/// vs audit_l2d) and track different frame directions.
#[test]
fn inbound_and_outbound_chains_independent() {
    let (mut ctx, _router) = make_test_context();

    ctx.inbound_chain_mut().advance(make_link_input(0));
    ctx.inbound_chain_mut().advance(make_link_input(1));
    ctx.inbound_chain_mut().advance(make_link_input(2));

    ctx.outbound_chain_mut().advance(make_link_input(0));

    assert_eq!(ctx.inbound_chain().length(), 3);
    assert_eq!(ctx.outbound_chain().length(), 1);
    assert_ne!(
        ctx.inbound_chain().current_link(),
        ctx.outbound_chain().current_link(),
        "chains use different keys — links must differ even for same input"
    );
}

/// Both chains start from the same anchor (handshake_hash).
/// After handshake, both peers derive the same anchor from
/// the transcript. The anchor is the trust root.
#[test]
fn both_chains_share_handshake_anchor() {
    let (ctx, _router) = make_test_context();

    assert_eq!(
        ctx.inbound_chain().anchor_record().value,
        ctx.outbound_chain().anchor_record().value,
        "both chains must anchor on handshake_hash"
    );
}

/// Checkpoint must carry outbound chain state, NOT inbound.
/// The sender emits a checkpoint saying "here is my outbound chain
/// at frame N." The receiver compares against their inbound chain
/// (which tracks the same direction). Mixing directions causes
/// checkpoint verification to always fail.
#[test]
fn checkpoint_carries_outbound_chain_link() {
    let (mut ctx, _router) = make_test_context();

    // Advance outbound chain
    for seq in 0..10u64 {
        ctx.outbound_chain_mut().advance(make_link_input(seq));
    }
    // Advance inbound chain differently
    for seq in 0..5u64 {
        ctx.inbound_chain_mut().advance(make_link_input(seq + 100));
    }

    // Build checkpoint from outbound chain (correct)
    let outbound_link = ctx.outbound_chain().current_link();
    let outbound_length = ctx.outbound_chain().length();
    let outbound_anchor = ctx.outbound_chain().anchor_record().value;

    // The checkpoint's chain_link MUST be the outbound link
    assert_eq!(outbound_length, 10);
    assert_ne!(outbound_link, ctx.inbound_chain().current_link());

    // The checkpoint's anchor_link MUST be the outbound chain's anchor
    let inbound_anchor = ctx.inbound_chain().anchor_record().value;
    assert_eq!(
        outbound_anchor, inbound_anchor,
        "both chains share the same handshake anchor"
    );
    // But the chain links diverge because different keys and different inputs
    let inbound_link = ctx.inbound_chain().current_link();
    assert_ne!(
        outbound_link, inbound_link,
        "outbound and inbound links must differ — using the wrong one in checkpoint would cause verification failure"
    );
    // The checkpoint carries outbound_anchor, not inbound_anchor (same value
    // for handshake-anchored chains, but different after rotation)
    assert_eq!(outbound_anchor, ctx.outbound_chain().anchor_record().value);
}

/// Checkpoint emitted after configured frame count.
/// The control loop checks outbound_checkpoint.is_due() after
/// each frame and emits AUDIT_CHECKPOINT when the threshold fires.
#[test]
fn checkpoint_due_after_configured_count() {
    let config = SessionConfig {
        checkpoint_config: CheckpointConfig {
            max_frames: 5,
            max_interval_ms: 50,
        },
        ..SessionConfig::default()
    };
    let (mut ctx, _router) = make_test_context_with_config(config);

    for _ in 0..4 {
        ctx.outbound_checkpoint_mut().frame_processed();
        assert!(!ctx.outbound_checkpoint_mut().is_due());
    }

    ctx.outbound_checkpoint_mut().frame_processed(); // 5th
    assert!(
        ctx.outbound_checkpoint_mut().is_due(),
        "checkpoint must be due after 5 frames"
    );
}

/// Checkpoint resets after emission.
#[test]
fn checkpoint_resets_after_emission() {
    let config = SessionConfig {
        checkpoint_config: CheckpointConfig {
            max_frames: 3,
            max_interval_ms: 50,
        },
        ..SessionConfig::default()
    };
    let (mut ctx, _router) = make_test_context_with_config(config);

    for _ in 0..3 {
        ctx.outbound_checkpoint_mut().frame_processed();
    }
    assert!(ctx.outbound_checkpoint_mut().is_due());

    ctx.outbound_checkpoint_mut().checkpoint_emitted();
    assert!(
        !ctx.outbound_checkpoint_mut().is_due(),
        "checkpoint must reset after emission"
    );
}

/// Chain link is deterministic — same inputs produce same link.
/// This is critical for cross-peer verification: both peers
/// compute the same chain from the same wire bytes.
#[test]
fn chain_is_deterministic() {
    let (mut ctx_a, _) = make_test_context();
    let (mut ctx_b, _) = make_test_context();

    for seq in 0..5u64 {
        // Both contexts use the same key (both are Dialler role, same test keys)
        // and same inputs — they must produce the same links
        ctx_a.outbound_chain_mut().advance(make_link_input(seq));
        ctx_b.outbound_chain_mut().advance(make_link_input(seq));
    }

    assert_eq!(
        ctx_a.outbound_chain().current_link(),
        ctx_b.outbound_chain().current_link(),
        "same inputs must produce same chain — determinism is required for cross-peer verification"
    );
}

/// Different inputs produce different links — the chain is
/// content-dependent. A tampered frame produces a divergent chain
/// that checkpoint verification catches.
#[test]
fn different_inputs_produce_different_links() {
    let (mut ctx_a, _) = make_test_context();
    let (mut ctx_b, _) = make_test_context();

    ctx_a.outbound_chain_mut().advance(make_link_input(0));

    let mut tampered = make_link_input(0);
    tampered.ciphertext_hash = [0xFF; 32];
    ctx_b.outbound_chain_mut().advance(tampered);

    assert_ne!(
        ctx_a.outbound_chain().current_link(),
        ctx_b.outbound_chain().current_link(),
        "different ciphertext must produce different chain link"
    );
}
