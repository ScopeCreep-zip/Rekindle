//! Media-key distribution over voice signaling (plan C7.20), between
//! mock participants that seal and open with the real HPKE construction.

use std::sync::Arc;
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

/// One participant: its pseudonym key, roster, keyring and outbox.
struct Peer {
    signing: SigningKey,
    is_stage: bool,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    keys: ChannelSenderKeyStore,
    sent: Mutex<Vec<CommunityEnvelope>>,
    scope: Arc<rekindle_lifecycle::SessionScope>,
}

impl Peer {
    fn new(seed: u8, is_stage: bool) -> Arc<Self> {
        Arc::new(Self {
            signing: SigningKey::from_bytes(&[seed; 32]),
            is_stage,
            transport: Arc::new(tokio::sync::Mutex::new(VoiceTransport::new(
                CHANNEL.to_string(),
            ))),
            keys: ChannelSenderKeyStore::default(),
            sent: Mutex::new(Vec::new()),
            scope: rekindle_lifecycle::SessionScope::new("test", Arc::new(|_| {})),
        })
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
            t.add_peer(&peer.id(), &[1], None);
        }
    }

    fn sender_keys(&self) -> Arc<ChannelSenderKeys> {
        self.keys.keys(COMMUNITY, CHANNEL)
    }

    /// Take the `VoiceMediaKey`s sent so far as `(recipient, index, sealed)`.
    fn take_keys(&self) -> Vec<(String, u64, Vec<u8>)> {
        self.sent
            .lock()
            .drain(..)
            .filter_map(|envelope| match envelope {
                CommunityEnvelope::Control(ControlPayload::VoiceMediaKey {
                    recipient,
                    key_index,
                    sealed,
                    ..
                }) => Some((recipient, key_index, sealed)),
                _ => None,
            })
            .collect()
    }

    /// Hand every key `from` sent to this peer, as the dispatcher would.
    fn receive(self: &Arc<Self>, from: &Peer, sent: &[(String, u64, Vec<u8>)]) {
        for (recipient, index, sealed) in sent {
            handle_voice_media_key(
                &self.deps(),
                COMMUNITY,
                &from.id(),
                CHANNEL,
                recipient,
                *index,
                sealed,
            );
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
        Some(vec![1])
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
    fn send_to_mesh(&self, _: &str, envelope: &CommunityEnvelope) {
        self.sent.lock().push(envelope.clone());
    }
    fn advertise_media_capabilities(&self, _: &str, _: &str) {}
    fn send_to_channel(&self, _: &str, _: &str, envelope: &CommunityEnvelope) {
        self.sent.lock().push(envelope.clone());
    }
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

/// A joiner gets our current key, sealed to it alone, and can open it.
#[tokio::test]
async fn joiner_receives_and_opens_our_key() {
    let alice = Peer::new(1, false);
    let bob = Peer::new(2, false);
    alice.roster(&[&bob]).await;

    on_peer_added(
        &*alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    let sent = alice.take_keys();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, bob.id());
    bob.receive(&alice, &sent);

    let ours = alice.sender_keys().send_key(Instant::now());
    let theirs = bob
        .sender_keys()
        .secret_at(&alice.id(), ours.index)
        .unwrap();
    assert_eq!(*theirs, *ours.secret);
}

/// A departure rotates: everyone left gets a fresh key, the departed
/// one gets nothing, and its own keys are forgotten.
#[tokio::test]
async fn departure_rotates_to_the_rest() {
    let alice = Peer::new(1, false);
    let bob = Peer::new(2, false);
    let carol = Peer::new(3, false);
    alice.roster(&[&carol]).await;
    let before = alice.sender_keys().send_key(Instant::now());
    alice
        .sender_keys()
        .install(&bob.id(), 7, Zeroizing::new([9; 32]));

    on_peer_removed(
        &*alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    let sent = alice.take_keys();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, carol.id());
    assert_ne!(sent[0].1, before.index);
    assert!(alice.sender_keys().secret_at(&bob.id(), 7).is_err());

    carol.receive(&alice, &sent);
    assert!(carol
        .sender_keys()
        .secret_at(&alice.id(), sent[0].1)
        .is_ok());
}

/// A stage listener's departure does not rotate the speakers' keys.
#[tokio::test]
async fn stage_departure_does_not_rotate() {
    let alice = Peer::new(1, true);
    let bob = Peer::new(2, true);
    alice.roster(&[&bob]).await;
    on_peer_removed(
        &*alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        "gone",
        true,
    )
    .await;
    assert!(alice.take_keys().is_empty());
}

/// A key sealed to someone else, or replayed under another index, is not
/// installed.
#[tokio::test]
async fn misdirected_or_rebound_keys_are_refused() {
    let alice = Peer::new(1, false);
    let bob = Peer::new(2, false);
    let carol = Peer::new(3, false);
    alice.roster(&[&bob]).await;
    on_peer_added(
        &*alice.deps(),
        COMMUNITY,
        CHANNEL,
        &alice.transport,
        &bob.id(),
        false,
    )
    .await;
    let sent = alice.take_keys();
    let (_, index, sealed) = &sent[0];

    // Bob's key, delivered to Carol: not hers.
    carol.receive(&alice, &sent);
    assert!(carol.sender_keys().secret_at(&alice.id(), *index).is_err());

    // Bob's key, relabelled with another index: the AAD no longer opens.
    let rebound = vec![(bob.id(), index + 1, sealed.clone())];
    bob.receive(&alice, &rebound);
    assert!(bob.sender_keys().secret_at(&alice.id(), index + 1).is_err());

    // Bob's key, claimed to be from Carol: the HPKE Auth sender fails.
    handle_voice_media_key(
        &bob.deps(),
        COMMUNITY,
        &carol.id(),
        CHANNEL,
        &bob.id(),
        *index,
        sealed,
    );
    assert!(bob.sender_keys().secret_at(&carol.id(), *index).is_err());
}

/// A request is answered only for a requester on our roster.
#[tokio::test]
async fn key_requests_are_answered_only_for_the_roster() {
    let alice = Peer::new(1, false);
    let bob = Peer::new(2, false);
    let mallory = Peer::new(4, false);
    alice.roster(&[&bob]).await;

    handle_voice_media_key_request(
        &alice.deps(),
        COMMUNITY,
        &mallory.id(),
        CHANNEL,
        &alice.id(),
    );
    handle_voice_media_key_request(&alice.deps(), COMMUNITY, &bob.id(), CHANNEL, &alice.id());
    // A request for someone else's key is not ours to answer.
    handle_voice_media_key_request(&alice.deps(), COMMUNITY, &bob.id(), CHANNEL, &mallory.id());
    alice.scope.close_and_wait().await;

    let sent = alice.take_keys();
    assert!(!sent.is_empty());
    assert!(sent.iter().all(|(recipient, _, _)| *recipient == bob.id()));
    bob.receive(&alice, &sent);
    let ours = alice.sender_keys().send_key(Instant::now());
    assert!(bob.sender_keys().secret_at(&alice.id(), ours.index).is_ok());
}

/// A member who leaves the community is dropped from the call we share
/// and our key rotates past it, even though media liveness would keep it
/// on the roster through presence reconcile.
#[tokio::test]
async fn community_departure_drops_the_member_and_rotates() {
    let alice = Peer::new(1, false);
    let bob = Peer::new(2, false);
    let carol = Peer::new(3, false);
    alice.roster(&[&bob, &carol]).await;
    let before = alice.sender_keys().send_key(Instant::now());

    crate::signaling::member_departed(&alice.deps(), COMMUNITY, &bob.id());
    alice.scope.close_and_wait().await;

    assert_eq!(alice.transport.lock().await.peer_keys(), vec![carol.id()]);
    let sent = alice.take_keys();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, carol.id());
    assert_ne!(sent[0].1, before.index);
}
