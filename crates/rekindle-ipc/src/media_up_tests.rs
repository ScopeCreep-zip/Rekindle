//! `BusPayload::MediaUp` over a real Noise bus (plan E4.1 S2): a client's
//! encoded frame reaches only the daemon, a daemon-origin one is dropped,
//! and frames never spend the request bucket.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use rekindle_types::media::{MediaScope, Track};
use rekindle_types::video::{Codec, UplinkFrame};

use super::framing::{decode_frame, encode_frame};
use super::message::{Message, MessageContext, SecurityLevel};
use super::noise_keys::{generate_keypair, ZeroizingKeypair};
use super::protocol::{AgentCapability, BusPayload, IpcRequest, IpcResponse};
use super::registry::ClearanceRegistry;
use super::server::{BusServer, DAEMON_AGENT_NAME};
use super::{AgentType, BusClient, SubscriptionFilter};

fn frame(ts: u64) -> UplinkFrame {
    UplinkFrame {
        scope: MediaScope::DirectCall {
            peer: "b".repeat(64),
        },
        track: Track::Camera,
        keyframe: ts == 0,
        codec: Codec::Vp8,
        payload: vec![0xAB, 0xCD],
        timestamp_ms: ts,
    }
}

/// A running bus with the daemon, a subscribed consumer and a sender,
/// each registered (the clients `Admin`, as an unlocked identity's).
struct Bus {
    _dir: tempfile::TempDir,
    daemon: BusClient,
    consumer: BusClient,
    sender: BusClient,
    registry: Arc<tokio::sync::RwLock<ClearanceRegistry>>,
    sender_key: [u8; 32],
}

async fn bus() -> Bus {
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let keys: Vec<ZeroizingKeypair> = (0..3).map(|_| generate_keypair().unwrap()).collect();
    let mut registry = ClearanceRegistry::new();
    for (i, (name, level, kind)) in [
        (
            DAEMON_AGENT_NAME,
            SecurityLevel::Internal,
            AgentType::System,
        ),
        ("client-consumer", SecurityLevel::Admin, AgentType::Human),
        ("client-sender", SecurityLevel::Admin, AgentType::Human),
    ]
    .into_iter()
    .enumerate()
    {
        let public: [u8; 32] = keys[i].public().try_into().unwrap();
        registry.register(name.to_string(), public, level, kind, vec![]);
    }
    let registry = Arc::new(tokio::sync::RwLock::new(registry));
    let sender_key = keys[2].public().try_into().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus.sock");
    let server =
        Arc::new(BusServer::bind(&sock, server_kp.into_inner(), Arc::clone(&registry)).unwrap());
    server.start_event_delivery(Arc::new(rekindle_events::SessionJournal::with_rng(
        Arc::new(rekindle_utils::random::OsRandom),
    )));
    let srv = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = srv.run().await;
    });
    let mut clients = Vec::new();
    for kp in &keys {
        clients.push(
            BusClient::connect(Uuid::now_v7(), &sock, &server_pub, kp.as_inner())
                .await
                .unwrap(),
        );
    }
    let sender = clients.pop().unwrap();
    let consumer = clients.pop().unwrap();
    let daemon = clients.pop().unwrap();
    let resp = consumer
        .request(
            IpcRequest::Subscribe {
                filters: vec![SubscriptionFilter::all()],
                from: None,
            },
            SecurityLevel::Open,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(matches!(resp, IpcResponse::Ok { .. }), "{resp:?}");
    Bus {
        _dir: dir,
        daemon,
        consumer,
        sender,
        registry,
        sender_key,
    }
}

#[test]
fn media_up_is_appended_after_media_and_round_trips() {
    let msg = Message::new(
        &MessageContext::new(Uuid::nil()),
        BusPayload::MediaUp(frame(5)),
        SecurityLevel::Internal,
        std::time::Instant::now(),
    );
    let bytes = encode_frame(&msg).unwrap();
    // Pin the bare discriminants independently of the envelope and decoder.
    assert_eq!(postcard::to_allocvec(&msg.payload).unwrap()[0], 4);
    let back: Message<BusPayload> = decode_frame(&bytes).unwrap();
    assert!(matches!(back.payload, BusPayload::MediaUp(f) if f == frame(5)));
}

