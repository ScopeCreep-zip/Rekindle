//! Phase 23.C — audit-log read orchestration lifted from
//! `commands/community/audit.rs`. DHT inspect of the occupied subkeys →
//! the governance runtime's verified, overflow-following read →
//! flatten into `AuditLogEntryInfoDto` rows → sort + paginate.

use rekindle_types::permissions;

use crate::commands::community::helpers::require_permission;
use crate::commands::community::types::AuditLogEntryInfoDto;
use crate::state::SharedState;
use crate::state_helpers;

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

    // The occupied subkeys, network-confirmed. A failed inspect is an
    // error: there is no blind 0..255 scan (plan C7.5).
    let report = state_helpers::record_pool(state)?
        .inspect_once(
            &gov_key_str,
            None,
            rekindle_records::port::ReportScope::UpdateGet,
        )
        .await
        .map_err(|e| format!("audit log inspect failed: {e}"))?;
    let occupied_subkeys: Vec<u32> = report
        .subkeys
        .iter()
        .zip(&report.network_seqs)
        .filter(|(_, seq)| seq.is_some())
        .map(|(subkey, _)| *subkey)
        .collect();

    // The governance runtime's reader: W26-verifies every payload and
    // follows each author's overflow chain, every page verified against the
    // author that pointed at it, through the record pool.
    let adapter = state_helpers::host_governance(state)?;
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
