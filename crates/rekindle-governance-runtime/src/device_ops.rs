//! Our device-key acts in a community (ADR 0022, plan E3.2 S7d-keys K5):
//! `p` retires one of our device keys (rules 4 and 5), or our device key
//! carries the re-admission of a member's fork-answering key (rules 7 and
//! 8). Each element is built by [`device_writer`] under the rebuild lock
//! from the state and op set its carrier is built on
//! ([`apply::write_built`]), so it is valid where it lands.

use rekindle_governance::device_judge::MemberDevices;
use rekindle_governance::device_writer;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use zeroize::Zeroizing;

use crate::apply::{self, Held};
use crate::deps::{GovernanceRuntimeDeps, OpOutcome};
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
        Box::new(move |deps: &D, on: &Held<'_>| retirement(deps, &anchor, on, device_key));
    apply::write_built(deps, community, build).await
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
        Box::new(move |_: &D, on: &Held<'_>| readmission(on, &member, device_key));
    apply::write_built(deps, community, build).await
}

fn retirement<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    on: &Held<'_>,
    device_key: [u8; 32],
) -> Result<GovernanceEntry, GovernanceRuntimeError> {
    if let Some(ours) = deps.device_op_key(community)? {
        if SigningKey::from_bytes(&ours).verifying_key().to_bytes() == device_key {
            return Err(GovernanceRuntimeError::DeviceActRefused(
                "this device's own key is re-enrolled, not retired",
            ));
        }
    }
    let held = member_set(on, on.author)?;
    let record = (held.member_record.as_deref())
        .ok_or_else(|| GovernanceRuntimeError::MemberRecordMissing(on.author.to_hex()))?;
    let last_seq = last_seq_of(on, device_key);
    let identity = Zeroizing::new(
        deps.identity_secret()
            .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?,
    );
    let p = derive::derive_community_pseudonym(&identity, community);
    let retired = device_writer::retire(&p, community, record, held, device_key, last_seq)
        .map_err(|_| GovernanceRuntimeError::DeviceActRefused("the key is not ours"))?;
    Ok(GovernanceEntry::DeviceRetired(retired))
}

fn readmission(
    on: &Held<'_>,
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
    on: &Held<'a>,
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
fn last_seq_of(on: &Held<'_>, device_key: [u8; 32]) -> u64 {
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
