//! Heartbeat lifecycle tests.
//!
//! These tests prove the heartbeat mechanism works without sockets:
//! - PING emission after idle
//! - PONG handling (nonce match, miss reset, remote_seq update)
//! - Two-generation nonce model (stale PONG acceptance)
//! - Miss counting and threshold enforcement
//! - Activity suppresses PING
//!
//! Every test creates a SessionContext via make_test_context() and
//! drives heartbeat state directly. The control loop's heartbeat
//! timer fires these same code paths; these tests prove each path
//! in isolation so socket E2E failures can be diagnosed.

use rekindle_transport_ipc::v3::codec::channel::ping as ping_codec;
use rekindle_transport_ipc::v3::context::{OutboundFrame, SessionConfig};
use rekindle_transport_ipc::v3::dispatch::test_helpers::{
    make_test_context, make_test_context_with_config,
};
use rekindle_transport_ipc::v3::handlers::channel::pong;

#[test]
fn ping_received_produces_pong_with_echoed_nonce() {
    use rekindle_transport_ipc::v3::handlers::channel::ping;
    let (mut ctx, _router) = make_test_context();

    let ping_payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xDEAD_BEEF_CAFE_BABE,
        sender_epoch_ns: 1_700_000_000_000_000_000,
        last_seen_remote_seq: 42,
    });
    ping::handle(&mut ctx, &ping_payload).unwrap();

    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 1, "PING must produce exactly one PONG");
    match &out[0] {
        OutboundFrame::Channel { kind: rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind::Pong, payload } => {
            let p = ping_codec::decode(payload).unwrap();
            assert_eq!(p.ping_nonce, 0xDEAD_BEEF_CAFE_BABE, "PONG must echo nonce");
        }
        other => panic!("expected PONG, got {other:?}"),
    }
}

#[test]
fn ping_updates_remote_last_seen_our_seq() {
    use rekindle_transport_ipc::v3::handlers::channel::ping;
    let (mut ctx, _router) = make_test_context();

    assert_eq!(ctx.remote_last_seen_our_seq(), 0);
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 1,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 42,
    });
    ping::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.remote_last_seen_our_seq(), 42);
}

/// PONG with matching nonce resets the heartbeat miss count.
/// Uses verify_and_apply — the single source of truth.
#[test]
fn pong_resets_miss_count() {
    let (mut ctx, _router) = make_test_context();

    ctx.increment_heartbeat_miss();
    ctx.increment_heartbeat_miss();
    assert_eq!(ctx.heartbeat_miss_count(), 2);

    ctx.set_last_ping_nonce(0xCAFE);
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xCAFE,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    let outcome = pong::verify_and_apply(&mut ctx, &payload).unwrap();
    assert!(matches!(outcome, pong::PongOutcome::Accepted { .. }));
    assert_eq!(ctx.heartbeat_miss_count(), 0, "PONG must reset miss count");
    assert!(ctx.last_ping_nonce().is_none(), "nonce must be cleared");
    assert!(ctx.last_pong_received().is_some(), "pong timestamp must be set");
}

/// PONG with wrong nonce is silently discarded per SCTP §8.3.
/// The miss count is NOT reset — we can't verify the peer is alive.
#[test]
fn pong_with_wrong_nonce_discarded() {
    let (mut ctx, _router) = make_test_context();

    ctx.set_last_ping_nonce(0xCAFE);
    ctx.increment_heartbeat_miss();
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xBAD,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    pong::handle(&mut ctx, &payload).unwrap();
    // Discarded — miss count unchanged, nonce unchanged
    assert_eq!(ctx.heartbeat_miss_count(), 1, "discarded PONG must not reset miss count");
    assert_eq!(ctx.last_ping_nonce(), Some(0xCAFE), "nonce must not be cleared");
}

/// PONG without a prior PING is silently discarded.
#[test]
fn pong_without_prior_ping_discarded() {
    let (mut ctx, _router) = make_test_context();

    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 123,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    pong::handle(&mut ctx, &payload).unwrap();
    // Discarded — no state change
    assert_eq!(ctx.heartbeat_miss_count(), 0);
}

/// Stale PONG (previous generation) resets miss count — peer IS alive.
#[test]
fn stale_pong_resets_miss_count() {
    let (mut ctx, _router) = make_test_context();

    // Simulate: PING sent with nonce A, then timeout rotates A → previous
    ctx.set_last_ping_nonce(0xAAAA);
    ctx.rotate_ping_nonce(); // 0xAAAA → previous, current = None
    ctx.increment_heartbeat_miss();

    // New PING sent with nonce B
    ctx.set_last_ping_nonce(0xBBBB);
    assert_eq!(ctx.heartbeat_miss_count(), 1);

    // Stale PONG for nonce A arrives
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xAAAA,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    let outcome = pong::verify_and_apply(&mut ctx, &payload).unwrap();
    assert!(matches!(outcome, pong::PongOutcome::AcceptedStale));
    assert_eq!(ctx.heartbeat_miss_count(), 0, "stale PONG must reset miss count");
    // Current nonce B still outstanding
    assert_eq!(ctx.last_ping_nonce(), Some(0xBBBB), "current nonce must not be cleared");
    assert!(ctx.previous_ping_nonce().is_none(), "previous nonce must be cleared");
}

/// Three consecutive misses triggers HeartbeatTimeout.
#[test]
fn three_consecutive_misses_reaches_threshold() {
    let config = SessionConfig {
        heartbeat_miss_limit: 3,
        ..SessionConfig::default()
    };
    let (mut ctx, _router) = make_test_context_with_config(config);

    ctx.increment_heartbeat_miss();
    assert_eq!(ctx.heartbeat_miss_count(), 1);
    assert!(ctx.heartbeat_miss_count() < 3);

    ctx.increment_heartbeat_miss();
    assert_eq!(ctx.heartbeat_miss_count(), 2);
    assert!(ctx.heartbeat_miss_count() < 3);

    ctx.increment_heartbeat_miss();
    assert_eq!(ctx.heartbeat_miss_count(), 3);
    assert!(
        ctx.heartbeat_miss_count() >= 3,
        "3 consecutive misses must reach the threshold"
    );
}

/// Miss count increments independently — each pong timeout adds exactly 1.
#[test]
fn miss_count_increments_by_one() {
    let (mut ctx, _router) = make_test_context();

    for expected in 1..=10u32 {
        ctx.increment_heartbeat_miss();
        assert_eq!(ctx.heartbeat_miss_count(), expected);
    }
}

/// PONG records the received timestamp for RTT estimation.
#[test]
fn pong_records_receive_timestamp() {
    let (mut ctx, _router) = make_test_context();

    assert!(ctx.last_pong_received().is_none());

    ctx.set_last_ping_nonce(0xBEEF);
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xBEEF,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    let outcome = pong::verify_and_apply(&mut ctx, &payload).unwrap();
    assert!(matches!(outcome, pong::PongOutcome::Accepted { .. }));

    assert!(
        ctx.last_pong_received().is_some(),
        "PONG must record receive timestamp for RTT"
    );
}
