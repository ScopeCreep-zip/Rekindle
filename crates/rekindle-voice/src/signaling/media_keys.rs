//! Distributing call-media sender keys over voice signaling (plan C7.20).
//!
//! Every participant keys its own media
//! (`rekindle_secrets::media_sender_key::keyring`) and sends
//! the key, sealed, to each other participant it holds on its roster:
//!
//! - when it adds a participant (join apply or the joiner's ack): our
//!   current key to them, or a rotated one to everyone
//!   ([`ChannelSenderKeys::on_join`]);
//! - when a participant leaves a voice channel: a fresh key to everyone
//!   left ([`ChannelSenderKeys::on_leave`]);
//! - when asked (`VoiceMediaKeyRequest`): the current and pending keys to
//!   the asker.
//!
//! Stage channels (§10.7: anyone may listen) do not rotate on a listener's
//! join or leave; a joiner still gets every speaker's current key.
//!
//! Keys travel as `ControlPayload::VoiceMediaKey`, sent to the channel
//! roster (ttl = 0) and opened only by their recipient.

use std::sync::Arc;
use std::time::Instant;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::media_sender_key::seal_aad;

use crate::signaling::deps::VoiceSignalingDeps;
use crate::transport::VoiceTransport;
use rekindle_secrets::media_sender_key::keyring::{JoinShare, OwnKey};

/// Send `key` sealed to `recipient` (pseudonym hex).
fn send_key(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    recipient: &str,
    key: &OwnKey,
) {
    let aad = seal_aad(channel_id, recipient, key.index);
    let Some(sealed) = deps.seal_media_key(community_id, recipient, &aad, &key.secret) else {
        tracing::warn!(
            community = %community_id,
            channel = %channel_id,
            recipient,
            "media key not sealed: pseudonym or recipient key unusable",
        );
        return;
    };
    deps.send_to_channel(
        community_id,
        channel_id,
        &CommunityEnvelope::Control(ControlPayload::VoiceMediaKey {
            channel_id: channel_id.to_string(),
            recipient: recipient.to_string(),
            key_index: key.index,
            sealed,
        }),
    );
}

/// Send `key` to every participant on the roster.
async fn send_key_to_roster(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: &tokio::sync::Mutex<VoiceTransport>,
    key: &OwnKey,
) {
    let peers = transport.lock().await.peer_keys();
    for peer in peers {
        send_key(deps, community_id, channel_id, &peer, key);
    }
}

/// We added `joiner` to our roster: it gets our key.
pub(in crate::signaling) async fn on_peer_added(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: &tokio::sync::Mutex<VoiceTransport>,
    joiner: &str,
    is_stage: bool,
) {
    let keys = deps.channel_sender_keys(community_id, channel_id);
    let now = Instant::now();
    if is_stage {
        send_key(deps, community_id, channel_id, joiner, &keys.send_key(now));
        return;
    }
    match keys.on_join(now) {
        JoinShare::Current(key) => send_key(deps, community_id, channel_id, joiner, &key),
        JoinShare::Rotated(key) => {
            send_key_to_roster(deps, community_id, channel_id, transport, &key).await;
        }
    }
}

/// `departed` left the channel (already off our roster): rotate so it
/// cannot read what follows, and forget its keys.
pub(in crate::signaling) async fn on_peer_removed(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    transport: &tokio::sync::Mutex<VoiceTransport>,
    departed: &str,
    is_stage: bool,
) {
    let keys = deps.channel_sender_keys(community_id, channel_id);
    keys.forget(departed);
    if is_stage {
        return;
    }
    let key = keys.on_leave(Instant::now());
    send_key_to_roster(deps, community_id, channel_id, transport, &key).await;
}

/// A `VoiceMediaKey` from `sender`: install it if it is sealed to us.
pub(in crate::signaling) fn handle_voice_media_key(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    sender: &str,
    channel_id: &str,
    recipient: &str,
    key_index: u64,
    sealed: &[u8],
) {
    let Some(me) = deps.my_pseudonym(community_id) else {
        return;
    };
    if recipient != me || !deps.voice_engine_bound_to(community_id, channel_id) {
        return;
    }
    let aad = seal_aad(channel_id, recipient, key_index);
    let Some(secret) = deps.open_media_key(community_id, sender, &aad, sealed) else {
        tracing::warn!(
            community = %community_id,
            channel = %channel_id,
            sender,
            key_index,
            "media key did not open: not sealed to us by its sender",
        );
        return;
    };
    deps.channel_sender_keys(community_id, channel_id)
        .install(sender, key_index, secret);
    tracing::debug!(community = %community_id, channel = %channel_id, sender, key_index,
        "installed a sender's media key");
}

/// A `VoiceMediaKeyRequest` from `requester` for `sender`'s key: answer
/// if it is ours and the requester is on our roster.
pub(in crate::signaling) fn handle_voice_media_key_request(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    requester: &str,
    channel_id: &str,
    sender: &str,
) {
    let Some(me) = deps.my_pseudonym(community_id) else {
        return;
    };
    if sender != me || !deps.voice_engine_bound_to(community_id, channel_id) {
        return;
    }
    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let deps_task = Arc::clone(deps);
    let cid = community_id.to_string();
    let ch = channel_id.to_string();
    let requester = requester.to_string();
    deps.scope()
        .spawn_or_drop("voice media key answer", async move {
            // Only a participant we hold on the roster gets our key.
            if !transport.lock().await.peer_keys().contains(&requester) {
                return;
            }
            for key in deps_task.channel_sender_keys(&cid, &ch).shareable() {
                send_key(&*deps_task, &cid, &ch, &requester, &key);
            }
        });
}

#[cfg(test)]
mod tests;
