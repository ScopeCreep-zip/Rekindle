//! Who signs our governance ops (ADR 0022 rule 1, plan E3.2 S7d-keys.5/.6):
//! this device's per-community op key `d`, a random Ed25519 key minted on
//! this device and held only in its device vault, while the op's `author`
//! stays the community pseudonym `p`. `p` signs only the endorsement of
//! `d` (rule 10), carried by `d`'s seq-0 op (rule 3): the creator's in the
//! `GenesisCert`, a joiner's in its `MemberJoined`.
//!
//! A key is minted only where this device's first op in the community is
//! written (origin and join). Every later write needs the stored key: a
//! key the vault no longer holds is refused, never minted again over an
//! endorsement other members already hold (threat model T7, no creative
//! paths); re-enrolment is the `ReEnrolDevice` intent's (S7d-keys.10).

use rekindle_governance::device_writer::{self, DeviceRecord};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::DeviceEndorsement;
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// The two keys of one of our ops: `author` (`p`) names the member, and
/// `device` (`d`) signs.
pub struct OpSigner<'a> {
    pub author: PseudonymKey,
    pub device: &'a SigningKey,
}

/// This device's op key for `community`. Minted and stored when this
/// device holds none and `first_op` (the op being written is our
/// `MemberJoined`, which makes this device its member's first); otherwise
/// refused.
///
/// A held key under `first_op` is a re-join: the `MemberJoined` binds the
/// new `M(p)` as this device's op record (ADR 0022 rule 3, "The first
/// device's record is `M(p)`, bound by `MemberJoined` with owner `K_M(p)`"),
/// and the endorsement it carries signs that record (rule 10). The entry is
/// stored again with the key alone, the first device's entry, so the
/// `M(p,d)` an earlier membership's `DeviceKey` bound (signed for the old
/// member record, which rule 1 no longer accepts) never names our op
/// record or its owner again.
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceOpKeyMissing`] for a later op with no
/// stored key, or the device vault's failure.
pub fn device_op_key<D: GovernanceRuntimeDeps + ?Sized>(
    deps: &D,
    community: &str,
    first_op: bool,
) -> Result<SigningKey, GovernanceRuntimeError> {
    if let Some(key) = held_key(deps.device_op_key(community)?, community, first_op)? {
        if first_op {
            deps.store_device_op_key(community, &zeroize::Zeroizing::new(key.to_bytes()))?;
        }
        return Ok(key);
    }
    let mut secret = zeroize::Zeroizing::new([0u8; 32]);
    deps.rng().fill(secret.as_mut_slice());
    deps.store_device_op_key(community, &secret)?;
    Ok(SigningKey::from_bytes(&secret))
}

/// Our op record in `community`: the record our device key's endorsement
/// binds in `state`'s device set (ADR 0022 rule 3), where this device's
/// op chain lives. The member's first device's endorsement
/// ([`first_endorsement`]) binds `M(p)`, the record its set is signed in;
/// its ops land on our member record in `state` (the cert's, else our
/// admitted `MemberJoined`'s, a rejoin's new one included), as before any
/// second device. Any other device's endorsement binds its own `M(p,d)`.
/// The write path stores our ops under it and the outbox publishes them
/// on it.
///
/// # Errors
/// No membership, pseudonym or pinned cert, no stored device key
/// ([`GovernanceRuntimeError::DeviceOpKeyMissing`]), no endorsement of it
/// in `state` ([`GovernanceRuntimeError::DeviceUnendorsed`]: an unendorsed
/// key's ops land nowhere, never on `M(p)` by default), or a first device
/// whose member the state does not admit
/// ([`GovernanceRuntimeError::MemberRecordMissing`]).
pub fn own_op_record<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    state: &GovernanceState,
) -> Result<String, GovernanceRuntimeError> {
    let me = crate::op_log::own_member(deps, community, state)?;
    let device = device_op_key(deps, community, false)?;
    let member =
        (me.record).ok_or_else(|| GovernanceRuntimeError::MemberRecordMissing(me.hex.clone()))?;
    // Lead decision 63: only an endorsement signed for the current member
    // record binds; a key endorsed under an ended membership (before a
    // re-join) writes nothing until this device re-enrols.
    let endorsement = (state.devices.member(&me.pseudonym)).and_then(|md| {
        md.endorsement_for(
            &me.pseudonym,
            community,
            &member,
            &device.verifying_key().to_bytes(),
        )
    });
    match endorsement {
        None => Err(GovernanceRuntimeError::DeviceUnendorsed(
            community.to_string(),
        )),
        Some(e) if e.device_record == member => Ok(member),
        Some(e) => Ok(e.device_record.clone()),
    }
}

/// Whether this device may write in `community` (ADR 0022 rule 4, lead
/// decision 63): its key is endorsed for the current member record, or it
/// must (re-)enrol: no key held, or a key whose endorsement binds no
/// current record (a fresh device, or one whose person re-joined).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnDevice {
    /// Our key is endorsed for the current member record.
    Endorsed,
    /// Our ops are refused until this device enrols a fresh key.
    NeedsEnrol,
}

/// [`OwnDevice`] for `community` against `state`.
///
/// # Errors
/// As [`own_op_record`] except its two unendorsed refusals
/// (`DeviceUnendorsed`, `DeviceOpKeyMissing`), which are
/// [`OwnDevice::NeedsEnrol`].
pub fn own_device<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    state: &GovernanceState,
) -> Result<OwnDevice, GovernanceRuntimeError> {
    match own_op_record(deps, community, state) {
        Ok(_) => Ok(OwnDevice::Endorsed),
        Err(
            GovernanceRuntimeError::DeviceUnendorsed(_)
            | GovernanceRuntimeError::DeviceOpKeyMissing(_),
        ) => Ok(OwnDevice::NeedsEnrol),
        Err(e) => Err(e),
    }
}

/// The unlock catch-up's device check (ADR 0022 rule 4, lead decision 63):
/// when our held key no longer binds the current member record, the stale
/// `M(p,d)` binding is dropped (the entry keeps the key alone, which
/// [`own_op_record`] still refuses as `DeviceUnendorsed`), so no write
/// ever lands on a record of an ended membership. Answers our standing.
///
/// # Errors
/// As [`own_device`], or the device vault's.
pub fn catch_up_own_device<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    state: &GovernanceState,
) -> Result<OwnDevice, GovernanceRuntimeError> {
    let standing = own_device(deps, community, state)?;
    if standing == OwnDevice::NeedsEnrol {
        if let Some(held) = deps.device_op_key(community)? {
            deps.store_device_op_key(community, &held)?;
        }
    }
    Ok(standing)
}

/// Whether `e` endorses this device's held key in `community`.
///
/// # Errors
/// The device vault's.
pub fn is_own_key<D: GovernanceRuntimeDeps + ?Sized>(
    deps: &D,
    community: &str,
    e: &DeviceEndorsement,
) -> Result<bool, GovernanceRuntimeError> {
    Ok(deps.device_op_key(community)?.is_some_and(|held| {
        SigningKey::from_bytes(&held).verifying_key().to_bytes() == e.device_key
    }))
}

/// The stored key, `None` when one may be minted (this device's first op
/// in `community`), or the refusal of a later op whose key is gone.
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceOpKeyMissing`].
fn held_key(
    held: Option<zeroize::Zeroizing<[u8; 32]>>,
    community: &str,
    first_op: bool,
) -> Result<Option<SigningKey>, GovernanceRuntimeError> {
    match held {
        Some(secret) => Ok(Some(SigningKey::from_bytes(&secret))),
        None if first_op => Ok(None),
        None => Err(GovernanceRuntimeError::DeviceOpKeyMissing(
            community.to_string(),
        )),
    }
}

