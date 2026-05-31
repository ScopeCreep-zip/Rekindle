//! `attempt_pending_retry` body — parses the row's JSON body as
//! either a DM `MessageEnvelope` or `PendingChannelMessage` and
//! dispatches via the appropriate transport. Lifted out of
//! `deps_impl.rs` so each trait method body stays one-liner thin.
//!
//! Returns the [`PendingRetryOutcome`] verbatim — the orchestrator
//! decides what to do with it (`Delivered` → delete row, `Failed`
//! → bump retry, `Unrecognized` → drop).

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessageEnvelope;
use rekindle_sync::{PendingMessageRow, PendingRetryOutcome};

use crate::state::AppState;
use crate::state_helpers;

pub(super) async fn attempt_pending_retry(
    state: &Arc<AppState>,
    row: &PendingMessageRow,
) -> PendingRetryOutcome {
    if let Ok(envelope) = serde_json::from_str::<MessageEnvelope>(&row.body) {
        return attempt_dm_retry(state, &row.recipient_key, &envelope).await;
    }
    if let Ok(channel_msg) =
        serde_json::from_str::<crate::services::community::PendingChannelMessage>(&row.body)
    {
        return attempt_channel_retry(state, &channel_msg).await;
    }
    PendingRetryOutcome::Unrecognized
}

/// Retry one pending DM envelope. Mirrors pre-port
/// `retry_pending_dm`: import the cached route, fall back to a
/// mailbox-DHT read on miss, send via `messaging::sender::send_envelope`.
async fn attempt_dm_retry(
    state: &Arc<AppState>,
    recipient_key: &str,
    envelope: &MessageEnvelope,
) -> PendingRetryOutcome {
    let route_id_and_rc = state_helpers::try_import_peer_route(state, recipient_key);
    if route_id_and_rc.is_none() && state_helpers::safe_api_and_routing_context(state).is_none() {
        // Not attached yet — bump retry, try again next tick.
        return PendingRetryOutcome::Failed;
    }
    let route_id_and_rc = if route_id_and_rc.is_some() {
        route_id_and_rc
    } else {
        try_mailbox_route_fallback(state, recipient_key).await
    };
    let Some((route_id, routing_context)) = route_id_and_rc else {
        return PendingRetryOutcome::Failed;
    };
    match rekindle_protocol::messaging::sender::send_envelope(&routing_context, route_id, envelope)
        .await
    {
        Ok(()) => {
            tracing::debug!(to = %recipient_key, "pending DM delivered successfully");
            PendingRetryOutcome::Delivered
        }
        Err(error) => {
            tracing::debug!(to = %recipient_key, %error, "pending DM retry failed");
            PendingRetryOutcome::Failed
        }
    }
}

/// Retry one pending channel message via SMPL DHT write. Mirrors
/// pre-port `retry_pending_channel_message`.
async fn attempt_channel_retry(
    state: &Arc<AppState>,
    channel_msg: &crate::services::community::PendingChannelMessage,
) -> PendingRetryOutcome {
    let lookup = {
        let communities = state.communities.read();
        let Some(community) = communities.get(&channel_msg.community_id) else {
            return PendingRetryOutcome::Failed;
        };
        let Some(channel_key) = community
            .channel_log_keys
            .get(&channel_msg.channel_id)
            .cloned()
        else {
            return PendingRetryOutcome::Failed;
        };
        let Some(slot_keypair_str) = community.slot_keypair.clone() else {
            return PendingRetryOutcome::Failed;
        };
        let Some(slot_index) = community.my_subkey_index else {
            return PendingRetryOutcome::Failed;
        };
        (channel_key, slot_keypair_str, slot_index)
    };
    let (channel_key, slot_keypair_str, slot_index) = lookup;

    let Some(rc) = state_helpers::safe_routing_context(state) else {
        return PendingRetryOutcome::Failed;
    };
    let Ok(writer) = slot_keypair_str.parse::<veilid_core::KeyPair>() else {
        return PendingRetryOutcome::Failed;
    };
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let channel_record_message =
        rekindle_protocol::dht::community::channel_record::ChannelMessage {
            sequence: channel_msg.sequence,
            sender_pseudonym: channel_msg.author_pseudonym.clone(),
            ciphertext: channel_msg.ciphertext.clone(),
            mek_generation: channel_msg.mek_generation,
            timestamp: channel_msg.timestamp.cast_unsigned(),
            reply_to: None,
            lamport_ts: channel_msg.lamport_ts,
            message_id: Some(channel_msg.message_id.clone()),
            attachment: None,
            flags: channel_msg.mention_flag_bits,
            mentioned_pseudonyms: channel_msg.mentioned_pseudonyms.clone(),
            mentioned_roles: channel_msg.mentioned_roles.clone(),
        };
    let (author_pseudo, signing_key) =
        match state_helpers::pseudonym_credentials(state, &channel_msg.community_id) {
            Ok(creds) => creds,
            Err(error) => {
                tracing::debug!(%error, "pending channel retry: no pseudonym credentials");
                return PendingRetryOutcome::Failed;
            }
        };
    if let Err(error) = rekindle_protocol::dht::community::channel_record::write_member_message(
        &mgr,
        &channel_key,
        slot_index,
        writer,
        author_pseudo,
        &signing_key,
        &channel_record_message,
    )
    .await
    {
        tracing::debug!(%error, "pending channel message retry failed");
        return PendingRetryOutcome::Failed;
    }
    // Architecture §28.2 — also fire the gossip MessageNotification
    // so peers reading the registry hear about the delivery
    // immediately, not just on next SMPL read.
    let _ = crate::services::community::send_to_mesh(
        state,
        &channel_msg.community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::MessageNotification {
            channel_id: channel_msg.channel_id.clone(),
            message_id: channel_msg.message_id.clone(),
            author_pseudonym: channel_msg.author_pseudonym.clone(),
            subkey_index: channel_msg.subkey_index,
            lamport_ts: channel_msg.lamport_ts,
            sequence: channel_msg.sequence,
            content_hash: channel_msg.content_hash.clone(),
            timestamp: channel_msg.timestamp.cast_unsigned(),
        },
    );
    tracing::debug!("pending channel message delivered");
    PendingRetryOutcome::Delivered
}

async fn try_mailbox_route_fallback(
    state: &Arc<AppState>,
    recipient_key: &str,
) -> Option<(veilid_core::RouteId, veilid_core::RoutingContext)> {
    let mailbox_key = state_helpers::friend_mailbox_key(state, recipient_key)?;
    let rc = state_helpers::safe_routing_context(state)?;
    let route_blob =
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(&rc, &mailbox_key).await {
            Ok(Some(blob)) if !blob.is_empty() => blob,
            Ok(_) => return None,
            Err(error) => {
                tracing::trace!(to = %recipient_key, %error, "failed to read mailbox");
                return None;
            }
        };
    state_helpers::cache_peer_route(state, recipient_key, route_blob.clone());
    match state_helpers::import_route_blob(state, &route_blob) {
        Ok(route_id) => {
            tracing::debug!(to = %recipient_key, "discovered route via mailbox fallback");
            Some((route_id, rc))
        }
        Err(error) => {
            tracing::trace!(to = %recipient_key, %error, "failed to import mailbox route");
            None
        }
    }
}
