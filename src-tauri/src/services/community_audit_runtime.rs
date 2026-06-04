//! Phase 23.C — audit-log read orchestration lifted from
//! `commands/community/audit.rs`. DHT inspect → parallel `get_dht_value`
//! across occupied subkeys → W26 signature verify per subkey payload →
//! flatten into `AuditLogEntryInfoDto` rows → sort + paginate.

use futures::stream::{FuturesUnordered, StreamExt};

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

/// Parse + W26-verify a governance subkey payload, optionally requiring it be
/// authored by `expect_author`. An overflow page must be signed by the same
/// author that pointed at it, else a member could redirect the chain at a
/// record they control.
fn verify_governance_payload(
    bytes: &[u8],
    expect_author: Option<&rekindle_types::id::PseudonymKey>,
) -> Option<rekindle_types::governance::GovernanceSubkeyPayload> {
    let payload =
        serde_json::from_slice::<rekindle_types::governance::GovernanceSubkeyPayload>(bytes)
            .ok()?;
    if let Some(expected) = expect_author {
        if payload.author_pseudonym != *expected {
            return None;
        }
    }
    let sig_arr: [u8; 64] = payload.signature.as_slice().try_into().ok()?;
    rekindle_secrets::derive::verify_pseudonym_signature(
        &payload.author_pseudonym.0,
        &payload.signing_bytes(),
        &sig_arr,
    )
    .ok()?;
    Some(payload)
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
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let gov_key: veilid_core::RecordKey = gov_key_str
        .parse()
        .map_err(|e| format!("invalid governance key: {e}"))?;

    let occupied_subkeys: Vec<u32> = match rc
        .inspect_dht_record(
            gov_key.clone(),
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            veilid_core::DHTReportScope::UpdateGet,
        )
        .await
    {
        Ok(report) => report
            .network_seqs()
            .iter()
            .enumerate()
            .filter(|(_, seq)| **seq != veilid_core::ValueSeqNum::default())
            .map(|(i, _)| u32::try_from(i).unwrap_or(0))
            .collect(),
        Err(e) => {
            tracing::warn!(
                community = %community_id,
                error = %e,
                "audit log inspect failed; scanning known governance subkey range"
            );
            (0..255u32).collect()
        }
    };

    let mut futs = FuturesUnordered::new();
    for subkey in occupied_subkeys {
        let rc = rc.clone();
        let gov_key = gov_key.clone();
        futs.push(async move { rc.get_dht_value(gov_key, subkey, false).await });
    }

    let mut rows = Vec::new();
    // Authors whose primary subkey points into an overflow chain — followed
    // after the primary sweep so the audit view shows complete history
    // (architecture §"Follow GovernanceOverflow pointers", line 1609).
    let mut chains: Vec<(rekindle_types::id::PseudonymKey, String)> = Vec::new();
    while let Some(result) = futs.next().await {
        if let Ok(Some(val)) = result {
            if val.data().is_empty() {
                continue;
            }
            if let Some(payload) = verify_governance_payload(val.data(), None) {
                let author = payload.author_pseudonym.clone();
                let actor = hex::encode(author.0);
                if let Some(next) = payload.overflow_next {
                    chains.push((author, next));
                }
                for entry in payload.entries {
                    rows.push(crate::audit_view::governance_entry_to_audit_row(
                        &actor, entry,
                    ));
                }
            }
        }
    }

    // Follow each author's overflow chain (one page per DFLT record at subkey 0),
    // verifying every page against the author that pointed at it. Shared
    // visited-set cycle guard; per-chain depth cap.
    let mut visited = std::collections::HashSet::new();
    for (author, first_key) in chains {
        let actor = hex::encode(author.0);
        let mut next = Some(first_key);
        let mut depth = 0usize;
        while let Some(key_str) = next.take() {
            if depth >= rekindle_governance_runtime::MAX_OVERFLOW_PAGES
                || !visited.insert(key_str.clone())
            {
                break;
            }
            depth += 1;
            let Ok(rec_key) = key_str.parse::<veilid_core::RecordKey>() else {
                break;
            };
            if rc.open_dht_record(rec_key.clone(), None).await.is_err() {
                break;
            }
            let Ok(Some(val)) = rc.get_dht_value(rec_key, 0, false).await else {
                break;
            };
            if val.data().is_empty() {
                break;
            }
            let Some(payload) = verify_governance_payload(val.data(), Some(&author)) else {
                break;
            };
            next = payload.overflow_next;
            for entry in payload.entries {
                rows.push(crate::audit_view::governance_entry_to_audit_row(
                    &actor, entry,
                ));
            }
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
                    format!("{}…", &pseudonym_key[..12])
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