/// Put this device's first endorsement into `body` when it is a
/// `MemberJoined` without one (rule 3: a self-admitted joiner's first op
/// is its join); any other body stays as it is.
///
/// # Errors
/// As [`first_endorsement`].
pub fn carry_first_device(
    body: &mut rekindle_types::governance::GovernanceEntry,
    p: &SigningKey,
    community: &str,
    d: &SigningKey,
) -> Result<(), GovernanceRuntimeError> {
    if let rekindle_types::governance::GovernanceEntry::MemberJoined {
        member_record,
        member_record_owner,
        first_device: first @ None,
        ..
    } = body
    {
        *first = Some(first_endorsement(
            p,
            community,
            d,
            member_record,
            member_record_owner,
        )?);
    }
    Ok(())
}

/// `p`'s endorsement of this device's first key `d` in `community` for
/// member record `record` (owner text `owner`): the first device's record
/// is `M(p)` itself (rule 3), and it extends an empty set.
///
/// # Errors
/// Never for a first device (an empty set always extends).
pub fn first_endorsement(
    p: &SigningKey,
    community: &str,
    d: &SigningKey,
    record: &str,
    owner: &str,
) -> Result<DeviceEndorsement, GovernanceRuntimeError> {
    device_writer::endorse(
        p,
        community,
        record,
        None,
        d.verifying_key().to_bytes(),
        DeviceRecord {
            record: record.to_owned(),
            owner: owner.to_owned(),
        },
        [0; 32],
    )
    .map_err(|e| GovernanceRuntimeError::Crypto(format!("device endorsement: {e:?}")))
}

#[cfg(test)]
#[path = "device_signer_tests.rs"]
mod tests;
