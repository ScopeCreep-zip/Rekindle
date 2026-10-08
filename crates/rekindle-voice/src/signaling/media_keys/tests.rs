//! Media-key distribution (plans C7.20, C7.22) between mock participants
//! that seal and open with the real HPKE construction and answer calls
//! the way a host does: a `VoiceMediaKey` is installed and acknowledged, a
//! `VoiceMediaKeyRequest` starts deliveries.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use parking_lot::Mutex;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::media_sender_key::keyring::{ChannelSenderKeyStore, ChannelSenderKeys};
use rekindle_types::lamport::LamportError;
use zeroize::Zeroizing;

use super::*;
use crate::signaling::deps::{CommunityVoiceEvent, StageChannelInfo, VoiceSignalingDeps};
use crate::transport::VoiceTransport;

const COMMUNITY: &str = "c";
const CHANNEL: &str = "ch";

/// Routes to participants, as the network would deliver an `app_call`.
type Net = Arc<Mutex<HashMap<Vec<u8>, Weak<Peer>>>>;

/// One participant: its pseudonym key, roster, keyring and calls made.
struct Peer {
    signing: SigningKey,
    route: Vec<u8>,
    net: Net,
    is_stage: bool,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    keys: ChannelSenderKeyStore,
    /// Every key call made, as `(recipient, key_index)`.
    calls: Mutex<Vec<(String, u64)>>,
    /// Calls to fail before the network starts delivering.
    drop_calls: AtomicUsize,
    scope: Arc<rekindle_lifecycle::SessionScope>,
}

impl Peer {
    fn new(net: &Net, seed: u8, is_stage: bool) -> Arc<Self> {
        let peer = Arc::new(Self {
            signing: SigningKey::from_bytes(&[seed; 32]),
            route: vec![seed],
            net: Arc::clone(net),
            is_stage,
            transport: Arc::new(tokio::sync::Mutex::new(VoiceTransport::new(
                CHANNEL.to_string(),
            ))),
            keys: ChannelSenderKeyStore::default(),
            calls: Mutex::new(Vec::new()),
            drop_calls: AtomicUsize::new(0),
            scope: rekindle_lifecycle::SessionScope::new("test", Arc::new(|_| {})),
        });
        net.lock().insert(peer.route.clone(), Arc::downgrade(&peer));
        peer
    }

    fn id(&self) -> String {
        hex::encode(self.signing.verifying_key().to_bytes())
    }

    fn deps(self: &Arc<Self>) -> Arc<dyn VoiceSignalingDeps> {
        Arc::clone(self) as Arc<dyn VoiceSignalingDeps>
    }

    async fn roster(&self, peers: &[&Arc<Peer>]) {
        let mut t = self.transport.lock().await;
        for peer in peers {
            t.add_peer(&peer.id(), &peer.route, None);
        }
    }

    fn sender_keys(&self) -> Arc<ChannelSenderKeys> {
        self.keys.keys(COMMUNITY, CHANNEL)
    }

    /// Our key at `index` as `peer` holds it, if it does.
    fn held_by(&self, peer: &Peer, index: u64) -> Option<[u8; 32]> {
        peer.sender_keys()
            .secret_at(&self.id(), index)
            .ok()
            .map(|s| *s)
    }

    fn take_calls(&self) -> Vec<(String, u64)> {
        std::mem::take(&mut *self.calls.lock())
    }

    /// Let every task this participant started run to its end, including
    /// deliveries those tasks start (the scope stays open meanwhile, as a
    /// login scope does).
    async fn settle(&self) {
        while !self.scope.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }
}

fn verifying(hex_key: &str) -> Option<[u8; 32]> {
    hex::decode(hex_key).ok()?.try_into().ok()
}

