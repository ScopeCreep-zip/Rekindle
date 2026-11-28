//! The apply pipeline (plan E3.2): bounds and permission checks against
//! the merged state, then our signed op is built, stored and published
//! (`op_log`), the state re-merged, and the UI snapshot events emitted.
//! All side effects flow through the `GovernanceRuntimeDeps` trait.

use rekindle_governance::state::GovernanceState;
use rekindle_governance::{merge, validate};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::{GovernanceRuntimeDeps, OpLanding, OpOutcome};
use crate::device_signer;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log;
use crate::own_op;
use crate::unlock_hold;
use rekindle_types::display_catch_up::{PendingGovKind, PendingGovOp, PendingGovTarget};

mod publication;
use publication::{publish_signed, ForumCommit};
pub use publication::{write_forum_thread, ForumCommitFuture};

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
        WriteEntry::Ready(Box::new(entry)),
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
        WriteEntry::Ready(Box::new(entry)),
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
    landing: OpLanding,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        WriteEntry::Ready(Box::new(entry)),
        landing,
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
        WriteEntry::Ready(Box::new(entry)),
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
        WriteEntry::Ready(Box::new(GovernanceEntry::Checkpoint(checkpoint))),
        OpLanding::Queued,
        op_log::Deps::Cited(heads),
    )
    .await
}

/// What a [`Build`] reads: the locked state, the verified op set and our
/// pseudonym `p`, exactly what the op it returns is built on.
pub(crate) struct BuiltOn<'a> {
    pub state: &'a GovernanceState,
    pub set: &'a op_log::VerifiedSet,
    pub author: &'a PseudonymKey,
}

/// An entry built under the rebuild lock from what the op will be built
/// on ([`BuiltOn`]): a `DeviceRetired`'s `lastSeq` must hold every op of its
/// key in the carrier's causal past (ADR 0022 rule 5), so it is read there.
pub(crate) type Build<D> =
    Box<dyn FnOnce(&D, &BuiltOn<'_>) -> Result<GovernanceEntry, GovernanceRuntimeError> + Send>;

/// The entry [`write_with`] writes: given, or built under its lock (with
/// the kind the unlock barrier lists it as while it is held).
pub(crate) enum WriteEntry<D> {
    Ready(Box<GovernanceEntry>),
    Built(Build<D>, PendingGovKind),
    ForumThread(Box<GovernanceEntry>, ForumCommit),
}

/// Our op for the entry `build` returns under the rebuild lock, answered
/// as [`write_entry`] answers.
pub(crate) async fn write_built<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    build: Build<D>,
    kind: PendingGovKind,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        WriteEntry::Built(build, kind),
        OpLanding::FirstAttempt,
        op_log::Deps::Frontier,
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
        WriteEntry::Ready(Box::new(GovernanceEntry::MemberLeft { heads })),
        OpLanding::FirstAttempt,
        op_log::Deps::Frontier,
        NotAdmitted::Withdraw,
    )
    .await?;
    Ok(match written {
        Written::Op(OpOutcome::Held { op_id }) => LeaveOutcome::Held { op_id },
        Written::Op(_) => LeaveOutcome::Left,
        Written::NotAdmitted(_) => LeaveOutcome::Withdrawn,
    })
}

