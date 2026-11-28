//! Ingress routing by tag (plan E2.3): `'D'` reaches the 1:1 handler,
//! `'G'` the gossip path, and nothing is trial-decoded, so an untagged
//! datagram (the pre-E2.3 bare gossip envelope included) reaches no
//! handler.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rekindle_codec::capnp_envelope::{encode_community_envelope, encode_signed_envelope};
use rekindle_codec::community::envelope::{
    sign_envelope, CommunityEnvelope, ControlPayload, SignedEnvelope,
};
use rekindle_codec::ingress::{gossip_datagram, DIRECT_TAG};
use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortError};

use super::super::dispatch_bare::dispatch_bare_request;
use super::{dispatch_app_call, dispatch_app_message, process_gossip, GossipIngressQueue};
use crate::gossip::DedupCache;
use crate::handler::{InboundHandler, TransportEvent};
use crate::payload::rpc::{CallResponse, InboundCall};
use crate::subscriptions::media_key::{MediaKeyAdmission, MediaKeyReply};

/// What reached the handler.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Direct(Vec<u8>),
    DirectCall(Vec<u8>),
    Media(Vec<u8>, Instant),
    MediaKey,
    CallDone,
    Gossip { community: String, sender: String },
    Forward { ttl: u8 },
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    /// Its bare-request handler runs twice past `CALL_DEADLINE`.
    slow_calls: bool,
    stall_direct_calls: bool,
    mismatch_ack: bool,
    /// The slow handler calls that ran to their end.
    finished: std::sync::atomic::AtomicUsize,
}

impl Recorder {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().clone()
    }
}

impl InboundHandler for Recorder {
    fn local_identity(&self) -> Option<[u8; 32]> {
        Some([1; 32])
    }

    fn clock(&self) -> &dyn rekindle_utils::Clock {
        &rekindle_utils::SystemClock
    }

    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        rekindle_lifecycle::SessionScope::new("test", Arc::new(|_| {}))
    }

    async fn on_direct(&self, datagram: &[u8]) {
        self.seen.lock().push(Seen::Direct(datagram.to_vec()));
    }

    async fn on_direct_call(&self, datagram: &[u8], _route: Option<&str>) -> Option<Vec<u8>> {
        self.seen.lock().push(Seen::DirectCall(datagram.to_vec()));
        if self.stall_direct_calls {
            tokio::time::sleep(Duration::from_secs(12)).await;
            self.seen.lock().push(Seen::CallDone);
        }
        // `[DIRECT_TAG, 0]` stands for a call the Host left unanswered.
        (datagram.get(1) != Some(&0)).then(|| b"reply".to_vec())
    }

    fn on_media(&self, raw: &[u8], arrived: Instant) {
        self.seen.lock().push(Seen::Media(raw.to_vec(), arrived));
    }

    async fn on_media_key_call(&self, payload: &ControlPayload) -> MediaKeyReply {
        self.seen.lock().push(Seen::MediaKey);
        match payload {
            ControlPayload::VoiceMediaKey {
                community_id,
                channel_id,
                sender,
                recipient,
                key_index,
                ..
            } => MediaKeyReply::Installed(Box::new(ControlPayload::VoiceMediaKeyAck {
                community_id: if self.mismatch_ack {
                    "other".into()
                } else {
                    community_id.clone()
                },
                channel_id: channel_id.clone(),
                sender: sender.clone(),
                recipient: recipient.clone(),
                key_index: *key_index,
            })),
            ControlPayload::VoiceMediaKeyRequest { .. } => MediaKeyReply::Requested,
            _ => MediaKeyReply::Refused,
        }
    }

    async fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        _envelope: CommunityEnvelope,
        _lamport_ts: u64,
    ) {
        self.seen.lock().push(Seen::Gossip {
            community: community_id.to_string(),
            sender: sender_pseudonym.to_string(),
        });
    }

    async fn on_gossip_forward(&self, envelope: &SignedEnvelope) {
        self.seen.lock().push(Seen::Forward { ttl: envelope.ttl });
    }

    async fn on_head_advanced(&self, _community: &str, _sender: &str, _hint: &[u8]) -> bool {
        false
    }

    async fn on_call(&self, _sender: Option<&str>, _request: InboundCall) -> CallResponse {
        if self.slow_calls {
            tokio::time::sleep(crate::operations::direct::CALL_DEADLINE * 2).await;
            self.finished
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return CallResponse::Ok(b"late".to_vec());
        }
        // Anything but `Ok` is a refusal.
        CallResponse::Ack
    }

    async fn on_value_change(&self, _key: &str, _subkeys: Vec<u32>, _first: Option<Vec<u8>>) {}

    async fn on_event(&self, _event: TransportEvent) {}
}

