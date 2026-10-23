//! The apply pipeline (plan E3.2): bounds and permission checks against
//! the merged state, then our signed op is built, stored and published
//! (`op_log`), the state re-merged, and the UI snapshot events emitted.
//! All side effects flow through the `GovernanceRuntimeDeps` trait.

use rekindle_governance::{merge, validate};
use rekindle_secrets::derive;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log;

/// Architecture §6 — classify a governance entry by which UI snapshot
/// it invalidates so we can emit the right event after a successful
/// CRDT apply. `Roles` triggers a `RolesChanged` snapshot;
/// `ChannelsOrCategories` triggers `ChannelsUpdated`. Returning both
/// false is intentional for entries that touch nothing the UI caches
/// (e.g. lifecycle events handled elsewhere).
#[derive(Default, Clone, Copy)]
struct EntryAffects {
    roles: bool,
    channels_or_categories: bool,
}

fn classify_entry(entry: &GovernanceEntry) -> EntryAffects {
    match entry {
        // Role definition + archive change the role list itself;
        // assignment/unassignment change a member's role_ids and are
        // handled by `MemberRolesChanged` elsewhere.
        GovernanceEntry::RoleDefinition { .. } | GovernanceEntry::RoleArchived { .. } => {
            EntryAffects {
                roles: true,
                ..EntryAffects::default()
            }
        }
        GovernanceEntry::ChannelCreated { .. }
        | GovernanceEntry::ChannelArchived { .. }
        | GovernanceEntry::ChannelUpdated { .. }
        | GovernanceEntry::CategoryCreated { .. }
        | GovernanceEntry::CategoryArchived { .. }
        | GovernanceEntry::CategoryUpdated { .. } => EntryAffects {
            channels_or_categories: true,
            ..EntryAffects::default()
        },
        _ => EntryAffects::default(),
    }
}

/// Apply a governance entry as our next signed op: validate it against the
/// merged state, store it in `governance_ops`, re-merge, emit the UI
/// events, and publish it on the governance-op outbox stream, returning
/// once it is on the DHT.
pub async fn write_entry<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<(), GovernanceRuntimeError> {
    refuse_out_of_bounds(&entry)?;

    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;

    let my_pseudo_hex = membership
        .my_pseudonym_hex
        .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.to_string()))?;
    let pseudo_bytes: [u8; 32] = hex::decode(&my_pseudo_hex)
        .map_err(|e| GovernanceRuntimeError::InvalidPseudonymHex(e.to_string()))?
        .try_into()
        .map_err(|_| {
            GovernanceRuntimeError::InvalidPseudonymHex("pseudonym must be 32 bytes".into())
        })?;
    let pseudo = PseudonymKey(pseudo_bytes);

    refuse_invalid(&pseudo, &entry, &gov_state)?;

    // The pinned GenesisCert names the creator; S5 writes it at origin
    // and S6 pins it at join (`GovernanceRuntimeDeps::genesis_cert`).
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_string()))?;
    let identity_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let key = derive::derive_community_pseudonym(&identity_secret, community_id);
    let set = op_log::verify_held(community_id, &deps.governance_ops(community_id).await?);
    let (op, encoded) = op_log::build_op(
        community_id,
        &cert,
        &gov_state,
        &set,
        &key,
        entry.clone(),
        deps.clock().now_secs(),
    )?;
    // Our member record M(p), where our op log lives: the cert's for the
    // creator, our `MemberJoined` for a joiner (S6).
    let record = op_log::author_record(&cert, &gov_state, &pseudo)
        .ok_or(GovernanceRuntimeError::MemberRecordMissing(my_pseudo_hex))?;
    deps.store_governance_op(
        community_id,
        &op_log::held(&op, record, encoded.clone(), op_log::OpFate::Applied),
    )
    .await?;

    // The state every reader merges from the op set with this op in it.
    let mut ops = set.ops;
    ops.push(op);
    deps.set_governance_state(community_id, merge::merge(&cert, &ops));

    deps.emit_event(GovernanceRuntimeEvent::GovernanceEntryApplied {
        community_id: community_id.to_string(),
        entry: Box::new(entry.clone()),
    });

    let affects = classify_entry(&entry);
    if affects.roles {
        deps.emit_event(GovernanceRuntimeEvent::RolesChanged {
            community_id: community_id.to_string(),
        });
    }
    if affects.channels_or_categories {
        deps.emit_event(GovernanceRuntimeEvent::ChannelsUpdated {
            community_id: community_id.to_string(),
        });
    }

    // The op is stored and applied here; it is written only once it is on
    // our `M`/2 and `M`/3, which a joiner's walk reads with nobody online.
    // A failed attempt stays queued for the outbox's retry and is an error,
    // so nothing that needs the op on the DHT (an invite link) is handed out.
    deps.publish_governance_op(community_id, encoded).await
}

