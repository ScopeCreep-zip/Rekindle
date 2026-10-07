//! Phase 19.i-REDO — thin facade.
//!
//! `send_message` + `forward_message` live in `rekindle_channel::pipeline`.
//! This module constructs a `ChannelAdapter` per call and:
//! - re-exports `SendChannelMessageResult` for the command callers
//! - hosts the delivery-settle task (`start_delivery_settle`): a channel
//!   write that missed consensus is held by the record pool, and when its
//!   slot settles the messages waiting on it are delivered or failed
//!   (plan C7.13; the record pool is the one retry layer)
//! - (the local echo of a sent message is emitted by `rekindle_channel`)
//! - delegates `enforce_slowmode` + `resolve_outbound_mentions` to
//!   crate primitives via the adapter (files_adapter still calls these)

use std::sync::Arc;

use rekindle_lifecycle::{ScopeClosed, SessionScope};

use crate::state::{AppState, SharedState};
use crate::state_helpers;
use rekindle_db::Db;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChannelMessageResult {
    pub status: String,
    pub message_id: String,
}

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::channel_adapter::ChannelAdapter, String> {
    crate::services::build_adapter(state, crate::services::channel_adapter::ChannelAdapter::new)
}

fn map_result(result: rekindle_channel::ChannelSendResult) -> SendChannelMessageResult {
    SendChannelMessageResult {
        status: result.status,
        message_id: result.message_id,
    }
}

pub async fn send_message(
    state: &SharedState,
    _pool: &Db,
    channel_id: &str,
    body: &str,
) -> Result<SendChannelMessageResult, String> {
    let adapter = build_adapter(state)?;
    // Find the community by walking communities for one that owns this channel.
    let community_id = {
        let communities = state.communities.read();
        communities
            .iter()
            .find_map(|(id, community)| {
                if community.channels.iter().any(|ch| ch.id == channel_id) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| "channel not found in any community".to_string())?
    };
    let result = rekindle_channel::send_channel_message(&adapter, &community_id, channel_id, body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(map_result(result))
}

pub async fn forward_message(
    state: &SharedState,
    _pool: &Db,
    source_community_id: &str,
    source_channel_id: &str,
    source_message_id: &str,
    dest_community_id: &str,
    dest_channel_id: &str,
) -> Result<SendChannelMessageResult, String> {
    let adapter = build_adapter(state)?;
    let result = rekindle_channel::forward_channel_message(
        &adapter,
        rekindle_channel::ForwardChannelMessageParams {
            source_community: source_community_id,
            source_channel: source_channel_id,
            source_message: source_message_id,
            dest_community: dest_community_id,
            dest_channel: dest_channel_id,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(map_result(result))
}

/// Architecture §28.7 slowmode gate, retained as a re-export for
/// files_adapter callers. Delegates to the crate's
/// `enforce_slowmode_with_bypass`.
pub(crate) fn enforce_slowmode(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::enforce_slowmode_with_bypass(&adapter, community_id, channel_id, now_ms)
        .map_err(|e| e.to_string())
}

/// Sender-side mention resolver, retained as a re-export for
/// files_adapter callers.
pub(crate) fn resolve_outbound_mentions(
    state: &SharedState,
    community_id: &str,
    sender_pseudonym_hex: &str,
    body: &str,
) -> (Vec<String>, Vec<String>, u32) {
    let Ok(adapter) = build_adapter(state) else {
        return (Vec::new(), Vec::new(), 0);
    };
    rekindle_channel::resolve_outbound_mentions(&adapter, community_id, sender_pseudonym_hex, body)
}

/// Follow the session's record pool for settled writes (plan C7.13): a
/// channel write that missed consensus is held and re-pushed by the pool,
/// and when its slot settles, the messages waiting on it are delivered, or
/// failed when the network superseded our page. Runs on the login scope.
///
/// # Errors
/// [`ScopeClosed`] when the session already ended.
pub fn start_delivery_settle(
    state: Arc<AppState>,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    let Ok(pool) = state_helpers::record_pool(&state) else {
        return Ok(());
    };
    let mut settled = pool.settled();
    scope.spawn_with_token("channel delivery settle", |stop| async move {
        loop {
            let event = match stop.run_until_cancelled(settled.recv()).await {
                None | Some(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return,
                Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(missed))) => {
                    // Their slots settle again on their next write.
                    tracing::warn!(missed, "channel delivery settle lagged");
                    continue;
                }
                Some(Ok(event)) => event,
            };
            let Ok(adapter) = build_adapter(&state) else {
                continue;
            };
            rekindle_channel::on_write_settled(
                &adapter,
                &event.record_key,
                event.subkey,
                event.landed,
            );
        }
    })
}
