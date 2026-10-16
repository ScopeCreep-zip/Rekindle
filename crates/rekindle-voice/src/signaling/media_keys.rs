//! Distributing call-media sender keys (plans C7.20, C7.22).
//!
//! Every participant keys its own media
//! (`rekindle_secrets::media_sender_key::keyring`) and sends the key,
//! sealed, to each other participant it holds on its roster:
//!
//! - when it adds a participant (join apply, the joiner's ack, a roster):
//!   our key ratchets a step and the joiner gets the new step, so it opens
//!   nothing sealed before it joined ([`ChannelSenderKeys::on_join`],
//!   plan C7.29);
//! - when a participant leaves a voice channel: a new generation to
//!   everyone left ([`ChannelSenderKeys::on_leave`]); a leave while one is
//!   pending starts the next when the pending one is applied;
//! - when asked (`VoiceMediaKeyRequest`): our keys from the asker's
//!   admission on, never an older one.
//!
//! Stage channels (§10.7: anyone may listen) do not rotate when a listener
//! leaves on its own; a member leaving the community (kicked, banned or
//! gone) always rotates ([`Departure::LeftCommunity`]).
//!
//! **Sealed to a session key** (plan C7.29): each participant announces an
//! X25519 key per channel session in its join or ack, signed by its
//! pseudonym; our keys are sealed to it (HPKE Base) and signed, and stop
//! opening once that session ends.
//!
//! **Delivery is acknowledged, never gossiped.** Every comparable system
//! sends media keys over its reliable messaging channel, not the media
//! path, and knows who has them: MatrixRTC marks a member `sharedWith`
//! only once its to-device send succeeds, RingRTC sends over Signal
//! messages and can `resend_media_keys`, Jitsi answers each `key-info`
//! with a `key-info-ack`. Here a key goes by `app_call` to the
//! recipient's roster route (the MEK transfer's path) and the recipient
//! replies `VoiceMediaKeyAck` once it opened and installed it. Unacked
//! deliveries retry with backoff while the recipient is on our roster and
//! the key is still ours to share. A recipient that still lacks a key —
//! it restarted, or every attempt was lost — asks with
//! `VoiceMediaKeyRequest`, also by `app_call`, so neither direction passes
//! the gossip layer's content dedup.
//!
//! [`ChannelSenderKeys::on_join`]: rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys::on_join
//! [`ChannelSenderKeys::on_leave`]: rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys::on_leave

use std::sync::Arc;
use std::time::{Duration, Instant};

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MediaSessionKey};
use rekindle_secrets::media_sender_key::keyring::{
    ChannelSenderKeys, LeaveRotation, OwnKey, SignedSessionKey,
};
use rekindle_secrets::media_sender_key::{seal_aad, session};

use crate::signaling::deps::VoiceSignalingDeps;
use crate::transport::VoiceTransport;

/// Waits before each delivery attempt after the first: about 30 s in all,
/// past which the recipient's own request takes over.
const DELIVERY_BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
];

/// One delivery: `key` to `recipient` in a channel.
struct Delivery {
    community_id: String,
    channel_id: String,
    recipient: String,
    key: OwnKey,
}

/// Deliver `key` to `recipient` in the background, retrying until it is
/// acknowledged, the recipient leaves our roster, or the key is no
/// longer ours to share.
fn spawn_delivery(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    recipient: &str,
    key: OwnKey,
) {
    let delivery = Delivery {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        recipient: recipient.to_string(),
        key,
    };
    let deps_task = Arc::clone(deps);
    let transport = Arc::clone(transport);
    deps.scope()
        .spawn_or_drop("voice media key delivery", async move {
            deliver(&deps_task, &transport, &delivery).await;
        });
}

/// Whether `reply` acknowledges exactly the key `envelope` carried.
fn acknowledges(envelope: &CommunityEnvelope, reply: Option<&CommunityEnvelope>) -> bool {
    let (
        CommunityEnvelope::Control(ControlPayload::VoiceMediaKey {
            channel_id,
            sender,
            recipient,
            key_index,
            ..
        }),
        Some(CommunityEnvelope::Control(ControlPayload::VoiceMediaKeyAck {
            channel_id: acked_channel,
            sender: acked_sender,
            recipient: acker,
            key_index: acked_index,
            ..
        })),
    ) = (envelope, reply)
    else {
        return false;
    };
    acked_channel == channel_id
        && acked_sender == sender
        && acker == recipient
        && acked_index == key_index
}

