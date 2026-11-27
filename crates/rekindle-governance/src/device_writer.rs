//! Authoring a member's device elements (ADR 0022 rules 1, 2, 4, 7 and
//! 10; plan E3.2 S7d-keys K5): what the pseudonym `p` signs to endorse or
//! retire one of its per-community device keys, and the `DeviceReadmit`
//! body a co-signer's device key carries.
//!
//! Every writer starts from the member's set as this device holds it (a
//! [`MemberDevices`] of the view at the deps of the op that will carry the
//! element), so an element is valid where it lands and
//! [`crate::device_judge::judge_op`] rules its key effective there:
//!
//! - an endorsement extends exactly the held set (`extendsSet`, rule 10)
//!   at an incarnation strictly above every element of it (rule 2);
//! - a retirement names the incarnation the key was endorsed at (rule 10);
//! - a readmit answers the same key, incarnation and fork evidence as the
//!   endorsement it admits (rule 10).

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    DeviceEndorsement, DeviceReadmit, DeviceRetired, MAX_DEVICE_SET_ELEMENTS,
};
use rekindle_types::id::PseudonymKey;

use crate::device::{device_set_hash, endorsement_signing_bytes, retirement_signing_bytes};
use crate::device_judge::MemberDevices;

/// Why an element cannot be written from the held set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceWriteError {
    /// The held set already has [`MAX_DEVICE_SET_ELEMENTS`] elements: one
    /// more endorsement would not fit its carrier.
    SetFull,
    /// The key to retire has no endorsement in the held set.
    NotEndorsed,
    /// A readmit answers a fork; this endorsement answers none.
    AnswersNoFork,
}

/// Where a new device key's record lives (rule 3): `M(p,d)` and the public
/// text of its owner `K_M(p,d)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub record: String,
    pub owner: String,
}

/// `p`'s endorsement of `device_key` in `community` for member record
/// `member_record` (`M(p)`), extending `held` (the member's set at the
/// carrier's deps; `None` for a first device). `fork_evidence` is the
/// fork it answers, or all zero.
///
/// # Errors
/// [`DeviceWriteError::SetFull`] when `held` cannot be extended in one op.
pub fn endorse(
    p: &SigningKey,
    community: &str,
    member_record: &str,
    held: Option<&MemberDevices>,
    device_key: [u8; 32],
    record: DeviceRecord,
    fork_evidence: [u8; 32],
) -> Result<DeviceEndorsement, DeviceWriteError> {
    // A BTreeSet iterates in the strict ascending order `well_formed` asks.
    let extends: Vec<_> = held
        .map(MemberDevices::elements)
        .unwrap_or_default()
        .into_iter()
        .collect();
    if extends.len() > MAX_DEVICE_SET_ELEMENTS {
        return Err(DeviceWriteError::SetFull);
    }
    let incarnation = extends.iter().map(|e| e.incarnation).max().unwrap_or(0) + 1;
    let mut e = DeviceEndorsement {
        device_key,
        incarnation,
        extends_set: device_set_hash(&extends),
        extends_elements: extends,
        device_record: record.record,
        device_record_owner: record.owner,
        fork_evidence,
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(community, member_record, &e);
    e.endorsement_sig = p.sign(&bytes).to_bytes().to_vec();
    Ok(e)
}

/// `p`'s endorsement of a member's first device `device_key` for the
/// member record `member_record` it binds (rule 3), extending an empty set
/// (a new member record starts its own set, lead decision 63) at an
/// incarnation above every element of `prior`, the member's elements this
/// device holds from an ended membership (`None` for a member new to the
/// community). Rule 2 keys the set by `(device key, incarnation)`, so a
/// re-joining device that keeps its key endorses it under a fresh
/// incarnation: the new record's element never shares an id with the one
/// signed for the ended record, which a union would keep only one of.
#[must_use]
pub fn endorse_first(
    p: &SigningKey,
    community: &str,
    prior: Option<&MemberDevices>,
    device_key: [u8; 32],
    record: DeviceRecord,
) -> DeviceEndorsement {
    let above = (prior
        .map(MemberDevices::elements)
        .unwrap_or_default()
        .iter())
    .map(|e| e.incarnation)
    .max()
    .unwrap_or(0);
    let mut e = DeviceEndorsement {
        device_key,
        incarnation: above + 1,
        extends_set: device_set_hash(&[]),
        extends_elements: Vec::new(),
        device_record: record.record,
        device_record_owner: record.owner,
        fork_evidence: [0; 32],
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(community, &e.device_record, &e);
    e.endorsement_sig = p.sign(&bytes).to_bytes().to_vec();
    e
}

/// `p`'s retirement of `device_key` above `last_seq` (rules 4, 5, 10), at
/// the incarnation `held` endorsed it at.
///
/// # Errors
/// [`DeviceWriteError::NotEndorsed`] when `held` has no endorsement of
/// `device_key`.
pub fn retire(
    p: &SigningKey,
    community: &str,
    member_record: &str,
    held: &MemberDevices,
    device_key: [u8; 32],
    last_seq: u64,
) -> Result<DeviceRetired, DeviceWriteError> {
    let author = PseudonymKey(p.verifying_key().to_bytes());
    let incarnation = held
        .endorsement_for(&author, community, member_record, &device_key)
        .map(|endorsement| endorsement.incarnation)
        .ok_or(DeviceWriteError::NotEndorsed)?;
    let mut r = DeviceRetired {
        device_key,
        incarnation,
        last_seq,
        retirement_sig: Vec::new(),
    };
    let bytes = retirement_signing_bytes(community, member_record, &r);
    r.retirement_sig = p.sign(&bytes).to_bytes().to_vec();
    Ok(r)
}

/// The `DeviceReadmit` body that admits `endorsement`, the fork-answering
/// endorsement of `member` (record `member_record`) (rules 7, 10). The op
/// carrying it is signed by the co-signer's own device key.
///
/// # Errors
/// [`DeviceWriteError::AnswersNoFork`] when `endorsement` answers no fork.
pub fn readmit(
    member: &PseudonymKey,
    member_record: &str,
    endorsement: &DeviceEndorsement,
) -> Result<DeviceReadmit, DeviceWriteError> {
    if !endorsement.answers_fork() {
        return Err(DeviceWriteError::AnswersNoFork);
    }
    Ok(DeviceReadmit {
        member: member.clone(),
        member_record: member_record.to_owned(),
        new_key: endorsement.device_key,
        incarnation: endorsement.incarnation,
        fork_evidence: endorsement.fork_evidence,
    })
}

#[cfg(test)]
#[path = "device_writer_tests.rs"]
mod tests;
