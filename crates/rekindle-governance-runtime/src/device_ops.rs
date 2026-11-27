//! Our device-key acts in a community (ADR 0022, plan E3.2 S7d-keys K5):
//! `p` retires one of our device keys (rules 4 and 5), or our device key
//! carries the re-admission of a member's fork-answering key (rules 7 and
//! 8). Each element is built by [`device_writer`] under the rebuild lock
//! from the state and op set its carrier is built on
//! ([`apply::write_built`]), so it is valid where it lands.

use rekindle_crypto::keypair_text;
use rekindle_governance::device_judge::MemberDevices;
use rekindle_governance::device_writer::{self, DeviceRecord};
use rekindle_governance::merge;
use rekindle_records::schema;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use zeroize::Zeroizing;

use crate::apply::{self, BuiltOn};
use crate::deps::{GovernanceRuntimeDeps, OpOutcome};
use crate::device_signer::{self, OwnDevice};
use crate::error::GovernanceRuntimeError;

/// Retire our device key `device_key` in `community` above the highest op
/// of it we hold: `p` signs the `DeviceRetired` (rule 4), carried by our
/// op. This device's own key is refused: a device never retires the key
/// that signs the carrier (it re-enrols instead, rule 4).
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceActRefused`] for this device's own key
/// or a key we never endorsed, or the write's.
pub async fn retire<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    device_key: [u8; 32],
) -> Result<OpOutcome, GovernanceRuntimeError> {
    let anchor = community.to_owned();
    let build: apply::Build<D> =
        Box::new(move |deps: &D, on: &BuiltOn<'_>| retirement(deps, &anchor, on, device_key));
    apply::write_built(
        deps,
        community,
        build,
        rekindle_types::display_catch_up::PendingGovKind::DeviceRetired,
    )
    .await
}

/// Co-sign the re-admission of `member`'s fork-answering device key
/// `device_key` in `community` (rules 7 and 8): our op carries the
/// `DeviceReadmit` naming the endorsement the member's set holds.
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceActRefused`] when the member's set
/// holds no fork-answering endorsement of the key, or the write's (a
/// co-signer without the authority rule 8 asks is `PermissionDenied`).
pub async fn readmit<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    member: PseudonymKey,
    device_key: [u8; 32],
) -> Result<OpOutcome, GovernanceRuntimeError> {
    let build: apply::Build<D> =
        Box::new(move |_: &D, on: &BuiltOn<'_>| readmission(on, &member, device_key));
    apply::write_built(
        deps,
        community,
        build,
        rekindle_types::display_catch_up::PendingGovKind::DeviceReadmit,
    )
    .await
}

/// Enrol this device in `community` (ADR 0022 rules 1-4, lead decision
/// 63): mint a fresh device key `d` (never a held one: a key endorsed
/// under an ended membership is never revived), create its own `M(p,d)`
/// with a random owner `K_M(p,d)`, keep both in the device vault, and
/// write `p`'s `DeviceKey` endorsement of `d` as `d`'s seq-0 op on that
/// record, built from the held set under the rebuild lock. A device whose
/// key is already endorsed for the current member record is refused.
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceActRefused`] for an endorsed device, a
/// key already in the set or a record another key binds (rule 3), the
/// record's creation, the vault's, or the write's.
pub async fn enrol<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    let state = deps.governance_state(community).unwrap_or_default();
    if device_signer::own_device(deps, community, &state)? == OwnDevice::Endorsed {
        return Err(GovernanceRuntimeError::DeviceActRefused(
            "this device's key is already endorsed",
        ));
    }
    let (secret, record, owner_keypair) = mint_device(deps).await?;
    deps.store_device_op_key(community, &secret)?;
    let anchor = community.to_owned();
    // The key is ours from here (the enrol answers it), but its `M(p,d)`
    // binding is stored only once the endorsement is built, after the
    // unlock barrier passed (plan E3.2 S7d-keys.8): the unlock's device
    // check (`catch_up_own_device`, which stores an unendorsed key again
    // alone) has run by then, so it never strips the binding of our own
    // held endorsement, and the op record is always written with its own
    // owner `K_M(p,d)` (ADR 0022 rule 3), never `K_M(p)`.
    let build: apply::Build<D> = Box::new(move |deps: &D, on: &BuiltOn<'_>| {
        let bound = record.record.clone();
        let entry = new_endorsement(deps, &anchor, on, record, &secret)?;
        deps.store_device_record(&anchor, &bound, &owner_keypair)?;
        Ok(GovernanceEntry::DeviceKey(entry))
    });
    apply::write_built(
        deps,
        community,
        build,
        rekindle_types::display_catch_up::PendingGovKind::DeviceKey,
    )
    .await
}

/// A fresh device key and its own new `M(p,d)` record with a random owner
/// `K_M(p,d)` (ADR 0022 rules 1-3): the enrolment's and the re-enrolment's
/// (S7d-keys.10). Nothing is stored yet.
///
/// # Errors
/// The record's creation, or an owner keypair it lacks.
pub(crate) async fn mint_device<D: GovernanceRuntimeDeps>(
    deps: &D,
) -> Result<(Zeroizing<[u8; 32]>, DeviceRecord, Zeroizing<String>), GovernanceRuntimeError> {
    let mut secret = Zeroizing::new([0u8; 32]);
    deps.rng().fill(secret.as_mut_slice());
    let info = deps.create_record(&schema::member_record()).await?;
    deps.release_record(info.lease).await;
    let owner_keypair = Zeroizing::new(info.owner_keypair.ok_or_else(|| {
        GovernanceRuntimeError::Adapter("enrol: the device record has no owner keypair".into())
    })?);
    let (public, _) = keypair_text::parse_keypair_text(&owner_keypair)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("device record owner: {e}")))?;
    let record = DeviceRecord {
        record: info.record_key,
        owner: keypair_text::public_key_text(&public),
    };
    Ok((secret, record, owner_keypair))
}

/// `p`'s endorsement of this device's new key `held` for `record`, extending the
/// set `on` holds, signed for the current member record (rule 10), naming
/// the fork event it answers ([`answered_fork`]). The
/// writer refuses what readers refuse: a key the set already endorses and
/// a record another endorsement (or the member record) binds (rule 3's
/// `RecordTaken`).
pub(crate) fn new_endorsement<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    on: &BuiltOn<'_>,
    record: DeviceRecord,
    held: &[u8; 32],
) -> Result<rekindle_types::governance::DeviceEndorsement, GovernanceRuntimeError> {
    let key = SigningKey::from_bytes(held).verifying_key().to_bytes();
    let me = crate::op_log::own_member(deps, community, on.state)?;
    let member = (me.record).ok_or(GovernanceRuntimeError::MemberRecordMissing(me.hex))?;
    let set = on.state.devices.member(on.author);
    refuse_taken(set, key, &record.record, &member)?;
    let identity = Zeroizing::new(
        deps.identity_secret()
            .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?,
    );
    let p = derive::derive_community_pseudonym(&identity, community);
    let fork = answered_fork(on, &member);
    device_writer::endorse(&p, community, &member, set, key, record, fork)
        .map_err(|_| GovernanceRuntimeError::DeviceActRefused("the device set is full"))
}

/// The fork evidence a new endorsement of our key names when built on `on`
/// (ADR 0022 rule 7, amended 2026-11-23): the lowest-stamp fork event of
/// ours `on` holds, a fork pair's hash or a retirement's under our member
/// record `member`, else none (`[0; 32]`). A key endorsed after a fork
/// event needs a `DeviceReadmit` naming the same evidence.
fn answered_fork(on: &BuiltOn<'_>, member: &str) -> [u8; 32] {
    let none = MemberDevices::default();
    let md = on.state.devices.member(on.author).unwrap_or(&none);
    merge::first_fork_evidence(&on.set.ops, on.author, md, member).unwrap_or([0; 32])
}

/// Rule 3 at the writer: refuse a key `set` already endorses and a record
/// another endorsement or the member record `member` binds.
fn refuse_taken(
    set: Option<&MemberDevices>,
    key: [u8; 32],
    record: &str,
    member: &str,
) -> Result<(), GovernanceRuntimeError> {
    if record == member {
        return Err(GovernanceRuntimeError::DeviceActRefused(
            "the member record is the first device's (RecordTaken)",
        ));
    }
    let endorsed = set.map(|md| md.endorsements.values()).into_iter().flatten();
    for e in endorsed {
        if e.device_key == key {
            return Err(GovernanceRuntimeError::DeviceActRefused(
                "the key is already endorsed",
            ));
        }
        if e.device_record == record {
            return Err(GovernanceRuntimeError::DeviceActRefused(
                "another device key binds the record (RecordTaken)",
            ));
        }
    }
    Ok(())
}

fn retirement<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    on: &BuiltOn<'_>,
    device_key: [u8; 32],
) -> Result<GovernanceEntry, GovernanceRuntimeError> {
    if let Some(ours) = deps.device_op_key(community)? {
        if SigningKey::from_bytes(&ours).verifying_key().to_bytes() == device_key {
            return Err(GovernanceRuntimeError::DeviceActRefused(
                "this device's own key is re-enrolled, not retired",
            ));
        }
    }
    let retired = p_retirement(deps, community, on, device_key, last_seq_of(on, device_key))?;
    Ok(GovernanceEntry::DeviceRetired(retired))
}

/// `p`'s `DeviceRetired` of our key `device_key` above `last_seq`, at the
/// incarnation the set `on` holds endorsed it at (rules 4, 5, 10): the
/// retirement's and the re-enrolment's (S7d-keys.10).
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceActRefused`] for a key the set does not
/// endorse under us, or no identity secret.
pub(crate) fn p_retirement<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    on: &BuiltOn<'_>,
    device_key: [u8; 32],
    last_seq: u64,
) -> Result<rekindle_types::governance::DeviceRetired, GovernanceRuntimeError> {
    let held = member_set(on, on.author)?;
    let record = (held.member_record.as_deref())
        .ok_or_else(|| GovernanceRuntimeError::MemberRecordMissing(on.author.to_hex()))?;
    let identity = Zeroizing::new(
        deps.identity_secret()
            .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?,
    );
    let p = derive::derive_community_pseudonym(&identity, community);
    device_writer::retire(&p, community, record, held, device_key, last_seq)
        .map_err(|_| GovernanceRuntimeError::DeviceActRefused("the key is not ours"))
}

