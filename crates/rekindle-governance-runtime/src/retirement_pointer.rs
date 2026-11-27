//! S7d-keys.9p2. The old GOV_HEAD only locates the retirement. Authority
//! comes from the landed DeviceReEnrol, with ADR 0022's device signature
//! and the two pseudonym signatures for the current member record.

use rekindle_codec::governance_op::{decode_op, DecodedOp};
use rekindle_governance::device::{endorsement_holds, retirement_holds};
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_records::chain::DeviceRetirementPointer;
use rekindle_records::port::{OpenIntent, RecordPort};
use rekindle_types::governance::{GovernanceEntry, OpHash, OpRef};

use crate::op_log::OwnMember;
use crate::{GovernanceRuntimeDeps, GovernanceRuntimeError};

/// Receive the signed seq-0 op named by our old record's retirement locator.
/// A well-formed locator or a DHT owner write alone gives no authority.
///
/// # Errors
/// The successor's log could not be read, or the op store failed.
pub async fn receive<D: GovernanceRuntimeDeps>(
    deps: &D,
    port: &dyn RecordPort,
    community: &str,
    old_record: &str,
    old_key: &[u8; 32],
    pointer: DeviceRetirementPointer,
) -> Result<bool, GovernanceRuntimeError> {
    let state = deps.governance_state(community).unwrap_or_default();
    let me = crate::op_log::own_member(deps, community, &state)?;
    let Some(member_record) = me.record.as_deref() else {
        return Ok(false);
    };
    let Some(old) = state.devices.member(&me.pseudonym).and_then(|devices| {
        devices.endorsement_for(&me.pseudonym, community, member_record, old_key)
    }) else {
        return Ok(false);
    };
    if old.device_record != old_record
        || pointer.successor_record == old_record
        || &pointer.successor_key == old_key
    {
        return Ok(false);
    }
    let locator = OpRef {
        op_hash: OpHash(pointer.op_hash),
        author: me.pseudonym.clone(),
        device_key: pointer.successor_key,
        author_seq: 0,
        author_record: pointer.successor_record,
    };
    let Some(encoded) = deps
        .op_store()
        .fetch_governance_op(&locator, &crate::op_walk::WalkLogCache::default())
        .await?
    else {
        return Ok(false);
    };
    let Ok(decoded) = decode_op(&encoded) else {
        return Ok(false);
    };
    let Some(owner) =
        verified_successor_owner(community, &me, &locator, old_key, old.incarnation, &decoded)
    else {
        return Ok(false);
    };
    let lease = port
        .acquire(&locator.author_record, None, OpenIntent::Check)
        .await
        .map_err(record_error)?;
    let actual = port.owner_of(lease);
    port.release(lease).await;
    if actual.map_err(record_error)? != owner {
        return Ok(false);
    }
    crate::op_receive::store_received_op(deps, community, encoded).await?;
    // Rebuild even on a duplicate: an earlier read may have been missing
    // a causal dependency of the carrier.
    Ok(true)
}

fn record_error(error: rekindle_records::port::PortError) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("retirement successor record: {error}"))
}

fn verified_successor_owner<'a>(
    community: &str,
    me: &OwnMember,
    locator: &OpRef,
    old_key: &[u8; 32],
    old_incarnation: u64,
    decoded: &'a DecodedOp,
) -> Option<&'a str> {
    let member_record = me.record.as_deref()?;
    let op = &decoded.op;
    let GovernanceEntry::DeviceReEnrol(re_enrol) = &op.body else {
        return None;
    };
    let retired = &re_enrol.retired;
    let successor = &re_enrol.endorsement;
    if op_hash(&decoded.unsigned, &op.sig) != locator.op_hash
        || op.author != me.pseudonym
        || op.device_key != locator.device_key
        || !re_enrol.carried_by(&op.device_key, op.seq)
        || retired.device_key != *old_key
        || retired.incarnation != old_incarnation
        || successor.device_record != locator.author_record
        || !retirement_holds(&me.pseudonym, community, member_record, retired)
        || !endorsement_holds(&me.pseudonym, community, member_record, successor)
    {
        return None;
    }
    rekindle_secrets::sign::verify_signature(
        &op.device_key,
        &op_signing_bytes(community, &decoded.unsigned),
        &op.sig,
    )
    .ok()?;
    Some(&successor.device_record_owner)
}

#[cfg(test)]
#[path = "retirement_pointer_tests.rs"]
mod tests;
