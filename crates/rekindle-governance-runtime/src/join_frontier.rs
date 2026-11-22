//! The joiner's read of the inviter's frontier against what the invite link
//! names (plan IV1b, lead decision (28)).
//!
//! The link carries the inviter's `InviteCreated` op by ref and the seq its
//! frontier (`M`/3) was written at when it handed the link out. A forced
//! read can still settle on an older frontier: Veilid 0.5.8 ends a get once
//! `get_value_count` nodes hold the newest value it has seen
//! (`storage_manager/get_value.rs:49-88`, `check_done`), so the closest
//! replicas that still agree on the pre-invite frontier answer it. That
//! frontier does not reach the invite, and the join would refuse it as
//! unknown. So the read is the hinted one (VU3, its seq returned), the link's
//! seq is its minimum, and a frontier below it has the op fetched by its ref
//! from the inviter's op log with the gap read `read_log_reaching` (plan
//! E3.3 D23: the head from the newest value a transaction's node set holds).
//! No wait and no repeated read: an op the gap read does not reach either is
//! an error that names the stale read, never `InviteUnknown`, which stays the
//! answer of a frontier that met the minimum.

use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::Frontier;
use rekindle_types::invite::InviteLink;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::op_log::HeldOp;

/// A member's `Frontier` (`M`/3) as a hinted read gave it: its seq, the
/// frontier, and a short hash of its bytes for the logs.
pub struct FrontierRead {
    pub seq: u64,
    pub frontier: Frontier,
    pub short_hash: String,
}

/// A member's `Frontier` (`M`/3), read from the network (VU3's hinted read:
/// forced, no inspect first), with the seq it was read at.
///
/// # Errors
/// The record is unreadable, holds no frontier, or it does not decode.
pub async fn read_frontier<D: GovernanceRuntimeDeps>(
    deps: &D,
    record: &str,
) -> Result<FrontierRead, GovernanceRuntimeError> {
    let lease = deps.acquire_record(record, None, OpenIntent::Check).await?;
    let read = deps.read_hinted_value(lease, member_record::FRONTIER).await;
    deps.release_record(lease).await;
    let (seq, bytes) = read?.ok_or_else(|| {
        GovernanceRuntimeError::Adapter(format!("member record {record} has no frontier"))
    })?;
    let short_hash = hex::encode(&blake3::hash(&bytes).as_bytes()[..4]);
    let frontier = rekindle_codec::governance_op::decode_frontier(&bytes)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("frontier of {record}: {e}")))?;
    Ok(FrontierRead {
        seq,
        frontier,
        short_hash,
    })
}

/// The ops the join's walk starts holding beside the frontier's heads: the
/// link's `InviteCreated`, fetched by its hash and seq from the inviter's op
/// record `inviter_record`, when `read` is older than the frontier seq the
/// link was minted at; nothing when it met that minimum. The link names no
/// signing device key, so the held op takes the one the op carries.
///
/// # Errors
/// The frontier is older than the link's and the op is not on the
/// inviter's op log as the gap read found it, or that read failed.
pub async fn invite_op_seed<D: GovernanceRuntimeDeps>(
    deps: &D,
    link: &InviteLink,
    inviter_record: &str,
    read: &FrontierRead,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    if read.seq >= u64::from(link.frontier_seq) {
        return Ok(Vec::new());
    }
    let port = deps.record_port()?;
    let seq = link.invite_op_seq;
    let log = rekindle_records::chain::read_log_reaching(
        &*port,
        inviter_record,
        member_record::GOV_HEAD,
        seq,
        seq + 1,
    )
    .await
    .map_err(|e| GovernanceRuntimeError::Adapter(format!("read the inviter's op log: {e}")))?;
    let op = (log.entries.into_iter())
        .find(|entry| entry.seq == seq + 1)
        .and_then(|entry| {
            crate::op_receive::held_at(link.invite_op, inviter_record, entry.ciphertext)
        });
    tracing::info!(
        community = %link.anchor.as_str(),
        frontier_seq = read.seq,
        minimum = link.frontier_seq,
        log_last_seq = log.last_seq,
        found = op.is_some(),
        "join: the inviter's frontier is older than the invite, its InviteCreated fetched by ref"
    );
    op.map(|op| vec![op]).ok_or_else(|| {
        GovernanceRuntimeError::Adapter(format!(
            "the inviter's frontier read (seq {}) is older than the invite (seq {}) \
             and its op log did not reach the invite",
            read.seq, link.frontier_seq
        ))
    })
}
