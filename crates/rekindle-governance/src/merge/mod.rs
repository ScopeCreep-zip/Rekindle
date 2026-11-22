//! CRDT merge engine — the core of flat governance (plan E3.2).
//!
//! Takes the community's verified op set and its pinned `GenesisCert` and
//! produces a `GovernanceState`. **Convergence:** the same op set gives the
//! same state on every reader, whatever order the ops arrived in.
//!
//! **Resolution** follows Matrix state resolution v2: power ops
//! ([`crate::power`]) are resolved first, in causal (stamp) order, each
//! checked against the power state so far; every other op is then ordered
//! by its mainline anchor (the latest resolved power op in its causal
//! past), its `issued_at` and its hash, and folded in that order.
//!
//! **Time-of-op judgement** (plan E3.2 S7c.3a): a non-power op is judged
//! against the power state in its own causal past, the state after its
//! anchor (genesis when it has none), never the final one, so a later
//! power op never re-judges an earlier non-power op (Keybase's
//! `AssertWasRoleOrAboveAt(seqno-1)`, p2panda's `state_at(dependencies)`).
//!
//! **Strong removal** follows p2panda-auth, per author: a `MemberRemoved`
//! or `BanEntry` carries its target's seq frontier, and every op of the
//! target at or above it is masked and quarantined, whatever deps it
//! cites, while every op below it stands, judged at its own position. A
//! demotion masks the demoted member's ops concurrent with it or after
//! it. Mutual removals both apply, a re-join after a removal is allowed
//! (its concurrent ops stay masked), and masking is transitive —
//! resolution reruns until the invalidated set is stable.
//!
//! **The creators** are the cert's creator set, `creator_pseudonym` and
//! `additional_creators` (MSC4289, Q3): no op at any lamport adds or
//! removes one. The primary creator is a member from the start,
//! bound to the cert's `creator_member_record` and its owner: the cert, not
//! a `MemberJoined`, carries the creator's binding (the genesis ops are
//! meta, roles, the owner assignment and `#general`), so a `MemberLeft`
//! still removes it like any member.

use std::collections::{BTreeMap, HashSet};

use rekindle_types::governance::{GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;
use rekindle_types::join_refusal::JoinRefusal;

use crate::checkpoint::{self, EndorsementFault, Hints};

use crate::op::{OpStamp, VerifiedOp};
use crate::power::{class_of, OpClass};
use crate::state::{
    AdminDeleteState, AdmissionDecision, AutoModRuleState, CategoryState, ChannelState,
    CommunityPolicyState, EventState, ExpressionState, GovernanceState, InviteState, MemberState,
    MetadataState, OnboardingState, OverwriteState, RoleState, SegmentState, ThreadState,
    TimeoutState, WelcomeScreenState,
};
use crate::validate::validate_write;

/// What `merge` decided about each op, besides the state.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeOutcome {
    pub state: GovernanceState,
    /// Applied ops, in the order they were applied.
    pub applied: Vec<OpHash>,
    /// Complete ops refused by validation or invalidated by resolution,
    /// the quarantined ones aside.
    pub refused: Vec<OpHash>,
    /// Ops masked by their author's removal, in stamp order, then the
    /// `Checkpoint` ops that count for nothing (S7c.4), with why (plan E3.2
    /// S7c.3a): shown, never applied.
    pub quarantined: Vec<(OpHash, QuarantineReason)>,
    /// Ops whose deps are missing from the set.
    pub pending: Vec<OpHash>,
    /// Authors caught signing two ops at one seq, each with its fork seq
    /// (the lowest seq it signed twice): its ops from there on are
    /// dropped, its earlier ones merged as any other (D15).
    pub equivocators: Vec<(PseudonymKey, u64)>,
    /// The checkpoint hints the applied `Checkpoint` ops carry (plan E3.2
    /// S7c.4); the ones that count for nothing are quarantined above.
    pub hints: Hints,
    /// Why each refused `MemberJoined` the invite gate refused was refused
    /// (plan E3.2 join step 5: the inviter answers it to the joiner).
    pub join_refusals: BTreeMap<OpHash, JoinRefusal>,
}

