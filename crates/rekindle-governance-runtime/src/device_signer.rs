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
/// device holds none and `first_op` (the op being written is this
/// device's first in the community); otherwise refused.
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
        return Ok(key);
    }
    let mut secret = zeroize::Zeroizing::new([0u8; 32]);
    deps.rng().fill(secret.as_mut_slice());
    deps.store_device_op_key(community, &secret)?;
    Ok(SigningKey::from_bytes(&secret))
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