async fn write<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: WriteEntry<D>,
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
    entry: WriteEntry<D>,
    landing: OpLanding,
    op_deps: op_log::Deps,
    not_admitted: NotAdmitted,
) -> Result<Written, GovernanceRuntimeError> {
    // Validation and our endorsement (ADR 0022 rule 3) precede the barrier:
    // neither a malformed entry nor an unendorsed device's op is ever held.
    let (ready, enrols) = match &entry {
        WriteEntry::Ready(ready) | WriteEntry::ForumThread(ready, _) => {
            refuse_out_of_bounds(ready)?;
            (Some(&**ready), false)
        }
        WriteEntry::Built(_, kind) => (
            None,
            matches!(
                kind,
                PendingGovKind::DeviceKey | PendingGovKind::DeviceReEnrol
            ),
        ),
    };
    refuse_now(deps, community_id, ready, enrols, landing, not_admitted).await?;
    // Held writes sign in arrival order after the unlock barrier and the
    // previous op's landing. Forum posts need a durable answer immediately.
    let (kind, target) = match &entry {
        WriteEntry::Ready(ready) | WriteEntry::ForumThread(ready, _) => {
            (PendingGovKind::of(ready), PendingGovTarget::of(ready))
        }
        WriteEntry::Built(_, kind) => (*kind, PendingGovTarget::default()),
    };
    let slot = if matches!(entry, WriteEntry::ForumThread(..)) {
        unlock_hold::BarrierSlot::Now(unlock_hold::take_immediate(deps, community_id)?)
    } else {
        unlock_hold::take_slot(deps, community_id, landing, |id| {
            let name = deps.pending_target_name(community_id, &target);
            let op = PendingGovOp::held_as(id, kind, target);
            match name {
                Some(name) => op.with_name(&name),
                None => op,
            }
        })
        .await?
    };
    match slot {
        unlock_hold::BarrierSlot::Now(turn) => {
            let written = write_signed(
                deps,
                community_id,
                entry,
                landing,
                op_deps,
                not_admitted,
                &turn,
            )
            .await;
            turn.finish(deps, community_id);
            written
        }
        unlock_hold::BarrierSlot::Held(held) => {
            held.echo(&entry);
            let op_id = held.op_id().to_owned();
            let label = held.label();
            let owned = deps.handle();
            let community = community_id.to_owned();
            deps.spawn_held_write(
                community_id,
                Box::pin(async move {
                    let signed = async {
                        let turn = held.turn(&owned, &community).await?;
                        let written = write_signed(
                            &owned,
                            &community,
                            entry,
                            landing,
                            op_deps,
                            not_admitted,
                            &turn,
                        )
                        .await;
                        turn.finish(&owned, &community);
                        written
                    };
                    if let Err(error) = signed.await {
                        unlock_hold::report_refused(&owned, &community, label, &error);
                    }
                }),
            );
            Ok(Written::Op(OpOutcome::Held { op_id }))
        }
    }
}

/// [`write_with`] past the unlock barrier, in `turn`.
async fn write_signed<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: WriteEntry<D>,
    landing: OpLanding,
    op_deps: op_log::Deps,
    not_admitted: NotAdmitted,
    turn: &unlock_hold::SigningTurn,
) -> Result<Written, GovernanceRuntimeError> {
    // Serialized with the rebuild (`dht_hydration`) from the state and
    // op-set reads to the install (D15: the state is a pure function of a
    // grow-only op set, so whoever reads last installs last; register S7l,
    // HIGH-2). Never held across the publish, and no rebuild runs under it.
    let lock = deps.rebuild_lock(community_id);
    let installing = locked(&lock, landing).await?;

    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    // Our member record M(p): the cert's for the creator, our
    // `MemberJoined` for a joiner (S6), in the state this write builds on;
    // it answers admission only, the op's record is our device's (below).
    // A join the merge never admitted has none: its
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
    let (mut entry, built_on, commit) = match entry {
        WriteEntry::Ready(ready) => (*ready, None, None),
        WriteEntry::ForumThread(ready, commit) => (*ready, None, Some(commit)),
        WriteEntry::Built(build, _) => {
            let held = deps.op_store().governance_ops(community_id).await?;
            let set = op_log::verify_held_with(deps, community_id, &held);
            let on = BuiltOn {
                state: &gov_state,
                set: &set,
                author: &pseudo,
            };
            let built = build(deps, &on)?;
            refuse_out_of_bounds(&built)?;
            (built, Some(set), None)
        }
    };
    refuse_invalid(&pseudo, &entry, &gov_state)?;
    let me = op_log::own_member(deps, community_id, &gov_state)?;
    if me.record.is_none() {
        return Ok(Written::NotAdmitted(me.hex));
    }
    // The pinned GenesisCert names the creator; S5 writes it at origin
    // and S6 pins it at join (`GovernanceRuntimeDeps::genesis_cert`).
    let cert = deps
        .genesis_cert(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(community_id.to_string()))?;
    let set = match built_on {
        Some(set) => set,
        None => op_log::verify_held_with(
            deps,
            community_id,
            &deps.op_store().governance_ops(community_id).await?,
        ),
    };
    let ours = own_op::Ours {
        community_id,
        cert: &cert,
        state: &gov_state,
        set: &set,
        author: pseudo,
    };
    let (op, encoded) = own_op::sign_ours(deps, ours, &mut entry, op_deps)?;
    // The record this device's op chain lives on, resolved from the signed
    // op as the publish path resolves it (`device_signer::op_record_of`,
    // ADR 0022 rules 3 and 10): a `MemberJoined` under the endorsement it
    // carries, any later op under the state's.
    let record = device_signer::op_record_of(deps, community_id, &gov_state, op.op())?;
    let held = op_log::held(&op, record, encoded.clone(), op_log::OpFate::Applied);
    let publication = publish_signed(deps, community_id, held, landing, turn, commit).await?;
    // Gossip before the install, which prunes a removed member it removes.
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

    // The outbox lands it on our `M`/2 and `M`/3; `landing` says how long we wait.
    publication.landed().await.map(Written::Op)
}