fn readmission(
    on: &BuiltOn<'_>,
    member: &PseudonymKey,
    device_key: [u8; 32],
) -> Result<GovernanceEntry, GovernanceRuntimeError> {
    let held = member_set(on, member)?;
    let record = (held.member_record.as_deref())
        .ok_or_else(|| GovernanceRuntimeError::MemberRecordMissing(member.to_hex()))?;
    let endorsement = (held.endorsements.values())
        .filter(|e| e.device_key == device_key && e.answers_fork())
        .max_by_key(|e| e.incarnation)
        .ok_or(GovernanceRuntimeError::DeviceActRefused(
            "the member's set holds no fork-answering endorsement of the key",
        ))?;
    let body = device_writer::readmit(member, record, endorsement)
        .map_err(|_| GovernanceRuntimeError::DeviceActRefused("the key answers no fork"))?;
    Ok(GovernanceEntry::DeviceReadmit(body))
}

fn member_set<'a>(
    on: &BuiltOn<'a>,
    member: &PseudonymKey,
) -> Result<&'a MemberDevices, GovernanceRuntimeError> {
    on.state
        .devices
        .member(member)
        .ok_or(GovernanceRuntimeError::DeviceActRefused(
            "the member has no device set here",
        ))
}

/// The highest seq of `device_key` among our ops in the set the carrier
/// is built on: a `DeviceRetired` below its carrier's causal past is
/// invalid (rule 5), and every op of the key at or below it stays.
fn last_seq_of(on: &BuiltOn<'_>, device_key: [u8; 32]) -> u64 {
    (on.set.ops.iter())
        .map(rekindle_governance::op::VerifiedOp::op)
        .filter(|op| op.author == *on.author && op.device_key == device_key)
        .map(|op| op.seq)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "device_ops_tests.rs"]
mod tests;