/// Why an op is quarantined rather than applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuarantineReason {
    /// After its author's removal: its seq is at or above the target seq
    /// frontier of `cause`, a `MemberRemoved` or `BanEntry` of its author.
    AfterAuthorRemoval { cause: OpHash },
    /// "two checkpoint endorsements in one round": a `Checkpoint` op whose
    /// author signed another at the same (height, round) (plan E3.2 S7c.4);
    /// `cause`: the lowest `(lamport, op_hash)` other one.
    TwoCheckpointEndorsementsInOneRound { cause: OpHash },
    /// "checkpoint equivocation": a `Checkpoint` op carrying one of two
    /// digests that each have a strict majority for one cut and one
    /// `prevCheckpoint` (plan E3.2 S7c.4); `cause`: the lowest
    /// `(lamport, op_hash)` `Checkpoint` op of the other digest.
    CheckpointEquivocation { cause: OpHash },
    /// "member record already claimed": a `MemberJoined` or
    /// `MemberRecordMoved` naming a member record `holder` holds (the
    /// record gate, ADR 0011: one member per record). The claimant learns
    /// why its join did not take (D15 "visible reason").
    RecordClaimed { holder: PseudonymKey },
    /// "quarantined because <cause> was": its authority (a role grant, an
    /// invite, a channel or a ban it acts on) came from `cause`, an op
    /// quarantined after its author's fork, and fails without it (ADR 0022
    /// rule 5, plan E3.2 S7d-keys.2a); with several, the lowest
    /// `(lamport, op_hash)`.
    AuthorityQuarantined { cause: OpHash },
}

impl QuarantineReason {
    /// The op that caused the quarantine; none for a record claimed by
    /// its holder (the cert's creator holds its record with no op).
    #[must_use]
    pub fn cause(&self) -> Option<OpHash> {
        match self {
            Self::AfterAuthorRemoval { cause }
            | Self::TwoCheckpointEndorsementsInOneRound { cause }
            | Self::CheckpointEquivocation { cause }
            | Self::AuthorityQuarantined { cause } => Some(*cause),
            Self::RecordClaimed { .. } => None,
        }
    }
}

/// Merge `ops` under the pinned `cert` into a canonical state.
#[must_use]
pub fn merge(cert: &GenesisCert, ops: &[VerifiedOp]) -> GovernanceState {
    let set = resolve::prepare(ops);
    ancestry::settle(cert, &set).settled.0.state
}

/// [`merge`], with the fate of every op.
#[must_use]
pub fn merge_outcome(cert: &GenesisCert, ops: &[VerifiedOp]) -> MergeOutcome {
    let set = resolve::prepare(ops);
    let ancestry::Settlement { settled, cascade } = ancestry::settle(cert, &set);
    let (round, masked_by, invite_refusals, record_refusals) = settled;
    // A forked op is reported once, by `equivocators` (D15); a cascade
    // victim once, quarantined with its cause.
    let shown = |i: usize| !set.forked[i] && !cascade.contains_key(&i);
    let join_refusals = (invite_refusals.into_iter())
        .filter(|&(i, _)| shown(i) && !round.applied[i] && masked_by[i].is_none())
        .map(|(i, r)| (set.ops[i].hash(), r))
        .collect();
    let mut applied: Vec<OpHash> = round.order.iter().map(|&i| set.ops[i].hash()).collect();
    let refused = (0..set.ops.len())
        .filter(|&i| shown(i) && !round.applied[i] && masked_by[i].is_none())
        .filter(|i| !record_refusals.contains_key(i))
        .map(|i| set.ops[i].hash())
        .collect();
    let mut quarantined: Vec<(OpHash, QuarantineReason)> = (0..set.ops.len())
        .filter(|&i| shown(i))
        .filter_map(|i| {
            let cause = set.ops[masked_by[i]?].hash();
            Some((
                set.ops[i].hash(),
                QuarantineReason::AfterAuthorRemoval { cause },
            ))
        })
        .collect();
    quarantined.extend(
        (record_refusals.iter())
            .filter(|&(&i, _)| shown(i) && !round.applied[i] && masked_by[i].is_none())
            .map(|(&i, holder)| {
                (
                    set.ops[i].hash(),
                    QuarantineReason::RecordClaimed {
                        holder: holder.clone(),
                    },
                )
            }),
    );
    quarantined.extend(cascade.iter().map(|(&i, &q)| {
        let cause = set.ops[q].hash();
        (
            set.ops[i].hash(),
            QuarantineReason::AuthorityQuarantined { cause },
        )
    }));
    let counted: HashSet<OpHash> = applied.iter().copied().collect();
    let hints = checkpoint::hints(cert, ops, |h| counted.contains(h));
    let faulted: HashSet<OpHash> = hints.faults.iter().map(|(h, _)| *h).collect();
    applied.retain(|h| !faulted.contains(h));
    quarantined.extend(hints.faults.iter().map(|&(hash, fault)| {
        let reason = match fault {
            EndorsementFault::TwoInOneRound { cause } => {
                QuarantineReason::TwoCheckpointEndorsementsInOneRound { cause }
            }
            EndorsementFault::Equivocation { cause } => {
                QuarantineReason::CheckpointEquivocation { cause }
            }
        };
        (hash, reason)
    }));
    MergeOutcome {
        state: round.state,
        applied,
        refused,
        quarantined,
        pending: set.pending,
        equivocators: set.equivocators,
        hints,
        join_refusals,
    }
}