/// Every reader drops an entry outside its value bounds (plan E3.0), so the
/// writer refuses it by name before validating permission, instead of
/// writing what no peer applies or reporting `PermissionDenied`.
fn refuse_out_of_bounds(entry: &GovernanceEntry) -> Result<(), GovernanceRuntimeError> {
    match validate::bounds::out_of_bounds_field(entry) {
        Some(field) => Err(GovernanceRuntimeError::OutOfBounds { field }),
        None => Ok(()),
    }
}

/// Every reader runs `validate_write` against its merged state before it
/// applies an entry, so the writer runs it before writing: a malformed
/// entry is `Malformed` (400), a missing permission `PermissionDenied`
/// (403), and neither is written or applied.
fn refuse_invalid(
    writer: &PseudonymKey,
    entry: &GovernanceEntry,
    state: &rekindle_governance::state::GovernanceState,
) -> Result<(), GovernanceRuntimeError> {
    if !validate::well_formed(writer, entry, state) {
        return Err(GovernanceRuntimeError::Malformed);
    }
    if !validate::validate_write(writer, entry, state) {
        return Err(GovernanceRuntimeError::PermissionDenied);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::ChannelId;

    #[test]
    fn classify_role_definition_marks_roles() {
        let entry = GovernanceEntry::RoleDefinition {
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            name: "test".into(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        };
        let affects = classify_entry(&entry);
        assert!(affects.roles);
        assert!(!affects.channels_or_categories);
    }

    #[test]
    fn classify_role_archived_marks_roles() {
        let entry = GovernanceEntry::RoleArchived {
            role_id: rekindle_types::id::RoleId([0u8; 16]),
        };
        let affects = classify_entry(&entry);
        assert!(affects.roles);
        assert!(!affects.channels_or_categories);
    }

    #[test]
    fn classify_channel_created_marks_channels() {
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([0u8; 16]),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(affects.channels_or_categories);
    }

    #[test]
    fn classify_category_updated_marks_channels() {
        let entry = GovernanceEntry::CategoryUpdated {
            category_id: rekindle_types::id::CategoryId([0u8; 16]),
            name: Some("renamed".into()),
            position: None,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(affects.channels_or_categories);
    }

    #[test]
    fn out_of_bounds_entries_are_refused_by_field() {
        use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
        use rekindle_types::key_format::MAX_DESCRIPTION_LEN;
        use rekindle_types::member::MAX_TIMEOUT_SECONDS;
        let slowmode = |seconds| GovernanceEntry::ChannelUpdated {
            channel_id: ChannelId([0u8; 16]),
            name: None,
            topic: None,
            forum_tags: None,
            position: None,
            slowmode_seconds: Some(seconds),
            nsfw: None,
            category_id: None,
        };
        let timeout = |duration_seconds| GovernanceEntry::TimeoutEntry {
            target: PseudonymKey([0u8; 32]),
            duration_seconds,
            reason: None,
            started_at: 0,
        };
        let meta = |description: String| GovernanceEntry::CommunityMeta {
            name: None,
            description: Some(description),
            icon_hash: None,
            banner_hash: None,
        };
        let field = |entry: &GovernanceEntry| match refuse_out_of_bounds(entry) {
            Err(GovernanceRuntimeError::OutOfBounds { field }) => Some(field),
            Err(other) => panic!("unexpected {other}"),
            Ok(()) => None,
        };
        assert_eq!(
            field(&slowmode(MAX_SLOWMODE_SECONDS + 1)),
            Some("slowmode_seconds")
        );
        assert_eq!(field(&slowmode(MAX_SLOWMODE_SECONDS)), None);
        assert_eq!(field(&timeout(0)), Some("duration_seconds"));
        assert_eq!(
            field(&timeout(MAX_TIMEOUT_SECONDS + 1)),
            Some("duration_seconds")
        );
        assert_eq!(field(&timeout(MAX_TIMEOUT_SECONDS)), None);
        let long = "x".repeat(MAX_DESCRIPTION_LEN + 1);
        assert_eq!(field(&meta(long)), Some("description"));
        assert_eq!(field(&meta("ok\n".into())), None);
    }

    /// `write_entry` refuses before touching anything: the mock has no
    /// expectations, so any state lookup or DHT call would panic.
    #[tokio::test]
    async fn write_entry_returns_out_of_bounds_before_any_io() {
        let deps = crate::deps::MockGovernanceRuntimeDeps::new();
        let entry = GovernanceEntry::TimeoutEntry {
            target: PseudonymKey([0u8; 32]),
            duration_seconds: 0,
            reason: None,
            started_at: 0,
        };
        let refused = write_entry(&deps, "community", entry).await;
        assert!(matches!(
            refused,
            Err(GovernanceRuntimeError::OutOfBounds {
                field: "duration_seconds"
            })
        ));
    }

    #[test]
    fn classify_role_assignment_affects_nothing() {
        let entry = GovernanceEntry::RoleAssignment {
            target: PseudonymKey([0u8; 32]),
            role_id: rekindle_types::id::RoleId([0u8; 16]),
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(!affects.channels_or_categories);
    }
}
