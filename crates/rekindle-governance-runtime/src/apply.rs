//! The apply pipeline (plan E3.2): bounds and permission checks against
//! the merged state, then our signed op is built, stored and published
//! (`op_log`), the state re-merged, and the UI snapshot events emitted.
//! All side effects flow through the `GovernanceRuntimeDeps` trait.

use rekindle_governance::{merge, validate};
use rekindle_secrets::derive;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::{GovernanceRuntimeDeps, OpLanding, OpOutcome};
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
        // assignment/unassignment change only the merged role
        // assignments, which every reader derives roles from.
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
/// merged state, store it in `governance_ops`, gossip it, re-merge, emit
/// the UI events and publish it on the governance-op outbox stream,
/// answering after its first attempt or `GOVERNANCE_ANSWER_BOUND`,
/// whichever comes first (plan E3.2 S6b, RV4 review M1/M2): `Landed`, or
/// `Pending` (applied, gossiped, queued; the outbox lands it and the
/// backend announces the landing). Neither is retried by the caller.
pub async fn write_entry<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        entry,
        OpLanding::FirstAttempt,
        op_log::Deps::Frontier,
    )
    .await
}

/// [`write_entry`] citing `extra` among its deps beside the frontier: a
/// transfer acceptance cites the offer it accepts (plan E3.2 S7s.4).
pub async fn write_entry_citing<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
    extra: Vec<rekindle_types::governance::OpHash>,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        entry,
        OpLanding::FirstAttempt,
        op_log::Deps::FrontierAnd(extra),
    )
    .await
}

/// [`write_entry`] for an invite's `InviteCreated` (of `invite_id`),
/// answering only once the op landed on `M`/2 with our frontier on `M`/3,
/// else [`GovernanceRuntimeError::NotYetPublished`] with the op queued: a
/// joiner must find it on the DHT before we hand the link out (plan E3.2
/// S6 (4)). Its revoke is queued with it, in one transaction, and goes
/// unless the link is handed out (review RV4 M3, [`OpLanding::InviteLanded`]).
pub async fn write_invite_created<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
    invite_id: [u8; 16],
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        entry,
        OpLanding::InviteLanded { invite_id },
        op_log::Deps::Frontier,
    )
    .await
}

/// [`write_entry`], returning once the op is queued on our stream, behind
/// our earlier ops: the governance-op outbox stream owns its delivery
/// (plan E3.2 S6b). For a writer that must not wait on the network: the
/// ingest loop (onboarding answers), the MEK rotation worker, and an
/// invite refused "not yet published" queueing its `InviteRevoked`.
pub async fn write_entry_queued<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        entry,
        OpLanding::Queued,
        op_log::Deps::Frontier,
    )
    .await
}

/// Our `Checkpoint` op (plan E3.2 S7c.4), queued like
/// [`write_entry_queued`]: its deps are its cut's heads and our previous
/// op, so whoever cites it has the whole cut in its causal past (S7c.3).
pub async fn write_checkpoint_queued<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    checkpoint: rekindle_types::governance::Checkpoint,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    let heads = checkpoint.cut.iter().map(|r| r.op_hash).collect();
    write(
        deps,
        community_id,
        GovernanceEntry::Checkpoint(checkpoint),
        OpLanding::Queued,
        op_log::Deps::Cited(heads),
    )
    .await
}

/// When [`write_with`] answers that our own join is not in the merged
/// state.
#[derive(Clone, Copy)]
enum NotAdmitted {
    /// After the entry's validation, as any op's missing author record.
    Refuse,
    /// Before it: our leave of a join never admitted withdraws, whatever
    /// the entry's validation would say.
    Withdraw,
}

/// What [`write_with`] did.
enum Written {
    /// Our op was applied and published.
    Op(OpOutcome),
    /// Our join (pseudonym hex) was never admitted: nothing was authored.
    NotAdmitted(String),
}

/// Our `MemberLeft`, unless the merged state never admitted our join:
/// then nothing is authored and [`LeaveOutcome::Withdrawn`] answers (plan
/// SU1, the withdrawal decision; ADR 0011: membership is the merged
/// state's alone).
///
/// Admission is decided first, in a locked state read, before our slot
/// heads are read from the DHT: a withdrawn join has no heads to record,
/// so the read would be wasted. A join admitted there is decided again in
/// the locked state the op is built on ([`write_with`]), so a leave never
/// authors on a state that does not admit us.
///
/// [`LeaveOutcome::Withdrawn`]: crate::moderation::LeaveOutcome::Withdrawn
pub(crate) async fn write_departure<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<crate::moderation::LeaveOutcome, GovernanceRuntimeError> {
    use crate::moderation::LeaveOutcome;
    let me = {
        let lock = deps.rebuild_lock(community_id);
        let _installing = lock.lock().await;
        let gov_state = deps.governance_state(community_id).ok_or_else(|| {
            GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string())
        })?;
        op_log::own_member(deps, community_id, &gov_state)?
    };
    if me.record.is_none() {
        return Ok(LeaveOutcome::Withdrawn);
    }
    let heads = deps.departed_slot_heads(community_id, &me.hex).await?;
    let written = write_with(
        deps,
        community_id,
        GovernanceEntry::MemberLeft { heads },
        OpLanding::FirstAttempt,
        op_log::Deps::Frontier,
        NotAdmitted::Withdraw,
    )
    .await?;
    Ok(match written {
        Written::Op(_) => LeaveOutcome::Left,
        Written::NotAdmitted(_) => LeaveOutcome::Withdrawn,
    })
}

