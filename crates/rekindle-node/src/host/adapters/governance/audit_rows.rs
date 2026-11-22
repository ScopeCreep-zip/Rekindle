//! The audit log's rows (plan E3.1 F4): each governance entry projected
//! onto an [`AuditRow`] by its author, named into a `CommunityAuditEntry`
//! by `audit_page`. Pure, so both hosts and the tests project the same way.

/// One governance entry as an audit row, before the Host names its author
/// and target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuditRow {
    pub action: String,
    /// 64 lowercase hex.
    pub actor_pseudonym: String,
    pub target: Option<String>,
    pub details: Option<String>,
    pub timestamp: u64,
}

pub(super) fn governance_entry_to_audit_row(
    actor_pseudonym: &str,
    lamport: u64,
    entry: rekindle_types::governance::GovernanceEntry,
) -> AuditRow {
    use rekindle_types::governance::GovernanceEntry;

    let actor_pseudonym = actor_pseudonym.to_string();
    let timestamp = lamport;
    match entry {
        GovernanceEntry::ChannelCreated {
            channel_id, name, ..
        } => AuditRow {
            action: "channel_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(channel_id.0)),
            details: Some(name.clone()),
            timestamp,
        },
        GovernanceEntry::ChannelArchived { channel_id, .. } => AuditRow {
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
        } => AuditRow {
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
        } => AuditRow {
            action: "role_defined".into(),
            actor_pseudonym,
            target: Some(hex::encode(role_id.0)),
            details: Some(format!("{name}, perms={permissions}, position={position}")),
            timestamp,
        },
        GovernanceEntry::RoleAssignment {
            target, role_id, ..
        } => AuditRow {
            action: "role_assigned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(format!("role={}", hex::encode(role_id.0))),
            timestamp,
        },
        GovernanceEntry::RoleUnassignment {
            target, role_id, ..
        } => AuditRow {
            action: "role_unassigned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: Some(format!("role={}", hex::encode(role_id.0))),
            timestamp,
        },
        GovernanceEntry::BanEntry { target, reason, .. } => AuditRow {
            action: "member_banned".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: reason,
            timestamp,
        },
        GovernanceEntry::UnbanEntry { target, .. } => AuditRow {
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
        } => AuditRow {
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
        GovernanceEntry::RemoveTimeoutEntry { target, .. } => AuditRow {
            action: "timeout_removed".into(),
            actor_pseudonym,
            target: Some(hex::encode(target.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::CommunityMeta {
            name, description, ..
        } => AuditRow {
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
        } => AuditRow {
            action: "category_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(category_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::CategoryArchived { category_id, .. } => AuditRow {
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
        } => AuditRow {
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
        } => AuditRow {
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
        } => AuditRow {
            action: "thread_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::ThreadArchived { thread_id, .. } => AuditRow {
            action: "thread_archived".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::ThreadUnarchived { thread_id, .. } => AuditRow {
            action: "thread_unarchived".into(),
            actor_pseudonym,
            target: Some(hex::encode(thread_id.0)),
            details: None,
            timestamp,
        },
        GovernanceEntry::EventCreated { event_id, name, .. } => AuditRow {
            action: "event_created".into(),
            actor_pseudonym,
            target: Some(hex::encode(event_id.0)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::EventArchived { event_id, .. } => AuditRow {
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
) -> AuditRow {
    use rekindle_types::governance::GovernanceEntry;

    match entry {
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name,
            kind,
            animated,
            ..
        } => AuditRow {
            action: "expression_added".into(),
            actor_pseudonym,
            target: Some(hex::encode(expression_id)),
            details: Some(format!("name={name}, kind={kind}, animated={animated}")),
            timestamp,
        },
        GovernanceEntry::ExpressionRemoved { expression_id, .. } => AuditRow {
            action: "expression_removed".into(),
            actor_pseudonym,
            target: Some(hex::encode(expression_id)),
            details: None,
            timestamp,
        },
        GovernanceEntry::OnboardingConfig { .. } => AuditRow {
            action: "onboarding_config_updated".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
        GovernanceEntry::WelcomeScreen { .. } => AuditRow {
            action: "welcome_screen_updated".into(),
            actor_pseudonym,
            target: None,
            details: None,
            timestamp,
        },
        GovernanceEntry::AdminDelete {
            message_id, reason, ..
        } => AuditRow {
            action: "message_deleted".into(),
            actor_pseudonym,
            target: Some(hex::encode(message_id)),
            details: reason,
            timestamp,
        },
        GovernanceEntry::SegmentAdded {
            segment_index,
            registry_key,
            anchor,
            ..
        } => AuditRow {
            action: "segment_added".into(),
            actor_pseudonym,
            target: Some(segment_index.to_string()),
            details: Some(format!("registry={registry_key}, governance={anchor}")),
            timestamp,
        },
        GovernanceEntry::AutoModRule { rule_id, name, .. } => AuditRow {
            action: "automod_rule_updated".into(),
            actor_pseudonym,
            target: Some(hex::encode(rule_id)),
            details: Some(name),
            timestamp,
        },
        GovernanceEntry::RoleArchived { role_id, .. } => AuditRow {
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
        GovernanceEntry::InviteRevoked { invite_id, .. } => AuditRow {
            action: "invite_revoked".into(),
            actor_pseudonym,
            target: Some(hex::encode(invite_id)),
            details: None,
            timestamp,
        },
        GovernanceEntry::MEKGenerationBump { generation, .. } => AuditRow {
            action: "mek_rotated".into(),
            actor_pseudonym,
            target: None,
            details: Some(format!("generation={generation}")),
            timestamp,
        },
        _ => AuditRow {
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
) -> AuditRow {
    AuditRow {
        action: "invite_created".into(),
        actor_pseudonym,
        target: Some(hex::encode(invite_id)),
        details: Some(format!(
            "code_hash={code_hash}, max_uses={max_uses:?}, expires_at={expires_at:?}"
        )),
        timestamp,
    }
}