#[async_trait]
impl VoiceSignalingDeps for Peer {
    fn my_pseudonym(&self, _: &str) -> Option<String> {
        Some(self.id())
    }
    fn our_media_route_blob(&self) -> Option<Vec<u8>> {
        Some(self.route.clone())
    }
    fn stage_channel_info(&self, _: &str, _: &str) -> Option<StageChannelInfo> {
        Some(StageChannelInfo {
            is_stage: self.is_stage,
            speakers: Vec::new(),
            moderator: None,
        })
    }
    fn update_stage_channel(&self, _: &str, _: &str, _: Option<String>, _: Vec<String>, _: String) {
    }
    fn online_voice_members(&self, _: &str) -> Vec<(String, Vec<u8>)> {
        Vec::new()
    }
    fn decode_channel_id(&self, _: &str) -> Option<[u8; 16]> {
        None
    }
    fn my_permissions(&self, _: &str, _: Option<[u8; 16]>) -> u64 {
        0
    }
    fn sender_has_perm(&self, _: &str, _: &str, _: u64) -> bool {
        false
    }
    fn transport_handle(&self) -> Option<Arc<tokio::sync::Mutex<VoiceTransport>>> {
        Some(Arc::clone(&self.transport))
    }
    fn voice_engine_channel_id(&self) -> Option<String> {
        Some(CHANNEL.to_string())
    }
    fn voice_engine_bound_to(&self, community_id: &str, channel_id: &str) -> bool {
        community_id == COMMUNITY && channel_id == CHANNEL
    }
    fn set_voice_engine_muted(&self, _: bool) {}
    fn set_voice_engine_deafened(&self, _: bool) {}
    fn media_live_peers(&self) -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }
    fn channel_sender_keys(&self, community_id: &str, channel_id: &str) -> Arc<ChannelSenderKeys> {
        self.keys.keys(community_id, channel_id)
    }
    fn seal_media_key(
        &self,
        _: &str,
        recipient: &str,
        aad: &[u8],
        secret: &[u8; 32],
    ) -> Option<Vec<u8>> {
        rekindle_secrets::media_sender_key::seal(&self.signing, &verifying(recipient)?, aad, secret)
            .ok()
    }
    fn open_media_key(
        &self,
        _: &str,
        sender: &str,
        aad: &[u8],
        sealed: &[u8],
    ) -> Option<Zeroizing<[u8; 32]>> {
        rekindle_secrets::media_sender_key::open(&self.signing, &verifying(sender)?, aad, sealed)
            .ok()
    }
    async fn call_peer(
        &self,
        route_blob: &[u8],
        envelope: &CommunityEnvelope,
    ) -> Option<CommunityEnvelope> {
        let CommunityEnvelope::Control(payload) = envelope else {
            return None;
        };
        if let ControlPayload::VoiceMediaKey {
            recipient,
            key_index,
            ..
        } = payload
        {
            self.calls.lock().push((recipient.clone(), *key_index));
        }
        if self
            .drop_calls
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return None;
        }
        let target = self.net.lock().get(route_blob)?.upgrade()?;
        match payload {
            ControlPayload::VoiceMediaKey { .. } => {
                handle_voice_media_key(&target.deps(), payload).map(CommunityEnvelope::Control)
            }
            ControlPayload::VoiceMediaKeyRequest { .. } => {
                handle_voice_media_key_request(&target.deps(), payload);
                None
            }
            _ => None,
        }
    }
    fn send_to_mesh(&self, _: &str, _: &CommunityEnvelope) {}
    fn advertise_media_capabilities(&self, _: &str, _: &str) {}
    fn send_to_channel(&self, _: &str, _: &str, _: &CommunityEnvelope) {}
    fn my_display_name(&self) -> Option<String> {
        None
    }
    async fn persist_hand_raise(&self, _: String, _: String, _: bool) {}
    fn next_lamport(&self, _: &str) -> Result<u64, LamportError> {
        Ok(1)
    }
    fn stage_speakers(&self, _: &str, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn start_mcu_loop(&self) {}
    async fn stop_mcu_loop(&self) {}
    fn emit_event(&self, _: CommunityVoiceEvent) {}
    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        Arc::clone(&self.scope)
    }
}