/// The replies sent to inbound calls.
#[derive(Default)]
struct Replies(Mutex<Vec<Vec<u8>>>);

#[async_trait::async_trait]
impl AppMessagePort for Replies {
    async fn send(
        &self,
        _target: &PeerTarget,
        _profile: &rekindle_types::config::SafetyProfile,
        _message: Vec<u8>,
    ) -> Result<(), PortError> {
        Ok(())
    }

    async fn call(
        &self,
        _target: &PeerTarget,
        _profile: &rekindle_types::config::SafetyProfile,
        _message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        Ok(Vec::new())
    }

    async fn reply(&self, _call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        self.0.lock().push(message);
        Ok(())
    }
}

/// A signed community envelope from a real pseudonym key.
fn signed(ttl: u8) -> SignedEnvelope {
    signed_as([7u8; 16], &format!("ch_{ttl}"), ttl)
}

/// The envelope for `channel`, sent as message `message_id`.
fn signed_as(message_id: [u8; 16], channel: &str, ttl: u8) -> SignedEnvelope {
    let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let pseudonym = hex::encode(key.verifying_key().to_bytes());
    let mut channel_tag = [0u8; 16];
    for (slot, byte) in channel_tag.iter_mut().zip(channel.bytes()) {
        *slot = byte;
    }
    let inner = CommunityEnvelope::TypingIndicator {
        channel_tag,
        ttl_ms: 6_000,
        pseudonym_key: pseudonym.clone(),
    };
    let bytes = encode_community_envelope(&inner).unwrap();
    let mut signed = sign_envelope(&key, "gov-key", &pseudonym, message_id, &bytes);
    signed.ttl = ttl;
    signed
}

async fn message(handler: &Arc<Recorder>, raw: &[u8]) {
    let mut dedup = DedupCache::new(64);
    queued_messages(handler, &mut dedup, [raw.to_vec()]).await;
}

async fn queued_messages(
    handler: &Arc<Recorder>,
    dedup: &mut DedupCache,
    datagrams: impl IntoIterator<Item = Vec<u8>>,
) {
    let queue = GossipIngressQueue::new();
    for datagram in datagrams {
        dispatch_app_message(handler.as_ref(), &queue, datagram, Instant::now());
    }
    while queue.depth() > 0 {
        process_gossip(handler, dedup, queue.pop().await).await;
    }
}

#[tokio::test]
async fn a_direct_datagram_reaches_only_the_direct_handler() {
    let handler = Arc::new(Recorder::default());
    let datagram = [DIRECT_TAG, 1, 2, 3];
    message(&handler, &datagram).await;
    assert_eq!(handler.seen(), [Seen::Direct(datagram.to_vec())]);
}

#[tokio::test]
async fn a_gossip_datagram_is_delivered_and_forwarded() {
    let handler = Arc::new(Recorder::default());
    let envelope = signed(5);
    message(&handler, &gossip_datagram(&envelope).unwrap()).await;
    assert_eq!(
        handler.seen(),
        [
            Seen::Forward { ttl: 4 },
            Seen::Gossip {
                community: "gov-key".into(),
                sender: envelope.sender_pseudonym,
            },
        ]
    );
}

