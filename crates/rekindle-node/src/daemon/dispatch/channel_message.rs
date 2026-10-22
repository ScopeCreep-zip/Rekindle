//! Channel message handlers: Send, History, and the writer slots a read covers.

use std::collections::HashMap;
use std::sync::Arc;

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use super::channel::resolve_channel;
use super::{state_error, DaemonContext};

pub(crate) async fn handle_send(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    body: &str,
    reply_to: Option<u64>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // Resolve the channel from merged governance rather than a DHT
    // round trip through `community_detail`.
    let Some((_, channel_id)) = resolve_channel(ctx, &membership.governance_key, channel) else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };

    // Resolve where this write lands. For segment 0 that is the record
    // key `ChannelCreated` carries; beyond it, the first writer from our
    // segment creates the record lazily and announces it with
    // `ChannelSegmentLinked`. Either way the key is merged governance
    // state — nothing has to be registered with anyone.
    let adapter = super::adapter(ctx);
    let channel_record_key = match rekindle_governance_runtime::ensure_channel_segment_record(
        &adapter,
        &membership.governance_key,
        &channel_id,
    )
    .await
    {
        Ok(key) => key,
        Err(e) => return IpcResponse::error(500, format!("no channel record to write to: {e}")),
    };

    // Our writer credential is derived from the shared slot seed, not
    // stored: the seed is the durable secret and the keypair is a pure
    // function of it plus our slot. This is the same derivation the
    // presence write uses, so a member who can publish presence can
    // publish messages.
    let Some(slot_seed) = membership.slot_seed else {
        return IpcResponse::error(
            500,
            "no slot seed for this community — cannot derive a channel writer key",
        );
    };
    let slot_keypair_str =
        match rekindle_crypto::keypair_text::slot_keypair_text(&slot_seed, membership.slot_index) {
            Ok(kp) => kp,
            Err(e) => return IpcResponse::error(500, format!("derive slot keypair: {e}")),
        };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    // W26 authorship: the entry is signed with our community pseudonym,
    // which is what stops a peer holding any slot keypair from forging
    // another member's messages.
    let pseudonym_signing_key = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(
        &signing_key,
        &membership.governance_key,
    );

    let target = rekindle_transport::operations::channel::ChannelWriteTarget {
        channel_record_key,
        slot_index: membership.slot_index,
        slot_keypair_str,
    };

    // The message clock is the mesh clock inbound gossip merges into, so
    // this message orders after everything we have received.
    let lamport_ts = {
        let subscriptions = ctx.subscriptions.read();
        match subscriptions
            .as_ref()
            .map(|s| s.next_message_lamport(&membership.governance_key))
        {
            Some(Ok(lamport)) => lamport,
            Some(Err(e)) => return IpcResponse::error(500, format!("message clock: {e}")),
            None => return IpcResponse::error(503, "subscriptions not running"),
        }
    };

    match rekindle_transport::operations::channel::send_message(
        &transport,
        &membership,
        &channel_id,
        body,
        reply_to,
        lamport_ts,
        &*crate::daemon::mek_rotation::key_provider(ctx),
        &target,
        &pseudonym_signing_key,
        ctx.clock.now_ms(),
    )
    .await
    {
        Ok(sent) => {
            ctx.communities
                .mark_channel_synced(&membership.governance_key, &channel_id);

            // PATH 2 — gossip the notification. The SMPL write above is
            // PATH 1 and authoritative; this is what gives online peers
            // the message in 50–150ms instead of waiting for a watch to
            // fire or the 60s inspect to notice. The desktop's pipeline
            // has always done this; the daemon wrote the record and told
            // nobody, so a daemon member's messages appeared to peers
            // only on their next history read.
            //
            // Carries no ciphertext, deliberately — gossip is
            // unencrypted at the envelope layer, so the notification
            // names the message and peers read the body from the record.
            let notification =
                rekindle_codec::community::envelope::CommunityEnvelope::MessageNotification {
                    channel_id: channel_id.clone(),
                    message_id: sent.message_id.clone(),
                    author_pseudonym: membership.pseudonym_key.clone(),
                    subkey_index: membership.slot_index,
                    lamport_ts: sent.lamport_ts,
                    sequence: sent.sequence,
                    content_hash: sent.content_hash.clone(),
                    timestamp: sent.timestamp,
                };
            crate::daemon::gossip::send(&ctx.gossip_tx, &membership.governance_key, &notification);

            IpcResponse::ok(&serde_json::json!({
                "message_id": sent.message_id,
                "timestamp": sent.timestamp,
                "channel_record_key": sent.channel_record_key,
                // false: held by the record pool until it lands (C7.13).
                "stored": sent.stored,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("send failed: {e}")),
    }
}

pub(crate) async fn handle_history(
    ctx: &DaemonContext,
    state: DaemonState,
    community: &str,
    channel: &str,
    limit: u32,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    let Some((_, channel_id)) = resolve_channel(ctx, &membership.governance_key, channel) else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    let channel_id = channel_id.as_str();
    // Every segment record holding this channel's messages, straight
    // out of merged governance. Replaces walking the registry's member
    // index for per-member log keys.
    let adapter = super::adapter(ctx);
    let record_keys = rekindle_governance_runtime::channel_record_keys_per_segment(
        &adapter,
        &membership.governance_key,
        channel_id,
    );
    // Author names come from the roster the presence poll materialised.
    let display_names: HashMap<String, String> = ctx
        .communities
        .members(&membership.governance_key)
        .into_iter()
        // A member who never set a name has no row to contribute; the
        // reader falls back to their pseudonym rather than rendering an
        // empty author.
        .filter_map(|(pseudonym, record)| record.display_name.map(|name| (pseudonym, name)))
        .collect();
    // The writer index: every member the roster placed, and ourselves
    // (plan C7.12).
    let writers = writer_slots(ctx, &membership);
    match query
        .channel_history(
            &membership.governance_key,
            channel_id,
            &record_keys,
            &writers,
            &display_names,
            limit as usize,
            &*crate::daemon::mek_rotation::key_provider(ctx),
        )
        .await
    {
        Ok(messages) => IpcResponse::ok(&messages),
        Err(e) => IpcResponse::error(500, format!("channel history: {e}")),
    }
}

/// `(segment_index, slot)` for every member the presence roster placed in
/// `membership`'s community, and our own: the slots a channel read covers
/// (plan C7.12).
pub(crate) fn writer_slots(
    ctx: &DaemonContext,
    membership: &rekindle_transport::session::CommunityMembership,
) -> Vec<(u32, u32)> {
    let mut writers: Vec<(u32, u32)> = ctx
        .communities
        .members(&membership.governance_key)
        .into_values()
        .map(|member| (member.segment_index, member.subkey_index))
        .collect();
    writers.push((membership.segment_index.unwrap_or(0), membership.slot_index));
    writers.sort_unstable();
    writers.dedup();
    writers
}

/// Edit or delete our message `message_id` in `channel` (plan E3.1 E2).
pub(crate) async fn handle_change(
    ctx: &DaemonContext,
    state: DaemonState,
    channel: &str,
    message_id: &str,
    change: crate::host::adapters::governance::channel_messages::MessageChange,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let Some(community_id) =
        crate::host::adapters::governance::channel_messages::community_of_channel(ctx, channel)
    else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    match super::adapter(ctx)
        .change_channel_message(&community_id, channel, message_id, change)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::Value::Null),
        Err(e) => IpcResponse::error(500, format!("channel message change failed: {e}")),
    }
}
