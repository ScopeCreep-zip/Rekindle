//! Media-key delivery on the outbox (plans C7.22, E2.5): every key we send
//! is a row of the host's `media_key` stream of its (channel session,
//! recipient), its payload the `VoiceMediaKey` envelope already sealed to
//! the recipient's session key and signed, never the key itself. The
//! outbox owns the retries: each attempt ([`attempt_media_key`]) settles
//! the row unsent once the key is no longer current or the recipient left
//! our roster, and otherwise calls the recipient's roster route, whose
//! `VoiceMediaKeyAck` settles it. The session's end
//! ([`end_media_key_session`]) discards its rows.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::capnp_envelope::try_decode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeyStore;

use crate::signaling::deps::VoiceSignalingDeps;

/// The host's `media_key` streams.
#[async_trait]
pub trait MediaKeyOutbox: Send + Sync {
    /// Queue `sealed` (an encoded, sealed `VoiceMediaKey` envelope) on the
    /// stream of (`session`, `recipient`) as `client_id`.
    ///
    /// # Errors
    /// Nothing was queued (no unlocked outbox, the storage failed, or
    /// `client_id` is already queued).
    async fn queue(
        &self,
        session: &str,
        recipient: &str,
        client_id: &str,
        sealed: Vec<u8>,
    ) -> Result<(), String>;

    /// `session` ended: discard every key still queued for it; how many.
    ///
    /// # Errors
    /// No unlocked outbox, or the storage failed.
    async fn discard_session(&self, session: &str) -> Result<usize, String>;
}

/// The session a channel's media-key streams belong to.
#[must_use]
pub fn session_scope(community_id: &str, channel_id: &str) -> String {
    format!("{community_id}/{channel_id}")
}

/// How one attempt of a queued key went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKeyAttempt {
    /// The recipient acknowledged exactly this key.
    Acknowledged,
    /// No acknowledgement; the outbox tries again.
    Unacknowledged,
    /// Not sent, and never will be: a later rotation replaced the key, the
    /// recipient left our roster, or we left the channel.
    Superseded,
}

/// Whether `reply` acknowledges exactly the key `envelope` carried.
pub(crate) fn acknowledges(
    envelope: &CommunityEnvelope,
    reply: Option<&CommunityEnvelope>,
) -> bool {
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

/// Attempt the queued key `sealed` once: send it to its recipient's
/// roster route if the key is still ours to share.
pub async fn attempt_media_key(
    deps: &Arc<dyn VoiceSignalingDeps>,
    sealed: &[u8],
) -> MediaKeyAttempt {
    let Ok(Some(envelope)) = try_decode_community_envelope(sealed) else {
        tracing::warn!("unreadable media key row; settled unsent");
        return MediaKeyAttempt::Superseded;
    };
    let CommunityEnvelope::Control(ControlPayload::VoiceMediaKey {
        community_id,
        channel_id,
        recipient,
        key_index,
        ..
    }) = &envelope
    else {
        tracing::warn!("a media key row that is not a media key; settled unsent");
        return MediaKeyAttempt::Superseded;
    };
    // Our session on the channel ended (its rows go with it).
    if !deps.voice_engine_bound_to(community_id, channel_id) {
        return MediaKeyAttempt::Superseded;
    }
    let Some(transport) = deps.transport_handle() else {
        return MediaKeyAttempt::Superseded;
    };
    let route = (transport.lock().await.peer_entries().into_iter())
        .find_map(|(peer, route)| (peer == *recipient).then_some(route));
    // Off our roster: it left, and a departure rotates past this key.
    let Some(route) = route else {
        return MediaKeyAttempt::Superseded;
    };
    // Replaced by a later rotation, which queued its own deliveries.
    if !(deps.channel_sender_keys(community_id, channel_id)).still_current(*key_index) {
        return MediaKeyAttempt::Superseded;
    }
    let reply = deps.call_peer(&route, &envelope).await;
    if acknowledges(&envelope, reply.as_ref()) {
        tracing::debug!(community = %community_id, channel = %channel_id,
            recipient = %recipient, key_index, "media key delivered");
        MediaKeyAttempt::Acknowledged
    } else {
        MediaKeyAttempt::Unacknowledged
    }
}

/// End our session on `(community, channel)`: its keys go (a rejoin draws
/// new ones, plan C7.20) and so does every key still queued for it.
pub async fn end_media_key_session(
    keys: &ChannelSenderKeyStore,
    outbox: &dyn MediaKeyOutbox,
    community_id: &str,
    channel_id: &str,
) {
    keys.end(community_id, channel_id);
    match outbox
        .discard_session(&session_scope(community_id, channel_id))
        .await
    {
        Ok(0) => {}
        Ok(count) => tracing::debug!(community = %community_id, channel = %channel_id, count,
            "undelivered media keys discarded with the session"),
        Err(error) => tracing::warn!(%error, "discarding the session's media keys failed"),
    }
}
