//! Ingress routing by tag (plan E2.3): `'D'` reaches the 1:1 handler,
//! `'G'` the gossip path, and nothing is trial-decoded, so an untagged
//! datagram (the pre-E2.3 bare gossip envelope included) reaches no
//! handler.

use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_codec::capnp_envelope::{encode_community_envelope, encode_signed_envelope};
use rekindle_codec::community::envelope::{sign_envelope, CommunityEnvelope, SignedEnvelope};
use rekindle_codec::ingress::{gossip_datagram, DIRECT_TAG};
use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortError};

use super::{dispatch_app_call, dispatch_app_message};
use crate::gossip::DedupCache;
use crate::handler::{InboundHandler, TransportEvent};
use crate::payload::rpc::{CallResponse, InboundCall};

/// What reached the handler.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Direct(Vec<u8>),
    DirectCall(Vec<u8>),
    Gossip { community: String, sender: String },
    Forward { ttl: u8 },
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
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

    async fn on_direct(&self, datagram: &[u8]) {
        self.seen.lock().push(Seen::Direct(datagram.to_vec()));
    }

    async fn on_direct_call(&self, datagram: &[u8]) -> Option<Vec<u8>> {
        self.seen.lock().push(Seen::DirectCall(datagram.to_vec()));
        // `[DIRECT_TAG, 0]` stands for a call the Host left unanswered.
        (datagram.get(1) != Some(&0)).then(|| b"reply".to_vec())
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

    async fn on_call(&self, _sender: Option<&str>, _request: InboundCall) -> CallResponse {
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

    async fn send_pinned(
        &self,
        _target: &PeerTarget,
        _profile: &rekindle_types::config::SafetyProfile,
        _class: rekindle_records::port::RouteClass,
        _message: Vec<u8>,
    ) -> Result<(), PortError> {
        Ok(())
    }

    async fn call_pinned(
        &self,
        _target: &PeerTarget,
        _profile: &rekindle_types::config::SafetyProfile,
        _class: rekindle_records::port::RouteClass,
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
    let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let pseudonym = hex::encode(key.verifying_key().to_bytes());
    let inner = CommunityEnvelope::MessageNotification {
        channel_id: "ab".repeat(8),
        message_id: format!("msg_{ttl}"),
        author_pseudonym: pseudonym.clone(),
        subkey_index: 1,
        lamport_ts: 1,
        sequence: 0,
        content_hash: "ef".repeat(32),
        timestamp: 1_700_000_000,
    };
    let bytes = encode_community_envelope(&inner).unwrap();
    let mut signed = sign_envelope(&key, "gov-key", &pseudonym, &bytes);
    signed.ttl = ttl;
    signed
}

async fn message(handler: &Arc<Recorder>, raw: &[u8]) {
    let mut dedup = DedupCache::new(64);
    dispatch_app_message(handler, &mut dedup, raw).await;
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
    message(&handler, &gossip_datagram(&envelope)).await;
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

#[tokio::test]
async fn untagged_and_malformed_datagrams_reach_no_handler() {
    let handler = Arc::new(Recorder::default());
    // The pre-E2.3 wire: a bare `SignedEnvelope`, valid and signed.
    message(&handler, &encode_signed_envelope(&signed(5))).await;
    // A transport frame header, an empty datagram, a broken gossip body.
    message(&handler, &[0x01, 0x10, 0, 0]).await;
    message(&handler, &[]).await;
    message(&handler, &[rekindle_codec::ingress::GOSSIP_TAG, 0xFF, 0xFF]).await;
    // A gossip envelope with a forged signature.
    let mut forged = signed(5);
    forged.envelope_bytes[4] ^= 0xFF;
    message(&handler, &gossip_datagram(&forged)).await;
    assert_eq!(handler.seen(), []);
}

#[tokio::test]
async fn a_direct_call_is_answered_and_a_gossip_call_is_refused() {
    let handler = Arc::new(Recorder::default());
    let replies = Replies::default();
    let datagram = [DIRECT_TAG, 9];
    dispatch_app_call(&handler, &replies, AppCallId(1), &datagram).await;
    dispatch_app_call(
        &handler,
        &replies,
        AppCallId(2),
        &gossip_datagram(&signed(5)),
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
    dispatch_app_call(&handler, &replies, AppCallId(1), &datagram).await;
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
    framed.extend(crate::frame::encode(crate::frame::TypeId::SyncRequest, b"x").unwrap());
    for (n, datagram) in [vec![rekindle_codec::ingress::GOSSIP_TAG, 0xFF], framed]
        .into_iter()
        .enumerate()
    {
        dispatch_app_call(&handler, &replies, AppCallId(n as u64), &datagram).await;
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
