//! The apply pipeline (plan E3.2): bounds and permission checks against
//! the merged state, then our signed op is built, stored and published
//! (`op_log`), the state re-merged, and the UI snapshot events emitted.
//! All side effects flow through the `GovernanceRuntimeDeps` trait.

use rekindle_governance::op::VerifiedOp;
use rekindle_governance::state::GovernanceState;
use rekindle_governance::{merge, validate};
use rekindle_secrets::derive;
use rekindle_types::governance::{GenesisCert, GovernanceEntry};
use rekindle_types::id::PseudonymKey;
use zeroize::Zeroizing;

use crate::deps::{GovernanceRuntimeDeps, OpLanding, OpOutcome};
use crate::device_signer;
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
    invite_id: [u8; 16],
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        WriteEntry::Ready(Box::new(entry)),
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

/// The entry [`write_with`] writes: given, or built under its lock.
pub(crate) enum WriteEntry<D> {
    Ready(Box<GovernanceEntry>),
    Built(Build<D>),
}

/// Our op for the entry `build` returns under the rebuild lock, answered
/// as [`write_entry`] answers.
pub(crate) async fn write_built<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    build: Build<D>,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    write(
        deps,
        community_id,
        WriteEntry::Built(build),
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
    if let WriteEntry::Ready(ready) = &entry {
        refuse_out_of_bounds(ready)?;
    }

    // Serialized with the rebuild (`dht_hydration`) from the state and
    // op-set reads to the install (D15: the state is a pure function of a
    // grow-only op set, so whoever reads last installs last; register S7l,
    // HIGH-2). Never held across the publish, and no rebuild runs under it.
    let lock = deps.rebuild_lock(community_id);
    let installing = lock.lock().await;

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
    let (mut entry, built_on) = match entry {
        WriteEntry::Ready(ready) => (*ready, None),
        WriteEntry::Built(build) => {
            let held = deps.governance_ops(community_id).await?;
            let set = op_log::verify_held_with(deps, community_id, &held);
            let on = BuiltOn {
                state: &gov_state,
                set: &set,
                author: &pseudo,
            };
            let built = build(deps, &on)?;
            refuse_out_of_bounds(&built)?;
            (built, Some(set))
        }
    };
    refuse_invalid(&pseudo, &entry, &gov_state)?;
    let me = op_log::own_member(deps, community_id, &gov_state)?;
    if me.record.is_none() {
        return Ok(Written::NotAdmitted(me.hex));
    }
    // The record this device's op chain lives on (ADR 0022 rule 3): a
    // `MemberJoined`, the device's first op, carries its own endorsement
    // of the record it names; any later op lands on the record our key's
    // endorsement binds in the state ([`device_signer::own_op_record`]).
    // Our `DeviceKey` endorsing our own fresh key is that key's first op
    // (rule 3): it lands on the `M(p,d)` it binds.
    let record = match &entry {
        GovernanceEntry::MemberJoined { member_record, .. } => member_record.clone(),
        GovernanceEntry::DeviceKey(e) if device_signer::is_own_key(deps, community_id, e)? => {
            e.device_record.clone()
        }
        _ => device_signer::own_op_record(deps, community_id, &gov_state)?,
    };

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
            &deps.governance_ops(community_id).await?,
        ),
    };
    let ours = Ours {
        community_id,
        cert: &cert,
        state: &gov_state,
        set: &set,
        author: pseudo,
    };
    let (op, encoded) = sign_ours(deps, ours, &mut entry, op_deps)?;
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

/// What our op is built on: the community, its pinned cert, the locked
/// state and the verified op set, and our pseudonym `p` (its author).
struct Ours<'a> {
    community_id: &'a str,
    cert: &'a GenesisCert,
    state: &'a GovernanceState,
    set: &'a op_log::VerifiedSet,
    author: PseudonymKey,
}

/// Our op for `entry`, signed by this device's op key under `p` (ADR 0022
/// rule 1); a `MemberJoined`, this device's first op here, gets `p`'s
/// endorsement of the key written into `entry` (rule 3). Kept off
/// the async path: the device key and `p`'s secret never live across an
/// await, so `write_with`'s future holds neither.
///
/// # Errors
/// No identity secret, no stored device key for a later op, or
/// [`op_log::build_op`]'s.
fn sign_ours<D: GovernanceRuntimeDeps>(
    deps: &D,
    ours: Ours<'_>,
    entry: &mut GovernanceEntry,
    op_deps: op_log::Deps,
) -> Result<(VerifiedOp, Vec<u8>), GovernanceRuntimeError> {
    let identity_secret = Zeroizing::new(
        deps.identity_secret()
            .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?,
    );
    let key = derive::derive_community_pseudonym(&identity_secret, ours.community_id);
    let first_op = matches!(entry, GovernanceEntry::MemberJoined { .. });
    let device = device_signer::device_op_key(deps, ours.community_id, first_op)?;
    device_signer::carry_first_device(entry, &key, ours.community_id, &device)?;
    let signer = device_signer::OpSigner {
        author: ours.author,
        device: &device,
    };
    op_log::build_op(
        ours.community_id,
        ours.cert,
        ours.state,
        ours.set,
        signer,
        entry.clone(),
        op_deps,
        deps.clock().now_secs(),
    )
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
#[path = "apply_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "device_record_tests.rs"]
mod device_record_tests;
