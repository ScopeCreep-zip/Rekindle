//! Phase 23.D.6 — thin facade. All automod compile + evaluate logic
//! lives in `rekindle_channel::automod` parameterised over
//! `ChannelMessagingDeps`. Adapter holds the per-community
//! `Arc<AutoModCompiledCache>` keyed by community_id (regex JIT state
//! is expensive — rebuilt only on governance fingerprint mismatch).

use std::sync::Arc;

use crate::services::channel_adapter::ChannelAdapter;
use crate::state::AppState;

pub use rekindle_channel::{AutoModAction, AutoModRuleInfo};

fn build_adapter(state: &Arc<AppState>) -> Result<ChannelAdapter, String> {
    crate::services::build_adapter(state, ChannelAdapter::new)
}

pub fn list_rules(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<Vec<AutoModRuleInfo>, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::list_automod_rules(&adapter, community_id).map_err(|e| e.to_string())
}

pub fn evaluate_message(
    state: &Arc<AppState>,
    community_id: &str,
    body: &str,
) -> Result<AutoModAction, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::evaluate_automod(&adapter, community_id, body).map_err(|e| e.to_string())
}

pub fn get_rule(
    state: &Arc<AppState>,
    community_id: &str,
    rule_id: &[u8; 16],
) -> Result<AutoModRuleInfo, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::get_automod_rule(&adapter, community_id, rule_id).map_err(|e| e.to_string())
}