/// Resolution rerun until the invalidated set is stable (`fixpoint`); the settled
/// round's invites carry the joins `invite_gate` counted on them. Returns
/// the round, per op the removal that masks it, and why the invite gate
/// refused each join it refused, and the holder of each record a refused
/// claim named.
type Settled = (
    Round,
    Vec<Option<usize>>,
    BTreeMap<usize, JoinRefusal>,
    BTreeMap<usize, PseudonymKey>,
);

/// `exclude`: ops every round leaves out besides the gates' (the forked
/// ops and the refusals kept, `ancestry`).
fn resolve_stable(cert: &GenesisCert, set: &resolve::OpSet<'_>, exclude: &[bool]) -> Settled {
    let skip = |invalid: &[bool]| -> Vec<bool> {
        invalid.iter().zip(exclude).map(|(a, b)| *a || *b).collect()
    };
    let settled = fixpoint::settle(
        set.ops.len(),
        |invalid| resolve_round(cert, set, &skip(invalid)),
        |round| Ok::<_, std::convert::Infallible>(gates::invalidated(cert, set, round)),
        |gated| &gated.invalid,
    );
    let (mut round, gated) = match settled {
        Ok(settled) => settled,
        Err(never) => match never {},
    };
    for (id, invite) in &mut round.state.invites {
        invite.uses = gated.uses.get(id).copied().unwrap_or(0);
    }
    (
        round,
        gated.masked_by,
        gated.invite_refusals,
        gated.record_refusals,
    )
}

/// One resolution pass with the ops in `invalid` left out.
pub(super) struct Round {
    pub state: GovernanceState,
    pub applied: Vec<bool>,
    /// Applied ops in judgement order (power ops first).
    pub order: Vec<usize>,
    /// The applied power ops, in their order.
    pub power_order: Vec<usize>,
    /// Each op's anchor: the position in `power_order` of the latest
    /// applied power op in its causal past.
    pub anchor: Vec<Option<usize>>,
}

/// The state before any op: the cert's creator set, policy and MEK
/// generation, with the primary creator admitted (the cert names its member
/// record; a further creator is admitted by its own `MemberJoined`, plan
/// E3.2 S7s decision (17)).
fn genesis_state(cert: &GenesisCert) -> GovernanceState {
    let mut state = GovernanceState {
        creators: cert.creator_set(),
        admission_mode: Some(cert.policy),
        mek_generation: crate::state::GENESIS_MEK_GENERATION,
        ..GovernanceState::default()
    };
    state.members.insert(
        cert.creator_pseudonym.clone(),
        MemberState {
            member_record: cert.creator_member_record.clone(),
            member_record_owner: cert.creator_member_record_owner.clone(),
            joined: OpStamp::MIN,
        },
    );
    state
}

/// One pass. Power ops resolve against each other as before: in stamp
/// order, each against the power state so far. Each non-power op is then
/// judged, in mainline order, against the power state after its anchor
/// (time-of-op judgement, plan E3.2 S7c.3a): the power ops are folded into
/// the running state one by one as the mainline reaches their anchors, so
/// non-power effects still fold in mainline order and the settled state
/// holds every applied op.
fn resolve_round(cert: &GenesisCert, set: &resolve::OpSet<'_>, invalid: &[bool]) -> Round {
    let n = set.ops.len();
    let mut applied = vec![false; n];
    let mut order = Vec::new();
    let candidates = (0..n).filter(|&i| !invalid[i]);
    let (device, others): (Vec<usize>, Vec<usize>) =
        candidates.partition(|&i| class_of(set.ops[i].body()) == OpClass::Device);
    let (power, rest): (Vec<usize>, Vec<usize>) =
        (others.into_iter()).partition(|&i| class_of(set.ops[i].body()) == OpClass::Power);
    let mut power_state = genesis_state(cert);
    // The device phase (ADR 0022 rule 6): before every power op, in stamp
    // order; device ops anchor no mainline op.
    for i in device {
        if try_apply(set.ops[i], &mut power_state) {
            applied[i] = true;
            order.push(i);
        }
    }
    let device_order = order.len();
    for i in power {
        if frontier_holds(set, i) && try_apply(set.ops[i], &mut power_state) {
            applied[i] = true;
            order.push(i);
        }
    }
    let power_order = order[device_order..].to_vec();
    let anchor: Vec<Option<usize>> = (0..n)
        .map(|i| power_order.iter().rposition(|&p| set.past.contains(i, p)))
        .collect();
    let mut mainline: Vec<(Option<usize>, u64, OpHash, usize)> = rest
        .into_iter()
        .map(|i| (anchor[i], set.ops[i].op().issued_at, set.ops[i].hash(), i))
        .collect();
    mainline.sort();
    let mut state = genesis_state(cert);
    for &d in &order[..device_order] {
        fold(set.ops[d], &mut state);
    }
    let mut folded = 0;
    for (at, _, _, i) in mainline {
        let through = at.map_or(0, |a| a + 1);
        for &p in power_order.get(folded..through).unwrap_or_default() {
            fold(set.ops[p], &mut state);
        }
        folded = folded.max(through);
        if try_apply(set.ops[i], &mut state) {
            applied[i] = true;
            order.push(i);
        }
    }
    for &p in &power_order[folded..] {
        fold(set.ops[p], &mut state);
    }
    state.devices.clone_from(&set.devices);
    Round {
        state,
        applied,
        order,
        power_order,
        anchor,
    }
}