/// The `VoiceMediaKey` envelope for `d`: our key sealed to the
/// recipient's session key and signed, or `None` while we hold no session
/// key for the recipient (its join or ack brings one).
fn sealed_envelope(
    deps: &Arc<dyn VoiceSignalingDeps>,
    keys: &ChannelSenderKeys,
    d: &Delivery,
    sender: &str,
) -> Option<CommunityEnvelope> {
    let aad = seal_aad(&d.channel_id, &d.recipient, d.key.index);
    let sealed = keys.seal_for(&d.recipient, &aad, &d.key.secret)?;
    let signature = deps.sign_sealed_media_key(
        &d.community_id,
        &d.channel_id,
        &d.recipient,
        d.key.index,
        &sealed,
    )?;
    Some(CommunityEnvelope::Control(ControlPayload::VoiceMediaKey {
        community_id: d.community_id.clone(),
        channel_id: d.channel_id.clone(),
        sender: sender.to_string(),
        recipient: d.recipient.clone(),
        key_index: d.key.index,
        sealed,
        signature,
    }))
}

async fn deliver(
    deps: &Arc<dyn VoiceSignalingDeps>,
    transport: &tokio::sync::Mutex<VoiceTransport>,
    d: &Delivery,
) {
    let Some(sender) = deps.my_pseudonym(&d.community_id) else {
        return;
    };
    let keys = deps.channel_sender_keys(&d.community_id, &d.channel_id);
    let mut envelope = None;
    let mut waits = DELIVERY_BACKOFF.iter();
    loop {
        let route = {
            let t = transport.lock().await;
            t.peer_entries()
                .into_iter()
                .find_map(|(peer, route)| (peer == d.recipient).then_some(route))
        };
        // Off our roster: it left, and a departure rotates past this key.
        let Some(route) = route else { return };
        // Superseded by a later rotation, which has its own deliveries.
        if !keys.still_current(d.key.index) {
            return;
        }
        if envelope.is_none() {
            envelope = sealed_envelope(deps, &keys, d, &sender);
        }
        if let Some(envelope) = &envelope {
            let reply = deps.call_peer(&route, envelope).await;
            if acknowledges(envelope, reply.as_ref()) {
                tracing::debug!(community = %d.community_id, channel = %d.channel_id,
                    recipient = %d.recipient, key_index = d.key.index, "media key delivered");
                return;
            }
        }
        let Some(wait) = waits.next() else {
            tracing::warn!(community = %d.community_id, channel = %d.channel_id,
                recipient = %d.recipient, key_index = d.key.index, sealed = envelope.is_some(),
                "media key not delivered; the recipient will ask for it");
            return;
        };
        tokio::time::sleep(*wait).await;
    }
}

/// Our session key for `(community, channel)` as the join handshake
/// announces it, signed by our pseudonym (plan C7.29). Unsigned (no
/// pseudonym key), receivers refuse it and seal nothing to us.
pub(crate) fn announced_session_key(
    keys: &ChannelSenderKeys,
    sign: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> MediaSessionKey {
    let key = keys.session_public().to_vec();
    MediaSessionKey {
        signature: sign(&key).unwrap_or_default(),
        key,
    }
}

/// Our signed session key for the channel, from the signaling side.
pub(in crate::signaling) fn our_session_key(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
) -> MediaSessionKey {
    announced_session_key(&deps.channel_sender_keys(community_id, channel_id), |key| {
        deps.sign_media_session_key(community_id, channel_id, key)
    })
}

/// `participant`'s session key as a roster relays it.
pub(in crate::signaling) fn relayed_session_key(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    participant: &str,
) -> MediaSessionKey {
    deps.channel_sender_keys(community_id, channel_id)
        .session_key_of(participant)
        .map(|k| MediaSessionKey {
            key: k.key,
            signature: k.signature,
        })
        .unwrap_or_default()
}

/// Learn `participant`'s session key from its join, ack or a roster, if
/// we are in that channel and its pseudonym signed the key.
pub(in crate::signaling) fn learn_session_key(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    participant: &str,
    key: &MediaSessionKey,
) {
    if !deps.voice_engine_bound_to(community_id, channel_id)
        || deps.my_pseudonym(community_id).as_deref() == Some(participant)
    {
        return;
    }
    let signed = SignedSessionKey {
        key: key.key.clone(),
        signature: key.signature.clone(),
    };
    if !deps
        .channel_sender_keys(community_id, channel_id)
        .learn_session_key(community_id, channel_id, participant, &signed)
    {
        tracing::warn!(community = %community_id, channel = %channel_id, participant,
            "media session key refused: not signed by the participant");
    }
}

/// Deliver `key` to every participant on the roster.
async fn deliver_to_roster(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    key: &OwnKey,
) {
    let peers = transport.lock().await.peer_keys();
    for peer in peers {
        spawn_delivery(
            deps,
            community_id,
            channel_id,
            transport,
            &peer,
            key.clone(),
        );
    }
}

/// We added `joiner` to our roster: our key ratchets and it gets the new
/// step (and a new generation goes to everyone when the join exhausted
/// one).
pub(in crate::signaling) async fn on_peer_added(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    joiner: &str,
) {
    let keys = deps.channel_sender_keys(community_id, channel_id);
    let join = keys.on_join(joiner, Instant::now());
    for key in join.joiner {
        spawn_delivery(deps, community_id, channel_id, transport, joiner, key);
    }
    if let Some(key) = join.everyone {
        deliver_to_roster(deps, community_id, channel_id, transport, &key).await;
    }
}

/// How a participant came off our roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Departure {
    /// It left the channel, or its presence expired.
    LeftChannel,
    /// It left the community: kicked, banned or gone.
    LeftCommunity,
}

