//! The channel pipeline's slowmode gate and mention resolver for the
//! shell's files adapter, over a `ChannelAdapter` built per call. A send,
//! a forward, and every post's settle and Retry are the Host's
//! (`rekindle_node::host::adapters::channel`, plan E3.1 E2b and E2d).

use crate::state::SharedState;

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::channel_adapter::ChannelAdapter, String> {
    crate::services::build_adapter(state, crate::services::channel_adapter::ChannelAdapter::new)
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