/// A `MemberRemoved` or `BanEntry` holds only when its target seq frontier
/// is the number of its target's ops in its own causal past (plan E3.2
/// S7c.3a), a pure function of its deps; every other op holds.
fn frontier_holds(set: &resolve::OpSet<'_>, i: usize) -> bool {
    let body = set.ops[i].body();
    let (Some(claimed), Some(target)) = (
        crate::power::target_seq_frontier(body),
        crate::power::removal_target(body),
    ) else {
        return true;
    };
    let counted = (0..i)
        .filter(|&o| set.ops[o].author() == target && set.past.contains(i, o))
        .map(|o| set.ops[o].op().seq + 1)
        .max()
        .unwrap_or(0);
    claimed == counted
}

fn try_apply(op: &VerifiedOp, state: &mut GovernanceState) -> bool {
    if !validate_write(op.author(), op.body(), state) {
        return false;
    }
    fold(op, state);
    true
}

/// The effects of an op already judged: its CRDT apply rule, the slot
/// heads a departure states, and who removed whom.
fn fold(op: &VerifiedOp, state: &mut GovernanceState) {
    apply(op.author(), op.body(), op.stamp(), state);
    // Every departure states the member's slot heads: the remover's view
    // for a kick, the leaver's own for a leave (A/B finding R3-8). Readers
    // judge by every merged departure's heads: a later departure from
    // another slot never drops what an earlier one kept (A/B F8-1).
    let departed = match op.body() {
        GovernanceEntry::MemberLeft { heads } => Some((op.author(), heads)),
        GovernanceEntry::MemberRemoved { target, heads, .. } => Some((target, heads)),
        _ => None,
    };
    if let Some((gone, heads)) = departed {
        let held = state.departed_heads.entry(gone.clone()).or_default();
        for head in heads {
            (held.entry(head.record_key.clone()).or_default()).insert((head.seq, head.digest));
        }
    }
    let remover_of = match op.body() {
        GovernanceEntry::MemberRemoved { target, .. } => {
            state.removed.get(target).is_some_and(|s| *s == op.stamp())
        }
        GovernanceEntry::BanEntry { target, .. } => state
            .ban_stamps
            .get(target)
            .is_some_and(|s| *s == op.stamp()),
        _ => false,
    }
    .then(|| crate::power::removal_target(op.body()))
    .flatten();
    if let Some(target) = remover_of {
        state.removed_by.insert(target.clone(), op.author().clone());
        match removal_reason(op.body()) {
            Some(reason) => state
                .removal_reasons
                .insert(target.clone(), reason.to_owned()),
            None => state.removal_reasons.remove(target),
        };
    }
}

/// The reason a removal op gives, when it gives one.
fn removal_reason(entry: &GovernanceEntry) -> Option<&str> {
    match entry {
        GovernanceEntry::MemberRemoved { reason, .. }
        | GovernanceEntry::BanEntry { reason, .. } => reason.as_deref().filter(|r| !r.is_empty()),
        _ => None,
    }
}

/// Apply a single governance entry, stamped by its op, to the state.
/// Public for incremental local updates after validation.
pub fn apply_entry(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    apply(author, entry, stamp, state);
}