fn net() -> Net {
    Arc::new(Mutex::new(HashMap::new()))
}

/// A joiner is called with our current key, opens it and acknowledges;
/// one call is enough.
#[tokio::test]
async fn joiner_receives_and_acknowledges_our_key() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    let ours = alice.sender_keys().send_key(Instant::now());

    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    alice.settle().await;

    assert_eq!(alice.take_calls(), vec![(bob.id(), ours.index)]);
    assert_eq!(alice.held_by(&bob, ours.index), Some(*ours.secret));
}

/// An unacknowledged delivery is retried until it lands.
#[tokio::test(start_paused = true)]
async fn lost_deliveries_are_retried_until_acknowledged() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    alice.drop_calls.store(3, Ordering::SeqCst);
    let ours = alice.sender_keys().send_key(Instant::now());

    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    alice.settle().await;

    assert_eq!(
        alice.take_calls().len(),
        4,
        "three lost, the fourth acknowledged"
    );
    assert_eq!(alice.held_by(&bob, ours.index), Some(*ours.secret));
}

/// Retries end when the recipient leaves our roster.
#[tokio::test(start_paused = true)]
async fn retries_stop_when_the_recipient_leaves() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    alice.roster(&[&bob]).await;
    alice.drop_calls.store(usize::MAX, Ordering::SeqCst);

    on_peer_added(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    tokio::task::yield_now().await;
    alice.transport.lock().await.remove_peer(&bob.id());
    alice.settle().await;

    assert!(alice.take_calls().len() < 3);
}

/// A departure rotates: everyone left is delivered a fresh key, the
/// departed one gets nothing, and its own keys are forgotten.
#[tokio::test]
async fn departure_rotates_to_the_rest() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let carol = Peer::new(&net, 3, false);
    alice.roster(&[&carol]).await;
    let before = alice.sender_keys().send_key(Instant::now());
    alice
        .sender_keys()
        .install(&bob.id(), 7, Zeroizing::new([9; 32]));

    on_peer_removed(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    alice.settle().await;

    let calls = alice.take_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, carol.id());
    assert_ne!(calls[0].1, before.index);
    assert!(alice.held_by(&carol, calls[0].1).is_some());
    assert!(alice.sender_keys().secret_at(&bob.id(), 7).is_err());
}

/// A stage listener's departure does not rotate the speakers' keys.
#[tokio::test]
async fn stage_departure_does_not_rotate() {
    let net = net();
    let alice = Peer::new(&net, 1, true);
    let bob = Peer::new(&net, 2, true);
    alice.roster(&[&bob]).await;
    on_peer_removed(
        &alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        "gone",
        true,
    )
    .await;
    alice.settle().await;
    assert!(alice.take_calls().is_empty());
}

fn key_payload(sender: &Peer, recipient: &str, index: u64, sealed: Vec<u8>) -> ControlPayload {
    ControlPayload::VoiceMediaKey {
        community_id: COMMUNITY.into(),
        channel_id: CHANNEL.into(),
        sender: sender.id(),
        recipient: recipient.into(),
        key_index: index,
        sealed,
    }
}