async fn write<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
    landing: OpLanding,
    op_deps: op_log::Deps,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    let refuse = NotAdmitted::Refuse;
    match write_with(deps, community_id, entry, landing, op_deps, refuse).await? {
        Written::Op(outcome) => Ok(outcome),
        Written::NotAdmitted(hex) => Err(GovernanceRuntimeError::MemberRecordMissing(hex)),
    }
}

async fn write_with<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
    landing: OpLanding,
    op_deps: op_log::Deps,
    not_admitted: NotAdmitted,
) -> Result<Written, GovernanceRuntimeError> {
    refuse_out_of_bounds(&entry)?;

    // Serialized with the rebuild (`dht_hydration`) from the state and
    // op-set reads to the install (D15: the state is a pure function of a
    // grow-only op set, so whoever reads last installs last; register S7l,
    // HIGH-2). Never held across the publish, and no rebuild runs under it.
    let lock = deps.rebuild_lock(community_id);
    let installing = lock.lock().await;

    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    // Our member record M(p), where our op log lives: the cert's for the
    // creator, our `MemberJoined` for a joiner (S6), in the state this
    // write builds on. A join the merge never admitted has none: its
    // leave withdraws before the entry's validation, authoring nothing
    // (plan SU1); any other op is validated first (a malformed entry is
    // refused as such, F3h), then refused for the missing record.
    if matches!(not_admitted, NotAdmitted::Withdraw) {
        let me = op_log::own_member(deps, community_id, &gov_state)?;
        if me.record.is_none() {
            return Ok(Written::NotAdmitted(me.hex));
        }
    }
    let (pseudo, _) = op_log::own_pseudonym(deps, community_id)?;
    refuse_invalid(&pseudo, &entry, &gov_state)?;
    let me = op_log::own_member(deps, community_id, &gov_state)?;
    let Some(record) = me.record else {
        return Ok(Written::NotAdmitted(me.hex));
    };

    // The pinned GenesisCert names the creator; S5 writes it at origin
    // and S6 pins it at join (`GovernanceRuntimeDeps::genesis_cert`).
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_string()))?;
    let identity_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let key = derive::derive_community_pseudonym(&identity_secret, community_id);
    let set = op_log::verify_held_with(
        deps,
        community_id,
        &deps.governance_ops(community_id).await?,
    );
    let (op, encoded) = op_log::build_op(
        community_id,
        &cert,
        &gov_state,
        &set,
        &key,
        entry.clone(),
        op_deps,
        deps.clock().now_secs(),
    )?;
    deps.store_signed_governance_op(
        community_id,
        &op_log::held(&op, record, encoded.clone(), op_log::OpFate::Applied),
    )
    .await?;
    deps.gossip_governance_op(community_id, &encoded);

    // The state every reader merges from the op set with this op in it.
    let mut ops = set.ops;
    ops.push(op);
    // The successors this state applies are judged as the rebuild judges
    // them (`op_log::rebuild_ops`), so our own `CommunitySuccessor` is
    // offered here without waiting for a peer's op; once per op
    // (`SuccessorJudgements`), run below outside the install lock. Boxed
    // with the state it reads, scoped so neither sits in every writer's
    // future.
    let announce_successors = {
        let installed = merge::merge(&cert, &ops);
        deps.set_governance_state(community_id, installed.clone());
        Box::pin(async move { crate::successor::announce(deps, community_id, &installed).await })
    };
    drop(installing);

    deps.emit_event(GovernanceRuntimeEvent::GovernanceEntryApplied {
        community_id: community_id.to_string(),
        entry: Box::new(entry.clone()),
    });
    announce_successors.await;

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

    // The op is stored and applied here; the outbox writes it to our `M`/2
    // and `M`/3, which a joiner's walk reads with nobody online. `landing`
    // says whether the caller needs it there before it returns.
    deps.publish_governance_op(community_id, encoded, landing)
        .await
        .map(Written::Op)
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

    /// A write reads the state and op set only under the rebuild lock
    /// (register S7l, HIGH-2): while a rebuild holds it, the write reads
    /// nothing, so a rebuild that read an older op set cannot install over
    /// the write's state.
    #[tokio::test]
    async fn a_write_waits_for_a_running_rebuild_before_reading_the_state() {
        let held = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let read = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut deps = crate::deps::MockGovernanceRuntimeDeps::new();
        let lock = std::sync::Arc::clone(&held);
        deps.expect_rebuild_lock()
            .returning(move |_| std::sync::Arc::clone(&lock));
        let seen = std::sync::Arc::clone(&read);
        deps.expect_governance_state().returning(move |_| {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            None
        });
        let deps = std::sync::Arc::new(deps);
        let rebuilding = held.lock().await;
        let writer = std::sync::Arc::clone(&deps);
        let write = tokio::spawn(async move {
            write_entry(
                writer.as_ref(),
                "community",
                GovernanceEntry::MemberLeft { heads: Vec::new() },
            )
            .await
        });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(
            !read.load(std::sync::atomic::Ordering::SeqCst),
            "the write read the state while a rebuild held the lock"
        );
        drop(rebuilding);
        let written = write.await.unwrap();
        assert!(read.load(std::sync::atomic::Ordering::SeqCst));
        assert!(matches!(
            written,
            Err(GovernanceRuntimeError::GovernanceStateMissing(_))
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
