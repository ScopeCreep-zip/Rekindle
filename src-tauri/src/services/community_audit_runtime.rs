//! Phase 23.C — audit-log read orchestration lifted from
//! `commands/community/audit.rs`. DHT inspect of the occupied subkeys →
//! the governance runtime's verified, overflow-following read →
//! flatten into `AuditLogEntryInfoDto` rows → sort + paginate.

use rekindle_types::permissions;

use crate::commands::community::helpers::require_permission;
use crate::commands::community::types::AuditLogEntryInfoDto;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedMemberInfo {
    pub pseudonym_key: String,
    pub display_name: String,
    pub banned_at: u64,
    pub reason: Option<String>,
    pub banned_by: String,
}

pub async fn get_audit_log_inner(
    state: &SharedState,
    community_id: String,
    before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Vec<AuditLogEntryInfoDto>, String> {
    require_permission(state, &community_id, permissions::VIEW_AUDIT_LOG)?;

    let gov_key_str = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        community
            .governance_key
            .clone()
            .ok_or("community has no governance key")?
    };
    let gov_key: veilid_core::RecordKey = gov_key_str
        .parse()
        .map_err(|e| format!("invalid governance key: {e}"))?;

    // The occupied subkeys, network-confirmed. A failed inspect is an
    // error: there is no blind 0..255 scan (plan C7.5).
    let occupied_subkeys: Vec<u32> = state_helpers::record_pool(state)?
        .inspect_once(&gov_key, None, veilid_core::DHTReportScope::UpdateGet)
        .await
        .map_err(|e| format!("audit log inspect failed: {e}"))?
        .network_seqs()
        .iter()
        .enumerate()
        .filter(|(_, seq)| seq.to_option().is_some())
        .filter_map(|(i, _)| u32::try_from(i).ok())
        .collect();

    // The governance runtime's reader: W26-verifies every payload and
    // follows each author's overflow chain, every page verified against the
    // author that pointed at it, through the record pool.
    let app_handle = state_helpers::app_handle(state).ok_or("app handle not initialized")?;
    let adapter = crate::services::governance_adapter::GovernanceAdapter::new(
        std::sync::Arc::clone(state),
        app_handle,
        state.db.current()?,
    );
    let readout = rekindle_governance_runtime::read_governance_with_overflow(
        &adapter,
        &gov_key_str,
        &occupied_subkeys,
    )
    .await;
    let mut rows = Vec::new();
    for (author, entries) in readout.authored {
        let actor = hex::encode(author.0);
        for entry in entries {
            rows.push(crate::audit_view::governance_entry_to_audit_row(
                &actor, entry,
            ));
        }
    }

    rows.sort_by(|a, b| {
        b.timestamp
            .cmp(&a.timestamp)
            .then_with(|| a.actor_pseudonym.cmp(&b.actor_pseudonym))
            .then_with(|| a.action.cmp(&b.action))
    });
    if let Some(before) = before_timestamp {
        rows.retain(|row| row.timestamp < before);
    }
    let page_size = usize::try_from(limit.max(1)).unwrap_or(100);
    rows.truncate(page_size);
    Ok(rows)
}

pub fn get_ban_list_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<BannedMemberInfo>, String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let mut bans: Vec<_> = community
        .governance_state
        .as_ref()
        .map(|gov| gov.bans.iter().cloned().collect())
        .unwrap_or_default();
    bans.sort_by_key(|a| hex::encode(a.0));

    Ok(bans
        .into_iter()
        .map(|pseudo| {
            let pseudonym_key = hex::encode(pseudo.0);
            BannedMemberInfo {
                display_name: if pseudonym_key.len() > 12 {
                    format!("{}…", rekindle_utils::text::prefix(&pseudonym_key, 12))
                } else {
                    pseudonym_key.clone()
                },
                pseudonym_key,
                banned_at: 0,
                reason: None,
                banned_by: String::new(),
            }
        })
        .collect())
}