/// Gossip dedups by message id, not content (lead decision 2026-11-19,
/// libp2p pubsub origin-stamped ids): one message arriving twice (the
/// sender's copy, then a relay's with a lower TTL) is delivered once,
/// while a deliberate re-send of byte-identical content under a new id
/// (the voice roster reconcile, a retry) is delivered again.
#[tokio::test]
async fn a_relayed_copy_is_suppressed_but_a_re_send_of_the_same_content_is_delivered() {
    let handler = Arc::new(Recorder::default());
    let mut dedup = DedupCache::new(64);
    let first = signed_as([1u8; 16], "general", 5);
    let mut relayed = first.clone();
    relayed.ttl = 4;
    let resent = signed_as([2u8; 16], "general", 5);
    assert_eq!(first.envelope_bytes, resent.envelope_bytes);

    queued_messages(
        &handler,
        &mut dedup,
        [&first, &relayed, &resent].map(|envelope| gossip_datagram(envelope).unwrap()),
    )
    .await;
    let gossip = Seen::Gossip {
        community: "gov-key".into(),
        sender: first.sender_pseudonym.clone(),
    };
    assert_eq!(
        handler.seen(),
        [
            Seen::Forward { ttl: 4 },
            gossip.clone(),
            Seen::Forward { ttl: 4 },
            gossip,
        ]
    );
}

#[tokio::test]
async fn untagged_and_malformed_datagrams_reach_no_handler() {
    let handler = Arc::new(Recorder::default());
    // The pre-E2.3 wire: a bare `SignedEnvelope`, valid and signed.
    message(&handler, &encode_signed_envelope(&signed(5)).unwrap()).await;
    // A transport frame header, an empty datagram, a broken gossip body.
    message(&handler, &[0x01, 0x10, 0, 0]).await;
    message(&handler, &[]).await;
    message(&handler, &[rekindle_codec::ingress::GOSSIP_TAG, 0xFF, 0xFF]).await;
    // A gossip envelope with a forged signature.
    let mut forged = signed(5);
    forged.envelope_bytes[4] ^= 0xFF;
    message(&handler, &gossip_datagram(&forged).unwrap()).await;
    assert_eq!(handler.seen(), []);
}

#[tokio::test]
async fn a_direct_call_is_answered_and_a_gossip_call_is_refused() {
    let handler = Arc::new(Recorder::default());
    let replies = Replies::default();
    let datagram = [DIRECT_TAG, 9];
    dispatch_app_call(
        &handler,
        &replies,
        &Mutex::<MediaKeyAdmission>::default(),
        AppCallId(1),
        &datagram,
        None,
    )
    .await;
    dispatch_app_call(
        &handler,
        &replies,
        &Mutex::<MediaKeyAdmission>::default(),
        AppCallId(2),
        &gossip_datagram(&signed(5)).unwrap(),
        None,
    )
    .await;
    assert_eq!(handler.seen(), [Seen::DirectCall(datagram.to_vec())]);
    assert_eq!(
        *replies.0.lock(),
        [b"reply".to_vec(), crate::operations::direct::NAK.to_vec()]
    );
}

/// A direct call the handler leaves unanswered gets no reply at all, never
/// a NAK (a refusal): the caller's `app_call` times out and is attempted
/// again (RV2).
#[tokio::test]
async fn a_direct_call_left_unanswered_gets_no_reply() {
    let handler = Arc::new(Recorder::default());
    let replies = Replies::default();
    let datagram = [DIRECT_TAG, 0];
    dispatch_app_call(
        &handler,
        &replies,
        &Mutex::<MediaKeyAdmission>::default(),
        AppCallId(1),
        &datagram,
        None,
    )
    .await;
    assert_eq!(handler.seen(), [Seen::DirectCall(datagram.to_vec())]);
    assert!(replies.0.lock().is_empty());
}