/// Every reader drops an entry outside its value bounds (plan E3.0), so the
/// writer refuses it by name before validating permission, instead of
/// writing what no peer applies or reporting `PermissionDenied`.
pub(crate) fn refuse_out_of_bounds(entry: &GovernanceEntry) -> Result<(), GovernanceRuntimeError> {
    match validate::bounds::out_of_bounds_field(entry) {
        Some(field) => Err(GovernanceRuntimeError::OutOfBounds { field }),
        None => Ok(()),
    }
}

/// [`refuse_invalid`]'s structural half and our device's endorsement
/// before the unlock barrier, read under the rebuild lock (register S7l,
/// HIGH-2): a malformed entry is `Malformed` (400) at once, and an op of a
/// device whose key the merged device set does not endorse is
/// `DeviceUnendorsed` at once (ADR 0022 rule 3: refused, never written,
/// so never held either); a device's own first op (its `MemberJoined`, or
/// the `DeviceKey` that enrols it) carries its endorsement and is resolved
/// where it is signed. Permission is judged in the state the op is signed
/// on, past the barrier. `ready` is the entry when it is known now, and
/// `enrols` marks a built `DeviceKey` or `DeviceReEnrol` (our enrolment
/// or re-enrolment, whose new key the set does not endorse yet).
async fn refuse_now<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    ready: Option<&GovernanceEntry>,
    enrols: bool,
    landing: OpLanding,
    not_admitted: NotAdmitted,
) -> Result<(), GovernanceRuntimeError> {
    let gov_state = {
        let lock = deps.rebuild_lock(community_id);
        let _installing = locked(&lock, landing).await?;
        deps.governance_state(community_id).ok_or_else(|| {
            GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string())
        })?
    };
    let carries_own = match ready {
        Some(ready) => {
            if matches!(not_admitted, NotAdmitted::Refuse) {
                let (pseudo, _) = op_log::own_pseudonym(deps, community_id)?;
                if !validate::well_formed(&pseudo, ready, &gov_state) {
                    return Err(GovernanceRuntimeError::Malformed);
                }
            }
            device_signer::carries_own_endorsement(deps, community_id, ready)?
        }
        None => enrols,
    };
    if carries_own {
        return Ok(());
    }
    device_signer::refuse_unendorsed(deps, community_id, &gov_state)
}

/// The community's rebuild lock, which a rebuild holds across its fetches
/// of missing deps (`op_log::rebuild_ops`): an invite (whose answer has a
/// deadline) waits for it only until its deadline, every other write until
/// it is free.
///
/// # Errors
/// [`GovernanceRuntimeError::InviteTimedOut`] at an invite's deadline.
async fn locked(
    lock: &tokio::sync::Mutex<()>,
    landing: OpLanding,
) -> Result<tokio::sync::MutexGuard<'_, ()>, GovernanceRuntimeError> {
    match landing {
        OpLanding::InviteLanded { deadline, .. } => tokio::time::timeout_at(deadline, lock.lock())
            .await
            .map_err(|_| invite_timed_out()),
        OpLanding::FirstAttempt | OpLanding::Queued => Ok(lock.lock().await),
    }
}

/// An invite whose steps before signing outran its command's network
/// deadline: refused with the deadline in words.
pub(crate) fn invite_timed_out() -> GovernanceRuntimeError {
    GovernanceRuntimeError::InviteTimedOut(
        rekindle_types::config::NETWORK_ANSWER_DEADLINE.as_secs(),
    )
}

/// Every reader runs `validate_write` against its merged state before it
/// applies an entry, so the writer runs it before writing: a malformed
/// entry is `Malformed` (400), a missing permission `PermissionDenied`
/// (403), and neither is written or applied.
pub(crate) fn refuse_invalid(
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
#[path = "apply_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "device_record_tests.rs"]
mod device_record_tests;
