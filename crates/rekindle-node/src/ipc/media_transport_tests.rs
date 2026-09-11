//! End-to-end tests for the `BusPayload::Media` frame transport (Step 3).
//!
//! Two levels of proof:
//! - **postcard round-trip** — a `MediaFrame`, and a full
//!   `Message<BusPayload::Media>`, survive the postcard wire encoding the bus
//!   uses (externally tagged, no JSON hop).
//! - **push → server-route → client-receive** — the daemon's `send_media`
//!   entrypoint pushes a frame across a real Noise bus, the server fans it out
//!   to a subscribed consumer, and the consumer reads it via
//!   `take_media_receiver`. Drop-oldest-under-overflow is proven
//!   deterministically on the queue primitive in `media_channel`; here we prove
//!   the wiring end to end and that `send_media` is exercised (not dead code).

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use rekindle_types::video::{Codec, MediaFrame};

use super::framing::{decode_frame, encode_frame};
use super::message::{Message, MessageContext, SecurityLevel};
use super::noise_keys::generate_keypair;
use super::protocol::{BusPayload, IpcRequest, IpcResponse};
use super::registry::ClearanceRegistry;
use super::server::{BusServer, DAEMON_AGENT_NAME};
use super::{AgentType, BusClient, SubscriptionFilter};

fn sample_frame(seq: u32) -> MediaFrame {
    MediaFrame {
        stream_id: "ab12cd34".into(),
        sequence: seq,
        keyframe: seq == 0,
        codec: Codec::Vp9,
        // Distinct per-frame content; sequence/timestamp already differ by seq.
        payload: vec![0xDE, 0xAD, 0xBE, 0xEF],
        timestamp_ms: 1_700_000_000_000 + u64::from(seq),
    }
}

#[test]
fn media_frame_postcard_roundtrip() {
    let frame = sample_frame(42);
    let bytes = encode_frame(&frame).expect("encode");
    let back: MediaFrame = decode_frame(&bytes).expect("decode");
    assert_eq!(frame, back, "MediaFrame must round-trip through postcard");
}

#[test]
fn bus_payload_media_variant_postcard_roundtrip() {
    // The whole envelope, with the appended `Media` variant, must survive the
    // postcard encoding the bus actually uses — proving the variant index is
    // stable and the payload is postcard-native (no JSON hop like `Response`).
    let ctx = MessageContext::new(Uuid::nil());
    let frame = sample_frame(7);
    let msg = Message::new(
        &ctx,
        BusPayload::Media(frame.clone()),
        SecurityLevel::Internal,
        std::time::Instant::now(),
    );
    let bytes = encode_frame(&msg).expect("encode");
    let back: Message<BusPayload> = decode_frame(&bytes).expect("decode");
    match back.payload {
        BusPayload::Media(got) => assert_eq!(got, frame),
        other => panic!("expected Media, got {other:?}"),
    }
}

#[tokio::test]
async fn push_route_receive_end_to_end() {
    // ── Keys + registry: the daemon is a known agent; the consumer ephemeral.
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let daemon_kp = generate_keypair().unwrap();
    let daemon_pub: [u8; 32] = daemon_kp.public().try_into().unwrap();

    let mut registry = ClearanceRegistry::new();
    registry.register(
        DAEMON_AGENT_NAME.to_string(),
        daemon_pub,
        SecurityLevel::Internal,
        AgentType::System,
        vec![],
    );

    // ── Bind + run the real bus server on a temp socket.
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus.sock");
    let server = Arc::new(BusServer::bind(&sock, server_kp.into_inner(), registry).unwrap());
    let srv = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = srv.run().await;
    });

    // ── Consumer connects (ephemeral) and subscribes so it joins the event
    //    router — the media audience.
    let consumer_kp = generate_keypair().unwrap();
    let mut consumer =
        BusClient::connect(Uuid::now_v7(), &sock, &server_pub, consumer_kp.as_inner())
            .await
            .unwrap();
    let mut media_rx = consumer.take_media_receiver().expect("media receiver");

    let resp = consumer
        .request(
            IpcRequest::Subscribe {
                filters: vec![SubscriptionFilter::all()],
            },
            SecurityLevel::Open,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(
        matches!(resp, IpcResponse::Ok(_)),
        "subscribe should succeed: {resp:?}"
    );

    // ── Daemon connects and pushes a frame via the send entrypoint.
    let daemon = BusClient::connect(Uuid::now_v7(), &sock, &server_pub, daemon_kp.as_inner())
        .await
        .unwrap();
    let frame = sample_frame(1);
    daemon
        .send_media(frame.clone(), SecurityLevel::Internal)
        .await
        .unwrap();

    // ── Consumer receives the exact frame over the wire.
    let got = tokio::time::timeout(Duration::from_secs(5), media_rx.recv())
        .await
        .expect("media frame should arrive")
        .expect("media channel open");
    assert_eq!(got, frame, "frame must survive push → route → receive");
}

#[tokio::test]
async fn client_origin_media_is_rejected_by_server() {
    // A non-daemon client that sends `Media` must have it dropped by the
    // server (mirroring the `Event` arm), never fanned out to other clients.
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    // Empty registry — every client is ephemeral (never the daemon).
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus.sock");
    let server =
        Arc::new(BusServer::bind(&sock, server_kp.into_inner(), ClearanceRegistry::new()).unwrap());
    let srv = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = srv.run().await;
    });

    // Consumer subscribes and takes its media receiver.
    let consumer_kp = generate_keypair().unwrap();
    let mut consumer =
        BusClient::connect(Uuid::now_v7(), &sock, &server_pub, consumer_kp.as_inner())
            .await
            .unwrap();
    let mut media_rx = consumer.take_media_receiver().unwrap();
    consumer
        .request(
            IpcRequest::Subscribe {
                filters: vec![SubscriptionFilter::all()],
            },
            SecurityLevel::Open,
            Duration::from_secs(5),
        )
        .await
        .unwrap();

    // A second ephemeral client tries to inject a frame.
    let attacker_kp = generate_keypair().unwrap();
    let attacker = BusClient::connect(Uuid::now_v7(), &sock, &server_pub, attacker_kp.as_inner())
        .await
        .unwrap();
    attacker
        .send_media(sample_frame(9), SecurityLevel::Open)
        .await
        .unwrap();

    // The consumer must never see it: the fan-out is daemon-only.
    let outcome = tokio::time::timeout(Duration::from_millis(500), media_rx.recv()).await;
    assert!(
        outcome.is_err(),
        "client-origin media must be dropped, not fanned out"
    );
}
