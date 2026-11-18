//! The audit log's rows (plan E3.1 F4): each governance entry projected
//! onto a [`CommunityAuditEntry`] by its author. Pure, so both hosts and the
//! tests project the same way.

use rekindle_types::dto::community::CommunityAuditEntry;

pub(super) fn governance_entry_to_audit_row(
    actor_pseudonym: &str,
    lamport: u64,
    entry: rekindle_types::governance::GovernanceEntry,
) -> CommunityAuditEntry {
    use rekindle_types::governance::GovernanceEntry;

    let actor_pseudonym = actor_pseudonym.to_string();
    let timestamp = lamport;
    match entry {
        GovernanceEntry::ChannelCreated {
            channel_id,
            name,
            record_key,
            ..
        } => CommunityAuditEntry {
            action: "channel_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(channel_id.0)),
            details: Some(format!("{name} ({record_key})")),
            timestamp,
        },
        GovernanceEntry::ChannelArchived { channel_id, .. } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
            action: "role_defined".into(),
            actor_pseudonym,
            target: Some(hex::encode(role_id.0)),
            details: Some(format!("{name}, perms={permissions}, position={position}")),
            timestamp,
        },
        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => CommunityAuditEntry {
            action: "role_assigned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(format!("role={}", hex::encode(role_id.0))),
            timestamp,
        },
        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => CommunityAuditEntry {
            action: "role_unassigned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(format!("role={}", hex::encode(role_id.0))),
            timestamp,
        },
        GovernanceEntry::BanEntry { target, reason, .. } => CommunityAuditEntry {
            action: "member_banned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: reason,
            timestamp,
        },
        GovernanceEntry::UnbanEntry { target, .. } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
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
        GovernanceEntry::RemoveTimeoutEntry { target, .. } => CommunityAuditEntry {
            action: "timeout_removed".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::CommunityMeta {
            name, description, ..
        } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
            action: "category_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(category_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::CategoryArchived { category_id, .. } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
            action: "thread_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::ThreadArchived { thread_id, .. } => CommunityAuditEntry {
            action: "thread_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::ThreadUnarchived { thread_id, .. } => CommunityAuditEntry {
            action: "thread_unarchived".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::EventCreated { event_id, name, .. } => CommunityAuditEntry {
            action: "event_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(event_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::EventArchived { event_id, .. } => CommunityAuditEntry {
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
) -> CommunityAuditEntry {
    use rekindle_types::governance::GovernanceEntry;

    match entry {
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name,
            kind,
            animated,
            ..
        } => CommunityAuditEntry {
            action: "expression_added".into(),
            actor_pseudonym,
            target: Some(hex::encode(expression_id)),
            details: Some(format!("name={name}, kind={kind}, animated={animated}")),
            timestamp,
        },
        GovernanceEntry::ExpressionRemoved { expression_id, .. } => CommunityAuditEntry {
            action: "expression_removed".into(),
            actor_pseudonym,
            target: Some(hex::encode(expression_id)),
            details: None,
            timestamp,
        },
        GovernanceEntry::OnboardingConfig { .. } => CommunityAuditEntry {
            action: "onboarding_config_updated".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
        GovernanceEntry::WelcomeScreen { .. } => CommunityAuditEntry {
            action: "welcome_screen_updated".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
        GovernanceEntry::AdminDelete {
            message_id, reason, ..
        } => CommunityAuditEntry {
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
        } => CommunityAuditEntry {
            action: "segment_added".into(),
            actor_pseudonym,
            target: Some(segment_index.to_string()),
            details: Some(format!(
                "registry={registry_key}, governance={governance_key}"
            )),
            timestamp,
        },
        GovernanceEntry::AutoModRule { rule_id, name, .. } => CommunityAuditEntry {
            action: "automod_rule_updated".into(),
            actor_pseudonym,
            target: Some(hex::encode(rule_id)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::RoleArchived { role_id, .. } => CommunityAuditEntry {
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
        GovernanceEntry::InviteRevoked { invite_id, .. } => CommunityAuditEntry {
            action: "invite_revoked".into(),
            actor_pseudonym,
            target: Some(hex::encode(invite_id)),
            details: None,
            timestamp,
        },
        GovernanceEntry::MEKGenerationBump { generation, .. } => CommunityAuditEntry {
            action: "mek_rotated".into(),
            actor_pseudonym,
            target: None,
            details: Some(format!("generation={generation}")),
            timestamp,
        },
        _ => CommunityAuditEntry {
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
) -> CommunityAuditEntry {
    CommunityAuditEntry {
        action: "invite_created".into(),
        actor_pseudonym,
        target: Some(hex::encode(invite_id)),
        details: Some(format!(
            "code_hash={code_hash}, max_uses={max_uses:?}, expires_at={expires_at:?}"
        )),
        timestamp,
    }
}
