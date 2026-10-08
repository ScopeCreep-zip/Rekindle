//! Distributing call-media sender keys (plans C7.20, C7.22).
//!
//! Every participant keys its own media
//! (`rekindle_secrets::media_sender_key::keyring`) and sends the key,
//! sealed, to each other participant it holds on its roster:
//!
//! - when it adds a participant (join apply, the joiner's ack, a roster):
//!   our current key to them, or a rotated one to everyone
//!   ([`ChannelSenderKeys::on_join`]);
//! - when a participant leaves a voice channel: a fresh key to everyone
//!   left ([`ChannelSenderKeys::on_leave`]);
//! - when asked (`VoiceMediaKeyRequest`): the current and pending keys to
//!   the asker.
//!
//! Stage channels (§10.7: anyone may listen) do not rotate on a listener's
//! join or leave; a joiner still gets every speaker's current key.
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

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::media_sender_key::keyring::{JoinShare, OwnKey};
use rekindle_secrets::media_sender_key::seal_aad;

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

async fn deliver(
    deps: &Arc<dyn VoiceSignalingDeps>,
    transport: &tokio::sync::Mutex<VoiceTransport>,
    d: &Delivery,
) {
    let Some(sender) = deps.my_pseudonym(&d.community_id) else {
        return;
    };
    let aad = seal_aad(&d.channel_id, &d.recipient, d.key.index);
    let Some(sealed) = deps.seal_media_key(&d.community_id, &d.recipient, &aad, &d.key.secret)
    else {
        tracing::warn!(community = %d.community_id, channel = %d.channel_id,
            recipient = %d.recipient, "media key not sealed: pseudonym or recipient key unusable");
        return;
    };
    let envelope = CommunityEnvelope::Control(ControlPayload::VoiceMediaKey {
        community_id: d.community_id.clone(),
        channel_id: d.channel_id.clone(),
        sender,
        recipient: d.recipient.clone(),
        key_index: d.key.index,
        sealed,
    });
    let keys = deps.channel_sender_keys(&d.community_id, &d.channel_id);
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
        if !keys.shareable().iter().any(|k| k.index == d.key.index) {
            return;
        }
        let reply = deps.call_peer(&route, &envelope).await;
        if acknowledges(&envelope, reply.as_ref()) {
            tracing::debug!(community = %d.community_id, channel = %d.channel_id,
                recipient = %d.recipient, key_index = d.key.index, "media key delivered");
            return;
        }
        let Some(wait) = waits.next() else {
            tracing::warn!(community = %d.community_id, channel = %d.channel_id,
                recipient = %d.recipient, key_index = d.key.index,
                "media key not acknowledged; the recipient will ask for it");
            return;
        };
        tokio::time::sleep(*wait).await;
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

/// We added `joiner` to our roster: it gets our key.
pub(in crate::signaling) async fn on_peer_added(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    joiner: &str,
    is_stage: bool,
) {
    let keys = deps.channel_sender_keys(community_id, channel_id);
    let now = Instant::now();
    if is_stage {
        spawn_delivery(
            deps,
            community_id,
            channel_id,
            transport,
            joiner,
            keys.send_key(now),
        );
        return;
    }
    match keys.on_join(now) {
        JoinShare::Current(key) => {
            spawn_delivery(deps, community_id, channel_id, transport, joiner, key);
        }
        JoinShare::Rotated(key) => {
            deliver_to_roster(deps, community_id, channel_id, transport, &key).await;
        }
    }
}

/// `departed` left the channel (already off our roster): rotate so it
/// cannot read what follows, and forget its keys.
pub(in crate::signaling) async fn on_peer_removed(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
    departed: &str,
    is_stage: bool,
) {
    let keys = deps.channel_sender_keys(community_id, channel_id);
    keys.forget(departed);
    if is_stage {
        return;
    }
    let key = keys.on_leave(Instant::now());
    deliver_to_roster(deps, community_id, channel_id, transport, &key).await;
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
    } = payload
    else {
        return None;
    };
    let me = deps.my_pseudonym(community_id)?;
    if *recipient != me || !deps.voice_engine_bound_to(community_id, channel_id) {
        return None;
    }
    let aad = seal_aad(channel_id, recipient, *key_index);
    let Some(secret) = deps.open_media_key(community_id, sender, &aad, sealed) else {
        tracing::warn!(community = %community_id, channel = %channel_id, sender, key_index,
            "media key did not open: not sealed to us by its sender");
        return None;
    };
    deps.channel_sender_keys(community_id, channel_id)
        .install(sender, *key_index, secret);
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
/// is on our roster, deliver our current and pending keys to it.
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
            for key in deps_task.channel_sender_keys(&cid, &ch).shareable() {
                spawn_delivery(&deps_task, &cid, &ch, &transport, &requester, key);
            }
        });
}

#[cfg(test)]
mod tests;
