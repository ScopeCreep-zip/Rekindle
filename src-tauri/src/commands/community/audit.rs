use futures::stream::{FuturesUnordered, StreamExt};
use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::require_permission;
use super::types::AuditLogEntryInfoDto;

/// Get paginated audit log entries for a community.
#[tauri::command]
pub async fn get_audit_log(
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
    community_id: String,
    before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Vec<AuditLogEntryInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_AUDIT_LOG)?;

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
    let rc = state_helpers::routing_context(state.inner()).ok_or("not attached")?;
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
    while let Some(result) = futs.next().await {
        if let Ok(Some(val)) = result {
            if val.data().is_empty() {
                continue;
            }
            if let Ok(payload) = serde_json::from_slice::<
                rekindle_types::governance::GovernanceSubkeyPayload,
            >(val.data())
            {
                // Architecture §26 W26 — only show audit entries from
                // signature-verified subkeys; otherwise the audit log
                // would echo the impersonator's claimed identity.
                let Ok(sig_arr): Result<[u8; 64], _> =
                    payload.signature.as_slice().try_into()
                else {
                    continue;
                };
                if rekindle_secrets::derive::verify_pseudonym_signature(
                    &payload.author_pseudonym.0,
                    &payload.signing_bytes(),
                    &sig_arr,
                )
                .is_err()
                {
                    continue;
                }
                let actor = hex::encode(payload.author_pseudonym.0);
                for entry in payload.entries {
                    rows.push(governance_entry_to_audit_row(&actor, entry));
                }
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

fn governance_entry_to_audit_row(
    actor_pseudonym: &str,
    entry: rekindle_types::governance::GovernanceEntry,
) -> AuditLogEntryInfoDto {
    use rekindle_types::governance::GovernanceEntry;

    let actor_pseudonym = actor_pseudonym.to_string();
    let timestamp = entry.lamport();
    match entry {
        GovernanceEntry::ChannelCreated {
            channel_id,
            name,
            record_key,
            ..
        } => AuditLogEntryInfoDto {
            action: "channel_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(channel_id.0)),
            details: Some(format!("{name} ({record_key})")),
            timestamp,
        },
        GovernanceEntry::ChannelArchived { channel_id, .. } => AuditLogEntryInfoDto {
            action: "channel_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(channel_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::ChannelUpdated {
            channel_id,
            name,
            topic,
            position,
            ..
        } => AuditLogEntryInfoDto {
            action: "channel_updated".into(),
            actor_pseudonym,
            target: Some(hex::encode(channel_id.0)),
            details: Some(
                [
                    name.map(|v| format!("name={v}")),
                    topic.map(|v| format!("topic={v}")),
                    position.map(|v| format!("position={v}")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", "),
            )
            .filter(|s| !s.is_empty()),
            timestamp,
        },
        GovernanceEntry::RoleDefinition {
            role_id,
            name,
            permissions,
            position,
            ..
        } => AuditLogEntryInfoDto {
            action: "role_defined".into(),
            actor_pseudonym,
            target: Some(hex::encode(role_id.0)),
            details: Some(format!("{name}, perms={permissions}, position={position}")),
            timestamp,
        },
        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => AuditLogEntryInfoDto {
            action: "role_assigned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(format!("role={}", hex::encode(role_id.0))),
            timestamp,
        },
        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => AuditLogEntryInfoDto {
            action: "role_unassigned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(format!("role={}", hex::encode(role_id.0))),
            timestamp,
        },
        GovernanceEntry::BanEntry { target, reason, .. } => AuditLogEntryInfoDto {
            action: "member_banned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: reason,
            timestamp,
        },
        GovernanceEntry::UnbanEntry { target, .. } => AuditLogEntryInfoDto {
            action: "member_unbanned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds,
            reason,
            ..
        } => AuditLogEntryInfoDto {
            action: "member_timed_out".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(
                [Some(format!("duration={duration_seconds}s")), reason]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            timestamp,
        },
        GovernanceEntry::RemoveTimeoutEntry { target, .. } => AuditLogEntryInfoDto {
            action: "timeout_removed".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::CommunityMeta {
            name, description, ..
        } => AuditLogEntryInfoDto {
            action: "community_updated".into(),
            actor_pseudonym,
            target: None,
            details: Some(
                [
                    name.map(|v| format!("name={v}")),
                    description.map(|v| format!("description={v}")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", "),
            )
            .filter(|s| !s.is_empty()),
            timestamp,
        },
        GovernanceEntry::CategoryCreated {
            category_id, name, ..
        } => AuditLogEntryInfoDto {
            action: "category_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(category_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::CategoryArchived { category_id, .. } => AuditLogEntryInfoDto {
            action: "category_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(category_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::CategoryUpdated {
            category_id,
            name,
            position,
            ..
        } => AuditLogEntryInfoDto {
            action: "category_updated".into(),
            actor_pseudonym,
            target: Some(hex::encode(category_id.0)),
            details: Some(
                [
                    name.map(|v| format!("name={v}")),
                    position.map(|v| format!("position={v}")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", "),
            )
            .filter(|s| !s.is_empty()),
            timestamp,
        },
        GovernanceEntry::PermissionOverwrite {
            channel_id,
            target_type,
            target_id,
            allow,
            deny,
            ..
        } => AuditLogEntryInfoDto {
            action: "permission_overwrite_set".into(),
            actor_pseudonym,
            target: Some(hex::encode(channel_id.0)),
            details: Some(format!(
                "target_type={target_type}, target_id={target_id}, allow={allow}, deny={deny}"
            )),
            timestamp,
        },
        GovernanceEntry::ThreadCreated {
            thread_id, name, ..
        } => AuditLogEntryInfoDto {
            action: "thread_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::ThreadArchived { thread_id, .. } => AuditLogEntryInfoDto {
            action: "thread_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::EventCreated { event_id, name, .. } => AuditLogEntryInfoDto {
            action: "event_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(event_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::EventArchived { event_id, .. } => AuditLogEntryInfoDto {
            action: "event_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(event_id.0)),
            details: None,
            timestamp,
        },
        other => governance_tail_audit_row(actor_pseudonym, timestamp, other),
    }
}

fn governance_tail_audit_row(
    actor_pseudonym: String,
    timestamp: u64,
    entry: rekindle_types::governance::GovernanceEntry,
) -> AuditLogEntryInfoDto {
    use rekindle_types::governance::GovernanceEntry;

    match entry {
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name,
            kind,
            animated,
            ..
        } => AuditLogEntryInfoDto {
            action: "expression_added".into(),
            actor_pseudonym,
            target: Some(hex::encode(expression_id)),
            details: Some(format!("name={name}, kind={kind}, animated={animated}")),
            timestamp,
        },
        GovernanceEntry::ExpressionRemoved { expression_id, .. } => AuditLogEntryInfoDto {
            action: "expression_removed".into(),
            actor_pseudonym,
            target: Some(hex::encode(expression_id)),
            details: None,
            timestamp,
        },
        GovernanceEntry::OnboardingConfig { .. } => AuditLogEntryInfoDto {
            action: "onboarding_config_updated".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
        GovernanceEntry::WelcomeScreen { .. } => AuditLogEntryInfoDto {
            action: "welcome_screen_updated".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
        GovernanceEntry::AdminDelete {
            message_id, reason, ..
        } => AuditLogEntryInfoDto {
            action: "message_deleted".into(),
            actor_pseudonym,
            target: Some(hex::encode(message_id)),
            details: reason,
            timestamp,
        },
        GovernanceEntry::SegmentAdded {
            segment_index,
            registry_key,
            governance_key,
            ..
        } => AuditLogEntryInfoDto {
            action: "segment_added".into(),
            actor_pseudonym,
            target: Some(segment_index.to_string()),
            details: Some(format!(
                "registry={registry_key}, governance={governance_key}"
            )),
            timestamp,
        },
        GovernanceEntry::AutoModRule { rule_id, name, .. } => AuditLogEntryInfoDto {
            action: "automod_rule_updated".into(),
            actor_pseudonym,
            target: Some(hex::encode(rule_id)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::RoleArchived { role_id, .. } => AuditLogEntryInfoDto {
            action: "role_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(role_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::InviteCreated {
            invite_id,
            code_hash,
            max_uses,
            expires_at,
            ..
        } => invite_created_audit_row(
            actor_pseudonym,
            invite_id,
            &code_hash,
            Some(max_uses),
            expires_at,
            timestamp,
        ),
        GovernanceEntry::InviteRevoked { invite_id, .. } => AuditLogEntryInfoDto {
            action: "invite_revoked".into(),
            actor_pseudonym,
            target: Some(hex::encode(invite_id)),
            details: None,
            timestamp,
        },
        GovernanceEntry::MEKGenerationBump { generation, .. } => AuditLogEntryInfoDto {
            action: "mek_rotated".into(),
            actor_pseudonym,
            target: None,
            details: Some(format!("generation={generation}")),
            timestamp,
        },
        _ => AuditLogEntryInfoDto {
            action: "unknown".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
    }
}

fn invite_created_audit_row(
    actor_pseudonym: String,
    invite_id: [u8; 16],
    code_hash: &str,
    max_uses: Option<u32>,
    expires_at: Option<u64>,
    timestamp: u64,
) -> AuditLogEntryInfoDto {
    AuditLogEntryInfoDto {
        action: "invite_created".into(),
        actor_pseudonym,
        target: Some(hex::encode(invite_id)),
        details: Some(format!(
            "code_hash={code_hash}, max_uses={max_uses:?}, expires_at={expires_at:?}"
        )),
        timestamp,
    }
}