/// Route an entry to its category-specific CRDT apply rule.
fn apply(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    use GovernanceEntry as G;
    match entry {
        G::ChannelCreated { .. }
        | G::ChannelArchived { .. }
        | G::ChannelUpdated { .. }
        | G::CategoryCreated { .. }
        | G::CategoryArchived { .. }
        | G::CategoryUpdated { .. }
        | G::PermissionOverwrite { .. } => apply_channels(entry, stamp, state),
        G::MemberApproved { .. } | G::MemberRejected { .. } => {
            apply_admission(entry, stamp, state);
        }
        G::MemberJoined { .. }
        | G::MemberRecordMoved { .. }
        | G::MemberLeft { .. }
        | G::MemberRemoved { .. } => apply_membership(author, entry, stamp, state),
        G::CommunityMeta { .. }
        | G::CommunityNotificationDefault { .. }
        | G::MEKGenerationBump { .. }
        | G::SegmentAdded { .. }
        | G::CommunityPolicy { .. }
        | G::InviteCreated { .. }
        | G::InviteRevoked { .. } => apply_community(author, entry, stamp, state),
        G::ThreadCreated { .. }
        | G::ThreadArchived { .. }
        | G::ThreadUnarchived { .. }
        | G::EventCreated { .. }
        | G::EventArchived { .. }
        | G::EventRsvp { .. } => apply_events(author, entry, stamp, state),
        G::ExpressionAdded { .. } | G::ExpressionRemoved { .. } | G::AttachmentPinned { .. } => {
            apply_expression(entry, stamp, state);
        }
        G::BanEntry { .. }
        | G::UnbanEntry { .. }
        | G::TimeoutEntry { .. }
        | G::RemoveTimeoutEntry { .. }
        | G::AutoModRule { .. } => apply_moderation(entry, stamp, state),
        // A tombstone (plan E3.3 M4): the first delete merged keeps its
        // moderator, so every reader names the same one.
        G::AdminDelete {
            message_id,
            channel_id,
            author: post_author,
            ..
        } => {
            (state.admin_deletes.entry(*message_id)).or_insert_with(|| AdminDeleteState {
                by: author.clone(),
                channel_id: *channel_id,
                author: post_author.clone(),
            });
        }
        G::ModReport { .. } => apply_mod_report(author, entry, stamp, state),
        G::OnboardingConfig { .. } | G::WelcomeScreen { .. } => {
            apply_onboarding(entry, stamp, state);
        }
        G::RoleDefinition { .. }
        | G::RoleAssignment { .. }
        | G::RoleUnassignment { .. }
        | G::RoleArchived { .. } => apply_roles(entry, stamp, state),
        // A hint over a cut changes no state (plan E3.2 S7c.3). The device
        // endorsement set is harvested from every carrier, quarantined ones
        // included (ADR 0022 rule 2), and a readmit counts in the deps of
        // the op it admits (rule 7): `resolve::prepare` harvests them from
        // the op set into `GovernanceState::devices` (K-CP), never here.
        G::Checkpoint(_) | G::DeviceKey(_) | G::DeviceRetired(_) | G::DeviceReadmit(_) => {}
        // Kept whole: the gates judge acceptances against applied offers,
        // and readers show every successor (plan E3.2 S7s.3e).
        G::CommunitySuccessor(successor) => {
            let row = crate::successor::SuccessorState {
                author: author.clone(),
                successor: successor.clone(),
                stamp,
            };
            state.successors.insert(stamp.hash, row);
        }
        G::SuccessorLinkRequest(_) | G::SuccessorLink(_) => {
            crate::link_round::apply(author, entry, stamp, state);
        }
    }
}

mod ancestry;
mod channels;
mod community;
mod cut;
#[cfg(test)]
mod device_cp_tests;
#[cfg(test)]
mod device_tests;
mod events;
mod expression;
mod fixpoint;
mod gates;
mod membership;
mod moderation;
mod onboarding;
mod resolve;
mod resume;
mod resume_gates;
mod roles;

#[cfg(test)]
mod ancestry_tests;
#[cfg(test)]
mod checkpoint_tests;
#[cfg(test)]
mod hint_tests;
#[cfg(test)]
mod op_proptests;
#[cfg(test)]
mod proptests;
#[cfg(test)]
mod resume_beside_tests;
#[cfg(test)]
mod resume_hint_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod tests;

use channels::apply_channels;
use community::apply_community;
pub use cut::merge_cut;
use events::apply_events;
use expression::apply_expression;
use membership::apply_membership;
use moderation::{apply_admission, apply_mod_report, apply_moderation};
use onboarding::apply_onboarding;
pub use resume::{merge_from, NotOnTop};
use roles::apply_roles;
