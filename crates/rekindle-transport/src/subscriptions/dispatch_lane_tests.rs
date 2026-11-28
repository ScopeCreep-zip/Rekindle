use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use rekindle_codec::capnp_envelope::{encode_community_envelope, try_decode_community_envelope};
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::ingress::{DIRECT_TAG, VOICE_TAG};
use rekindle_records::port::{AppCallId, AppMessagePort, PortEvent, RoutePort};
use tokio::sync::mpsc;

use super::{dispatch_app_call, Recorder, Replies, Seen};
use crate::broadcast::peer_registry::PeerRegistry;
use crate::config::TransportConfig;
use crate::liveness::{receipt_tests::OneRoute, Liveness};
use crate::operations::direct::NAK;
use crate::shared::SharedState;
use crate::subscriptions::dispatch::{run_dispatch_loop, RouteOwners};
use crate::subscriptions::media_key::{MediaKeyAdmission, MEDIA_KEY_BURST};

#[tokio::test(start_paused = true)]
async fn media_is_delivered_while_a_call_is_stalled() {
    let handler = Arc::new(Recorder {
        stall_direct_calls: true,
        ..Recorder::default()
    });
    let replies = Arc::new(Replies::default());
    let (events, events_rx) = mpsc::channel(8);
    let (shutdown, shutdown_rx) = mpsc::channel(1);
    let routes: Arc<dyn RoutePort> = OneRoute::new();
    let task = tokio::spawn(run_dispatch_loop(
        Arc::clone(&handler),
        Arc::new(TransportConfig::default()),
        events_rx,
        shutdown_rx,
        Arc::clone(&replies) as Arc<dyn AppMessagePort>,
        SharedState::new(),
        RouteOwners {
            liveness: Liveness::new(
                Arc::new(rekindle_utils::OsRandom),
                Arc::clone(&routes),
                true,
            ),
            peers: Arc::new(RwLock::new(PeerRegistry::new(60, 3, 5))),
            routes,
        },
    ));

    let call = vec![DIRECT_TAG, 5];
    events
        .send(PortEvent::AppCall {
            call_id: AppCallId(1),
            message: call.clone(),
            route: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(handler.seen(), [Seen::DirectCall(call)]);
    let media = vec![VOICE_TAG, 1, 0, 0, 0, 9];
    let arrived = tokio::time::Instant::now().into_std();
    events
        .send(PortEvent::AppMessage {
            message: media.clone(),
            route: None,
        })
        .await
        .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(handler.seen().last(), Some(&Seen::Media(media, arrived)));
    assert!(replies.0.lock().is_empty(), "the call is still blocked");
    tokio::time::sleep(Duration::from_secs(13)).await;
    assert_eq!(handler.seen().last(), Some(&Seen::CallDone));
    assert_eq!(*replies.0.lock(), [b"reply".to_vec()]);

    shutdown.send(()).await.unwrap();
    events.send(PortEvent::Shutdown).await.unwrap();
    task.await.unwrap();
}

fn request(requester: &str) -> ControlPayload {
    ControlPayload::VoiceMediaKeyRequest {
        community_id: "c".into(),
        channel_id: "ch".into(),
        requester: requester.into(),
        sender: "aa".repeat(32),
        key_index: 1,
    }
}

fn key() -> ControlPayload {
    ControlPayload::VoiceMediaKey {
        community_id: "c".into(),
        channel_id: "ch".into(),
        sender: "aa".repeat(32),
        recipient: "bb".repeat(32),
        key_index: 1,
        sealed: vec![7; 80],
        signature: vec![8; 64],
    }
}

async fn bare_call(
    handler: &Arc<Recorder>,
    replies: &Replies,
    admission: &Mutex<MediaKeyAdmission>,
    payload: ControlPayload,
) {
    let raw = encode_community_envelope(&CommunityEnvelope::control(payload)).unwrap();
    dispatch_app_call(handler, replies, admission, AppCallId(1), &raw, None).await;
}

#[tokio::test(start_paused = true)]
async fn the_bare_media_key_branch_admits_two_variants_per_requester_bucket() {
    let handler = Arc::new(Recorder::default());
    let replies = Replies::default();
    let admission = Mutex::default();
    let requester = "aa".repeat(32);
    for _ in 0..=MEDIA_KEY_BURST {
        bare_call(&handler, &replies, &admission, request(&requester)).await;
    }
    bare_call(&handler, &replies, &admission, request(&"bb".repeat(32))).await;
    bare_call(&handler, &replies, &admission, key()).await;
    tokio::time::advance(Duration::from_millis(500)).await;
    bare_call(&handler, &replies, &admission, request(&requester)).await;

    let replies = replies.0.lock();
    let burst = usize::try_from(MEDIA_KEY_BURST).unwrap();
    assert!(replies[..burst].iter().all(Vec::is_empty));
    assert_eq!(replies[burst], NAK, "one past the burst is refused");
    assert!(
        replies[burst + 1].is_empty(),
        "another requester is admitted"
    );
    let ack = try_decode_community_envelope(&replies[burst + 2])
        .unwrap()
        .unwrap();
    assert!(matches!(
        ack.as_control(),
        Some(ControlPayload::VoiceMediaKeyAck {
            community_id, channel_id, sender, recipient, key_index: 1,
        }) if community_id == "c" && channel_id == "ch"
            && sender == &"aa".repeat(32) && recipient == &"bb".repeat(32)
    ));
    assert!(replies[burst + 3].is_empty(), "one token refilled");
    assert_eq!(handler.seen(), vec![Seen::MediaKey; burst + 3]);
}

#[tokio::test]
async fn bare_media_key_ack_and_unrelated_control_are_refused() {
    let handler = Arc::new(Recorder {
        mismatch_ack: true,
        ..Recorder::default()
    });
    let replies = Replies::default();
    let admission = Mutex::default();
    bare_call(
        &handler,
        &replies,
        &admission,
        ControlPayload::VoiceMediaKeyAck {
            community_id: "c".into(),
            channel_id: "ch".into(),
            sender: "aa".repeat(32),
            recipient: "bb".repeat(32),
            key_index: 1,
        },
    )
    .await;
    bare_call(
        &handler,
        &replies,
        &admission,
        ControlPayload::MEKRotated {
            channel_id: Some("general".into()),
            new_generation: 9,
            rotator_pseudonym: None,
        },
    )
    .await;
    assert!(handler.seen().is_empty());
    bare_call(&handler, &replies, &admission, key()).await;
    assert_eq!(handler.seen(), [Seen::MediaKey]);
    assert_eq!(
        *replies.0.lock(),
        [NAK.to_vec(), NAK.to_vec(), NAK.to_vec()]
    );
}