/// Audit G10: a `'G'` call is refused by its tag, whatever follows it: one
/// whose body is no envelope, and one carrying a transport frame (what an
/// untagged call would decode), are NAKed and reach no handler.
#[tokio::test]
async fn a_gossip_call_is_refused_before_any_decode() {
    let handler = Arc::new(Recorder::default());
    let replies = Replies::default();
    let mut framed = vec![rekindle_codec::ingress::GOSSIP_TAG];
    framed.extend(crate::frame::encode(crate::frame::TypeId::CommunityLeave, b"x").unwrap());
    for (n, datagram) in [vec![rekindle_codec::ingress::GOSSIP_TAG, 0xFF], framed]
        .into_iter()
        .enumerate()
    {
        dispatch_app_call(
            &handler,
            &replies,
            &Mutex::<MediaKeyAdmission>::default(),
            AppCallId(n as u64),
            &datagram,
            None,
        )
        .await;
    }
    assert_eq!(handler.seen(), []);
    assert_eq!(
        *replies.0.lock(),
        [
            crate::operations::direct::NAK.to_vec(),
            crate::operations::direct::NAK.to_vec()
        ]
    );
}

/// The finding of round 23: a joiner's bare `BootstrapRequest` whose
/// handler runs past `CALL_DEADLINE` is answered `RETRY_LATER`, as a 1:1
/// call is (a deadline is no refusal, review RV5 H1), never the NAK that
/// withdraws the join; a request the handler refuses stays a NAK. The
/// clock is paused, so the deadline passes without waiting.
#[tokio::test(start_paused = true)]
async fn a_bare_request_past_the_deadline_is_answered_retry_later() {
    use crate::operations::direct::{NAK, RETRY_LATER};
    let bootstrap = || InboundCall::BootstrapRequest {
        joiner_pseudonym: "joiner".into(),
        anchor: "VLD0:anchor".into(),
        member_joined: rekindle_types::governance::OpRef {
            op_hash: rekindle_types::governance::OpHash([1; 32]),
            device_key: [2; 32],
            author: rekindle_types::id::PseudonymKey([3; 32]),
            author_record: "VLD0:record".into(),
            author_seq: 1,
        },
        member_joined_op: vec![4],
        joiner_route: vec![5],
    };
    let snapshot = || InboundCall::CheckpointStateRequest { request: vec![6] };
    let replies = Replies::default();
    let slow = Arc::new(Recorder {
        slow_calls: true,
        ..Recorder::default()
    });
    let refusing = Arc::new(Recorder::default());
    dispatch_bare_request(&slow, &replies, AppCallId(1), bootstrap()).await;
    dispatch_bare_request(&slow, &replies, AppCallId(2), snapshot()).await;
    dispatch_bare_request(&refusing, &replies, AppCallId(3), bootstrap()).await;
    assert_eq!(
        *replies.0.lock(),
        [RETRY_LATER.to_vec(), RETRY_LATER.to_vec(), NAK.to_vec()]
    );
}

/// Plan JP1 (review RV5 H1): a bare request answered retry-later at the
/// deadline still runs its handler to the end (a bootstrap answer's
/// invite revoke and key delivery), as a 1:1 call's work does: the
/// deadline drops only the answer, never the work.
#[tokio::test(start_paused = true)]
async fn a_bare_request_past_the_deadline_finishes_its_work() {
    use std::sync::atomic::Ordering;

    use crate::operations::direct::{CALL_DEADLINE, RETRY_LATER};
    let replies = Replies::default();
    let slow = Arc::new(Recorder {
        slow_calls: true,
        ..Recorder::default()
    });
    let request = InboundCall::CheckpointStateRequest { request: vec![6] };
    dispatch_bare_request(&slow, &replies, AppCallId(1), request).await;
    assert_eq!(*replies.0.lock(), [RETRY_LATER.to_vec()]);
    assert_eq!(slow.finished.load(Ordering::SeqCst), 0, "answered first");
    tokio::time::sleep(CALL_DEADLINE * 2).await;
    assert_eq!(
        slow.finished.load(Ordering::SeqCst),
        1,
        "the handler's work goes on past the deadline"
    );
}

#[path = "dispatch_lane_tests.rs"]
mod lane_tests;
