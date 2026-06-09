use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::ping;
use rekindle_transport_ipc::v3::handlers::channel::pong;
use rekindle_transport_ipc::v3::codec::channel::ping as ping_codec;

#[test]
fn ping_produces_pong_in_outbound() {
    let (mut ctx, router) = make_test_context();
    let ping_payload = ping_codec::PingPayload {
        ping_nonce: 0xDEAD_BEEF,
        sender_epoch_ns: 1_700_000_000_000_000_000,
        last_seen_remote_seq: 0,
    };
    let encoded = ping_codec::encode(&ping_payload);
    ping::handle(&mut ctx, &encoded).expect("ping handler must succeed");

    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 1, "ping must produce exactly one outbound frame");
    match &out[0] {
        OutboundFrame::Channel { kind: rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind::Pong, payload } => {
            let pong = ping_codec::decode(payload).expect("pong must decode");
            assert_eq!(pong.ping_nonce, 0xDEAD_BEEF, "pong must echo ping nonce");
        }
        other => panic!("expected PONG, got {other:?}"),
    }
    assert_no_router_deliveries(&router);
}

#[test]
fn ping_updates_last_seen_remote_seq() {
    let (mut ctx, router) = make_test_context();
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 1,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 42,
    });
    ping::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.remote_last_seen_our_seq(), 42);
    assert_no_router_deliveries(&router);
}

/// PONG with matching nonce resets miss count via verify_and_apply.
#[test]
fn pong_resets_heartbeat_miss_count() {
    let (mut ctx, router) = make_test_context();
    ctx.increment_heartbeat_miss();
    ctx.increment_heartbeat_miss();
    assert_eq!(ctx.heartbeat_miss_count(), 2);

    ctx.set_last_ping_nonce(0xCAFE);

    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xCAFE,
        sender_epoch_ns: 1_700_000_000_000_000_000,
        last_seen_remote_seq: 0,
    });
    let outcome = pong::verify_and_apply(&mut ctx, &payload).expect("pong verify must succeed");
    assert!(matches!(outcome, pong::PongOutcome::Accepted { .. }));
    assert_eq!(ctx.heartbeat_miss_count(), 0, "pong must reset miss count");
    assert!(ctx.last_ping_nonce().is_none(), "nonce must be cleared");
    assert_no_router_deliveries(&router);
}

/// PONG with wrong nonce is silently discarded per SCTP §8.3.
/// Miss count unchanged, nonce unchanged.
#[test]
fn pong_with_wrong_nonce_discarded() {
    let (mut ctx, router) = make_test_context();
    ctx.set_last_ping_nonce(0xCAFE);
    ctx.increment_heartbeat_miss();

    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xBAD,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    pong::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.heartbeat_miss_count(), 1, "discarded PONG must not reset miss count");
    assert_eq!(ctx.last_ping_nonce(), Some(0xCAFE), "nonce must not be cleared");
    assert_no_router_deliveries(&router);
}

/// PONG without a prior PING is silently discarded.
#[test]
fn pong_without_prior_ping_discarded() {
    let (mut ctx, router) = make_test_context();
    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 123,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    pong::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.heartbeat_miss_count(), 0);
    assert_no_router_deliveries(&router);
}

/// Stale PONG matches previous generation and resets miss count.
#[test]
fn stale_pong_accepted_and_resets_miss() {
    let (mut ctx, router) = make_test_context();

    ctx.set_last_ping_nonce(0xAAAA);
    ctx.rotate_ping_nonce();
    ctx.increment_heartbeat_miss();
    ctx.set_last_ping_nonce(0xBBBB);

    let payload = ping_codec::encode(&ping_codec::PingPayload {
        ping_nonce: 0xAAAA,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });
    let outcome = pong::verify_and_apply(&mut ctx, &payload).unwrap();
    assert!(matches!(outcome, pong::PongOutcome::AcceptedStale));
    assert_eq!(ctx.heartbeat_miss_count(), 0, "stale PONG must reset miss count");
    assert_eq!(ctx.last_ping_nonce(), Some(0xBBBB), "current nonce preserved");
    assert_no_router_deliveries(&router);
}
