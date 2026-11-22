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
    match op_record_of(state, &me.pseudonym, &device.verifying_key().to_bytes()) {
        None => Err(GovernanceRuntimeError::DeviceUnendorsed(
            community.to_string(),
        )),
        Some(OpRecord::Member) => me
            .record
            .ok_or(GovernanceRuntimeError::MemberRecordMissing(me.hex)),
        Some(OpRecord::Device(record)) => Ok(record),
    }
}

/// Where the endorsement of one device key binds its op chain.
#[derive(Debug, PartialEq, Eq)]
pub enum OpRecord {
    /// The member's first device: `M(p)`, the member record.
    Member,
    /// Any other device: its own `M(p,d)`.
    Device(String),
}

/// Where `p`'s endorsement of device key `key` in `state` binds its ops,
/// by the lowest incarnation's endorsement (rule 6's order; rule 3 gives
/// a key one record); `None` when `state` holds no endorsement of it.
#[must_use]
pub fn op_record_of(state: &GovernanceState, p: &PseudonymKey, key: &[u8; 32]) -> Option<OpRecord> {
    let md = state.devices.member(p)?;
    let (_, e) = md.endorsements.iter().find(|((_, k), _)| k == key)?;
    Some(if md.member_record.as_ref() == Some(&e.device_record) {
        OpRecord::Member
    } else {
        OpRecord::Device(e.device_record.clone())
    })
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
