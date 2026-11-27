//! Re-enrolling this device (ADR 0022 rules 4 and 9, plan E3.2
//! S7d-keys.10): after the unlock barrier found a restore, or another copy
//! of this data root retired our key, the user commits the
//! backend-originated `ReEnrolDevice` review and this device writes, in
//! each community, one `DeviceReEnrol{retired, endorsement}` op signed by
//! a fresh key at its seq 0 on the fresh key's own `M(p,d)`: `p` retires
//! the copied key at `lastSeq` and endorses the new one. Nothing here runs
//! without that commit (rule 9: no automatic recovery).

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::display_catch_up::PendingGovKind;
use rekindle_types::governance::{DeviceReEnrol, GovernanceEntry};

use crate::apply::{self, BuiltOn};
use crate::deps::{GovernanceRuntimeDeps, OpOutcome};
use crate::device_ops;
use crate::error::GovernanceRuntimeError;
use crate::unlock_barrier::BarrierPhase;

/// Whether this device must re-enrol in `community`: the barrier found a
/// restore, or the held set retires our key (another copy's
/// `DeviceRetired`, S7d-keys.9).
///
/// # Errors
/// The device vault's, or no pseudonym.
pub fn needed<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
) -> Result<bool, GovernanceRuntimeError> {
    if matches!(
        deps.unlock_barrier(community).phase(),
        BarrierPhase::Restored(_)
    ) {
        return Ok(true);
    }
    Ok(retired_from(deps, community)?.is_some())
}

/// The sequence from which this device's installed key is retired.
///
/// # Errors
/// Reading the held device key or deriving our pseudonym failed.
pub fn retired_from<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
) -> Result<Option<u64>, GovernanceRuntimeError> {
    let (Some(state), Some(held)) = (
        deps.governance_state(community),
        deps.device_op_key(community)?,
    ) else {
        return Ok(None);
    };
    let key = SigningKey::from_bytes(&held).verifying_key().to_bytes();
    let (author, _) = crate::op_log::own_pseudonym(deps, community)?;
    Ok(state
        .devices
        .member(&author)
        .and_then(|member| member.retired_from(&key)))
}

/// The `lastSeq` our old key is retired at: the barrier's maximum after a
/// restore (S7d-keys.8, IC4), else this copy's own local head of the key
/// (a clone signal, rule 4: "lastSeq = its own local head").
///
/// # Errors
/// No held key, or the op store's.
pub async fn last_seq<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
) -> Result<u64, GovernanceRuntimeError> {
    if let BarrierPhase::Restored(fact) = deps.unlock_barrier(community).phase() {
        return Ok(fact.last_seq);
    }
    let old = held_public(deps, community)?;
    let head = deps.op_store().signed_here_head(community, &old).await?;
    Ok(head.unwrap_or(0))
}

/// Re-enrol this device in `community`: mint a fresh key and record, store
/// the key (the old one never signs again), pass the barrier for it, and
/// write the `DeviceReEnrol` built under the rebuild lock.
///
/// # Errors
/// [`GovernanceRuntimeError::DeviceActRefused`] when the held set does not
/// endorse our key here (nothing is minted), or the record's, the vault's
/// or the write's.
pub async fn re_enrol<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
) -> Result<OpOutcome, GovernanceRuntimeError> {
    let old = held_public(deps, community)?;
    let (author, _) = crate::op_log::own_pseudonym(deps, community)?;
    let endorsed = deps.governance_state(community).is_some_and(|state| {
        (state.devices.member(&author))
            .is_some_and(|md| md.endorsements.values().any(|e| e.device_key == old))
    });
    if !endorsed {
        return Err(GovernanceRuntimeError::DeviceActRefused(
            "this device's key is not endorsed here",
        ));
    }
    let last_seq = last_seq(deps, community).await?;
    let (secret, record, owner_keypair) = device_ops::mint_device(deps).await?;
    deps.store_device_op_key(community, &secret)?;
    deps.unlock_barrier(community).re_keyed();
    let anchor = community.to_owned();
    let build: apply::Build<D> = Box::new(move |deps: &D, on: &BuiltOn<'_>| {
        let bound = record.record.clone();
        let retired = device_ops::p_retirement(deps, &anchor, on, old, last_seq)?;
        let endorsement = device_ops::new_endorsement(deps, &anchor, on, record, &secret)?;
        deps.store_device_record(&anchor, &bound, &owner_keypair)?;
        Ok(GovernanceEntry::DeviceReEnrol(DeviceReEnrol {
            retired,
            endorsement,
        }))
    });
    apply::write_built(deps, community, build, PendingGovKind::DeviceReEnrol).await
}

/// Our held device key's public half in `community`.
fn held_public<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
) -> Result<[u8; 32], GovernanceRuntimeError> {
    let held = (deps.device_op_key(community)?)
        .ok_or_else(|| GovernanceRuntimeError::DeviceOpKeyMissing(community.to_owned()))?;
    Ok(SigningKey::from_bytes(&held).verifying_key().to_bytes())
}
