//! Phase 19.i-REDO — thin facade.
//!
//! `forward_message` lives in `rekindle_channel::pipeline`.
//! This module constructs a `ChannelAdapter` per call and:
//! - re-exports `SendChannelMessageResult` for the command callers
//! - (a send, and every post's settle and Retry, are the Host's:
//!   `rekindle_node::host::adapters::channel`, plan E3.1 E2b)
//! - (the local echo of a sent message is emitted by `rekindle_channel`)
//! - delegates `enforce_slowmode` + `resolve_outbound_mentions` to
//!   crate primitives via the adapter (files_adapter still calls these)

use crate::state::SharedState;
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