/// A key sealed to someone else, relabelled with another index, or
/// claimed by another sender is neither installed nor acknowledged.
#[tokio::test]
async fn misdirected_or_rebound_keys_are_refused() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let carol = Peer::new(&net, 3, false);
    let aad = seal_aad(CHANNEL, &bob.id(), 5);
    let sealed = alice
        .seal_media_key(COMMUNITY, &bob.id(), &aad, &[4; 32])
        .unwrap();

    // Sealed to Bob, addressed to Carol.
    let to_carol = key_payload(&alice, &carol.id(), 5, sealed.clone());
    assert!(handle_voice_media_key(&carol.deps(), &to_carol).is_none());
    // Relabelled index: the AAD no longer opens.
    let rebound = key_payload(&alice, &bob.id(), 6, sealed.clone());
    assert!(handle_voice_media_key(&bob.deps(), &rebound).is_none());
    // Claimed to be from Carol: the HPKE Auth sender fails.
    let forged = key_payload(&carol, &bob.id(), 5, sealed.clone());
    assert!(handle_voice_media_key(&bob.deps(), &forged).is_none());
    assert!(bob.sender_keys().secret_at(&carol.id(), 5).is_err());

    // The genuine one installs and is acknowledged for exactly that key.
    let genuine = key_payload(&alice, &bob.id(), 5, sealed);
    let ack = handle_voice_media_key(&bob.deps(), &genuine).unwrap();
    assert!(acknowledges(
        &CommunityEnvelope::Control(genuine),
        Some(&CommunityEnvelope::Control(ack))
    ));
    assert_eq!(alice.held_by(&bob, 5), Some([4; 32]));
}

fn request(requester: &Peer, sender: &str) -> ControlPayload {
    ControlPayload::VoiceMediaKeyRequest {
        community_id: COMMUNITY.into(),
        channel_id: CHANNEL.into(),
        requester: requester.id(),
        sender: sender.into(),
        key_index: 0,
    }
}

/// A request is answered only for a requester on our roster, and only
/// when it asks for our key.
#[tokio::test]
async fn key_requests_are_answered_only_for_the_roster() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let mallory = Peer::new(&net, 4, false);
    alice.roster(&[&bob]).await;

    handle_voice_media_key_request(&alice.deps(), &request(&mallory, &alice.id()));
    handle_voice_media_key_request(&alice.deps(), &request(&bob, &alice.id()));
    handle_voice_media_key_request(&alice.deps(), &request(&bob, &mallory.id()));
    alice.settle().await;

    let calls = alice.take_calls();
    assert!(!calls.is_empty());
    assert!(calls.iter().all(|(recipient, _)| *recipient == bob.id()));
    let ours = alice.sender_keys().send_key(Instant::now());
    assert_eq!(alice.held_by(&bob, ours.index), Some(*ours.secret));
}

/// A member who leaves the community is dropped from the call we share
/// and our key rotates past it, even though media liveness would keep it
/// on the roster through presence reconcile.
#[tokio::test]
async fn community_departure_drops_the_member_and_rotates() {
    let net = net();
    let alice = Peer::new(&net, 1, false);
    let bob = Peer::new(&net, 2, false);
    let carol = Peer::new(&net, 3, false);
    alice.roster(&[&bob, &carol]).await;
    let before = alice.sender_keys().send_key(Instant::now());

    crate::signaling::member_departed(&alice.deps(), COMMUNITY, &bob.id());
    alice.settle().await;

    assert_eq!(alice.transport.lock().await.peer_keys(), vec![carol.id()]);
    let calls = alice.take_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, carol.id());
    assert_ne!(calls[0].1, before.index);
}

/// Hosts route on `is_voice_signaling`: the handshake legs must reach the
/// dispatcher. Media keys are not gossip and must not.
#[test]
fn hosts_route_the_handshake_and_not_the_keys() {
    let handshake = [
        ControlPayload::VoiceJoinAck {
            channel_id: CHANNEL.into(),
            joiner_pseudonym: "j".into(),
            display_name: None,
            route_blob: vec![1],
        },
        ControlPayload::VoiceJoinConfirmed {
            channel_id: CHANNEL.into(),
        },
    ];
    for payload in handshake {
        assert!(
            crate::signaling::is_voice_signaling(&payload),
            "{payload:?}"
        );
    }
    let net = net();
    let alice = Peer::new(&net, 1, false);
    for payload in [
        key_payload(&alice, "r", 1, vec![0; 80]),
        request(&alice, "s"),
    ] {
        assert!(
            !crate::signaling::is_voice_signaling(&payload),
            "{payload:?}"
        );
    }
}
