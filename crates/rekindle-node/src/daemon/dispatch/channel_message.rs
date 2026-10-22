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
    if reply_to.is_some() {
        return IpcResponse::error(400, "replies are sent with threads (plan E3.1 E3)");
    }
    let membership = match ctx.resolve_community(community) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let Some((_, channel_id)) = resolve_channel(ctx, &membership.governance_key, channel) else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    post(ctx, &membership.governance_key, &channel_id, body).await
}

/// Post `body` to `channel` (a channel id; plan E3.1 E2b).
pub(crate) async fn handle_post(
    ctx: &DaemonContext,
    state: DaemonState,
    channel: &str,
    body: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let Some(community_id) =
        crate::host::adapters::governance::channel_messages::community_of_channel(ctx, channel)
    else {
        return IpcResponse::error(404, format!("channel '{channel}' not found"));
    };
    post(ctx, &community_id, channel, body).await
}

/// The Host's channel send, on both modes: the post's state and id.
async fn post(
    ctx: &DaemonContext,
    community_id: &str,
    channel_id: &str,
    body: &str,
) -> IpcResponse {
    let channel = crate::host::adapters::channel::HostChannel::new(ctx);
    match channel.send(community_id, channel_id, body).await {
        Ok(sent) => {
            ctx.communities
                .mark_channel_synced(community_id, channel_id);
            IpcResponse::ok(&serde_json::json!({
                "status": sent.status,
                "messageId": sent.message_id,
                "timestamp": sent.timestamp_ms,
            }))
        }
        Err(e) => crate::host::adapters::channel::refusal(&e),
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