/// `departed` came off our roster: rotate so it cannot read what follows,
/// and forget its keys. Only a stage listener leaving on its own keeps our
/// key (anyone may listen to a stage, §10.7).
pub(in crate::signaling) async fn on_peer_removed(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    departed: &str,
    is_stage: bool,
    departure: Departure,
) {
    let keys = deps.channel_sender_keys(community_id, channel_id);
    keys.forget(departed);
    if is_stage && departure == Departure::LeftChannel {
        return;
    }
    match keys.on_leave(Instant::now()) {
        LeaveRotation::Rotated(key) => {
            deliver_to_roster(deps, community_id, channel_id, transport, &key).await;
        }
        LeaveRotation::Deferred(at) => {
            spawn_follow_up(deps, community_id, channel_id, transport, at);
        }
    }
}

/// At `at`, when the pending rotation is applied, start the one a leave
/// during its delay asked for, and send it to everyone then on the roster.
fn spawn_follow_up(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    at: Instant,
) {
    let deps_task = Arc::clone(deps);
    let transport = Arc::clone(transport);
    let (cid, ch) = (community_id.to_string(), channel_id.to_string());
    deps.scope()
        .spawn_or_drop("voice media key follow-up rotation", async move {
            tokio::time::sleep_until(at.into()).await;
            let keys = deps_task.channel_sender_keys(&cid, &ch);
            if let Some(key) = keys.follow_up(at) {
                deliver_to_roster(&deps_task, &cid, &ch, &transport, &key).await;
            }
        });
}

/// A `VoiceMediaKey` call: install the key if it is sealed to us by its
/// sender, and return the acknowledgement to reply with.
#[must_use]
pub fn handle_voice_media_key(
    deps: &Arc<dyn VoiceSignalingDeps>,
    payload: &ControlPayload,
) -> Option<ControlPayload> {
    let ControlPayload::VoiceMediaKey {
        community_id,
        channel_id,
        sender,
        recipient,
        key_index,
        sealed,
        signature,
    } = payload
    else {
        return None;
    };
    let me = deps.my_pseudonym(community_id)?;
    if *recipient != me || !deps.voice_engine_bound_to(community_id, channel_id) {
        return None;
    }
    let signed = session::sealed_key_message(channel_id, recipient, *key_index, sealed);
    if !session::verify_pseudonym(sender, &signed, signature) {
        tracing::warn!(community = %community_id, channel = %channel_id, sender, key_index,
            "media key refused: not signed by its sender");
        return None;
    }
    let keys = deps.channel_sender_keys(community_id, channel_id);
    let aad = seal_aad(channel_id, recipient, *key_index);
    let Ok(secret) = keys.open_sealed(&aad, sealed) else {
        tracing::warn!(community = %community_id, channel = %channel_id, sender, key_index,
            "media key did not open: not sealed to our session key");
        return None;
    };
    keys.install(sender, *key_index, secret);
    tracing::info!(community = %community_id, channel = %channel_id, sender, key_index,
        "installed a sender's media key");
    Some(ControlPayload::VoiceMediaKeyAck {
        community_id: community_id.clone(),
        channel_id: channel_id.clone(),
        sender: sender.clone(),
        recipient: me,
        key_index: *key_index,
    })
}

/// A `VoiceMediaKeyRequest` call: if it asks for our key and the requester
/// is on our roster, deliver our keys from its admission on.
pub fn handle_voice_media_key_request(
    deps: &Arc<dyn VoiceSignalingDeps>,
    payload: &ControlPayload,
) {
    let ControlPayload::VoiceMediaKeyRequest {
        community_id,
        channel_id,
        requester,
        sender,
        ..
    } = payload
    else {
        return;
    };
    if deps.my_pseudonym(community_id).as_ref() != Some(sender)
        || !deps.voice_engine_bound_to(community_id, channel_id)
    {
        return;
    }
    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let deps_task = Arc::clone(deps);
    let (cid, ch, requester) = (community_id.clone(), channel_id.clone(), requester.clone());
    deps.scope()
        .spawn_or_drop("voice media key answer", async move {
            // Only a participant we hold on the roster gets our key.
            if !transport.lock().await.peer_keys().contains(&requester) {
                return;
            }
            for key in deps_task
                .channel_sender_keys(&cid, &ch)
                .shareable_for(&requester)
            {
                spawn_delivery(&deps_task, &cid, &ch, &transport, &requester, key);
            }
        });
}

#[cfg(test)]
mod tests;