#[tokio::test]
async fn a_client_media_up_reaches_only_the_daemon() {
    let mut bus = bus().await;
    let mut daemon_up = bus.daemon.take_media_up_receiver().unwrap();
    let mut consumer_up = bus.consumer.take_media_up_receiver().unwrap();
    let mut consumer_media = bus.consumer.take_media_receiver().unwrap();
    bus.sender
        .send_media_up(frame(1), SecurityLevel::Open)
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), daemon_up.recv())
        .await
        .expect("the daemon receives the frame")
        .unwrap();
    assert_eq!(got, frame(1));
    let leaked = tokio::time::timeout(Duration::from_millis(500), async {
        tokio::select! {
            f = consumer_up.recv() => format!("uplink {f:?}"),
            f = consumer_media.recv() => format!("media {f:?}"),
        }
    })
    .await;
    assert!(leaked.is_err(), "another client never sees it: {leaked:?}");
}

#[tokio::test]
async fn a_daemon_origin_media_up_is_dropped() {
    let mut bus = bus().await;
    let mut consumer_up = bus.consumer.take_media_up_receiver().unwrap();
    let mut daemon_up = bus.daemon.take_media_up_receiver().unwrap();
    bus.daemon
        .send_media_up(frame(2), SecurityLevel::Internal)
        .await
        .unwrap();
    let leaked = tokio::time::timeout(Duration::from_millis(500), async {
        tokio::select! {
            f = consumer_up.recv() => format!("client {f:?}"),
            f = daemon_up.recv() => format!("daemon {f:?}"),
        }
    })
    .await;
    assert!(
        leaked.is_err(),
        "a daemon-origin MediaUp goes nowhere: {leaked:?}"
    );
}

#[tokio::test]
async fn media_up_frames_never_spend_the_request_bucket() {
    let mut bus = bus().await;
    let mut daemon_up = bus.daemon.take_media_up_receiver().unwrap();
    let request = bus.sender.request(
        IpcRequest::Status,
        SecurityLevel::Open,
        Duration::from_secs(2),
    );
    let flow = async {
        let pending = tokio::time::timeout(Duration::from_secs(2), bus.daemon.recv_bus_message())
            .await
            .expect("the daemon received the in-flight request")
            .unwrap()
            .unwrap();
        assert!(
            matches!(pending.payload, BusPayload::Request(req) if matches!(*req, IpcRequest::Status))
        );

        // Consume each frame so this measures admission, independently of
        // queue eviction. Alternate tracks on the same client connection.
        for ts in 0..150 {
            let mut sent = frame(ts);
            if ts % 2 == 1 {
                sent.track = Track::Screen;
            }
            bus.sender
                .send_media_up(sent.clone(), SecurityLevel::Open)
                .await
                .unwrap();
            let received = tokio::time::timeout(Duration::from_secs(1), daemon_up.recv())
                .await
                .expect("frame used the uplink bucket");
            assert_eq!(received, Some(sent));
            if ts == 49 {
                bus.daemon
                    .respond(
                        serde_json::to_vec(&IpcResponse::ok(&"ready")).unwrap(),
                        pending.msg_id,
                        SecurityLevel::Internal,
                    )
                    .await
                    .unwrap();
            }
        }
    };
    let (in_flight, ()) = tokio::join!(request, flow);
    assert!(matches!(in_flight.unwrap(), IpcResponse::Ok { data, .. } if data == "ready"));
    let resp = bus
        .sender
        .request(
            IpcRequest::Unsubscribe { filters: vec![] },
            SecurityLevel::Open,
            Duration::from_secs(1),
        )
        .await
        .expect("a request after 150 frames is not rate-limited");
    assert!(matches!(resp, IpcResponse::Ok { .. }), "{resp:?}");
}

#[tokio::test]
async fn an_uplink_uses_current_clearance_and_requires_voice_write_for_agents() {
    let mut bus = bus().await;
    let mut uplink = bus.daemon.take_media_up_receiver().unwrap();
    let mut registry = bus.registry.write().await;
    registry.revoke_by_name("client-sender");
    drop(registry);

    bus.sender
        .send_media_up(frame(1), SecurityLevel::Admin)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), uplink.recv())
            .await
            .is_err()
    );

    bus.registry.write().await.register(
        "client-sender".into(),
        bus.sender_key,
        SecurityLevel::Agent,
        AgentType::Bot,
        vec![AgentCapability::EventsRead],
    );
    bus.sender
        .send_media_up(frame(2), SecurityLevel::Admin)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), uplink.recv())
            .await
            .is_err()
    );

    bus.registry.write().await.register(
        "client-sender".into(),
        bus.sender_key,
        SecurityLevel::Agent,
        AgentType::Bot,
        vec![AgentCapability::VoiceWrite],
    );
    bus.sender
        .send_media_up(frame(3), SecurityLevel::Open)
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), uplink.recv())
            .await
            .unwrap(),
        Some(frame(3))
    );
}
