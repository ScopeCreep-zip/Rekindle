use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::state::AppState;
use crate::state_helpers;

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct BootstrapMemberEntry {
    pub pseudonym_key: String,
    pub status: String,
    pub route_blob: Vec<u8>,
    pub last_seen: u64,
}

#[derive(Debug, Clone)]
pub(super) struct BootstrapBundle {
    pub member_list: Vec<BootstrapMemberEntry>,
    pub governance_entry_count: usize,
    pub channel_mek_count: usize,
    pub recent_message_count: usize,
    pub has_wrapped_owner_keypair: bool,
}

pub(super) async fn fetch_bootstrap_bundle(
    state: &Arc<AppState>,
    governance_key: &str,
    inviter_route_blob: &[u8],
    joiner_pseudonym: &str,
) -> Result<BootstrapBundle, String> {
    let route_id = state_helpers::import_route_blob(state, inviter_route_blob)?;
    let rc = state_helpers::safe_routing_context(state).ok_or("Veilid node not attached")?;
    let request = CommunityEnvelope::Control(ControlPayload::BootstrapRequest {
        joiner_pseudonym: joiner_pseudonym.to_string(),
        governance_key: governance_key.to_string(),
    });
    let request_bytes =
        serde_json::to_vec(&request).map_err(|e| format!("serialize bootstrap request: {e}"))?;
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rc.app_call(veilid_core::Target::RouteId(route_id), request_bytes),
    )
    .await
    .map_err(|_| "bootstrap app_call timed out".to_string())?
    .map_err(|e| format!("bootstrap app_call failed: {e}"))?;

    match serde_json::from_slice::<CommunityEnvelope>(&response)
        .map_err(|e| format!("invalid bootstrap response envelope: {e}"))?
    {
        CommunityEnvelope::Control(ControlPayload::BootstrapResponse {
            governance_entries,
            member_list,
            channel_meks,
            recent_messages,
            wrapped_owner_keypair,
        }) => Ok(BootstrapBundle {
            member_list: member_list
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("invalid bootstrap member list: {e}"))?,
            governance_entry_count: governance_entries.len(),
            channel_mek_count: channel_meks.len(),
            recent_message_count: recent_messages.len(),
            has_wrapped_owner_keypair: !wrapped_owner_keypair.is_empty(),
        }),
        _ => Err("unexpected bootstrap response payload".into()),
    }
}
