//! Leased records over the session's record port: every record operation
//! names a lease from the unlocked session's pool (plan C7.5), with keys
//! and keypairs as text (plan C9.1).

use std::sync::Arc;

use rekindle_records::port::{RecordPort, ReportScope};

use super::super::node::TransportNode;
use super::seqs_as_opt_u64;
use crate::error::{Result, TransportError};

pub use rekindle_records::lease::LeaseId;

fn pool(node: &TransportNode) -> Result<Arc<dyn RecordPort>> {
    node.require_records()
}

/// Borrow `record_key`, writable with `writer` (keypair string form) when
/// given.
pub async fn acquire_str(
    node: &TransportNode,
    record_key: &str,
    writer: Option<&str>,
) -> Result<LeaseId> {
    Ok(pool(node)?.acquire(record_key, writer).await?)
}

/// End a borrow; the record closes when it was the last. A no-op once the
/// session is locked (the pool closed every record).
pub async fn release(node: &TransportNode, lease: LeaseId) {
    if let Ok(pool) = pool(node) {
        pool.release(lease).await;
    }
}

/// The record key a held lease borrows.
pub fn key_of(node: &TransportNode, lease: LeaseId) -> Option<String> {
    pool(node).ok()?.key_of(lease)
}

/// Create the universal v2.0 community SMPL record (`o_cnt: 0`) from raw
/// member public keys, held writable. Returns the creator lease, the
/// record key and the owner keypair string.
pub async fn create_smpl_leased(
    node: &TransportNode,
    member_pubkeys: &[[u8; 32]],
) -> Result<(LeaseId, String, String)> {
    let schema = rekindle_records::schema::community_smpl(member_pubkeys);
    Ok(pool(node)?.create(&schema, None).await?)
}

/// Create a single-owner DFLT(1) record owned by `owner` (a fresh key when
/// `None`), held writable. Returns the creator lease, the record key and
/// the owner keypair string.
pub async fn create_dflt_leased(
    node: &TransportNode,
    owner: Option<&str>,
) -> Result<(LeaseId, String, String)> {
    let schema = rekindle_records::schema::invite_secrets();
    Ok(pool(node)?.create(&schema, owner).await?)
}

/// Read a subkey of a leased record.
pub async fn get_leased(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    let value = pool(node)?.get(lease, subkey, force_refresh).await?;
    Ok(value.map(|v| v.data))
}

/// The value the network held instead of ours when a write was superseded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewerValue {
    pub seq: u32,
    pub data: Vec<u8>,
}

/// Write a subkey of a leased record as `writer`. `Ok(Some(newer))` when the
/// network already held a newer value (the compare-and-swap outcome);
/// `Ok(None)` when the value is stored or was already ours. A write that did
/// not reach consensus is an error, never a silent success.
pub async fn set_leased_str(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    data: Vec<u8>,
    writer: Option<&str>,
) -> Result<Option<NewerValue>> {
    use rekindle_records::port::SetOutcome;
    match pool(node)?.set(lease, subkey, data, writer).await? {
        SetOutcome::Superseded(newer) => Ok(Some(NewerValue {
            seq: newer.seq,
            data: newer.data,
        })),
        SetOutcome::Landed | SetOutcome::Unchanged => Ok(None),
        missed @ (SetOutcome::BelowConsensus | SetOutcome::Offline) => {
            Err(TransportError::DhtError {
                reason: format!("set: not stored ({missed:?})"),
            })
        }
    }
}

/// Local-cache sequence numbers of a leased record (no network I/O).
pub async fn inspect_leased_local_seqs(
    node: &TransportNode,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>> {
    let report = pool(node)?.inspect(lease, None, ReportScope::Local).await?;
    Ok(seqs_as_opt_u64(&report.local_seqs))
}

/// Network-confirmed sequence numbers of a leased record.
pub async fn inspect_leased_network_seqs(
    node: &TransportNode,
    lease: LeaseId,
) -> Result<Vec<Option<u64>>> {
    let report = pool(node)?
        .inspect(lease, None, ReportScope::UpdateGet)
        .await?;
    Ok(seqs_as_opt_u64(&report.network_seqs))
}

/// Indices of a leased record's subkeys that hold a value,
/// network-confirmed.
pub async fn inspect_leased_present_subkeys(
    node: &TransportNode,
    lease: LeaseId,
) -> Result<Vec<u32>> {
    let report = pool(node)?
        .inspect(lease, None, ReportScope::UpdateGet)
        .await?;
    Ok(report
        .subkeys
        .iter()
        .zip(&report.network_seqs)
        .filter(|(_, seq)| seq.is_some())
        .map(|(subkey, _)| *subkey)
        .collect())
}

/// Subkeys (of `subkeys`) that hold a value in the local copy after an
/// UpdateGet inspect of a leased record: the inbox and subscription-poll
/// scan's test. Not the pool's `inspect_present` (newer on the network
/// than locally), which answers a different question.
pub async fn inspect_leased_local_present(
    node: &TransportNode,
    lease: LeaseId,
    subkeys: &[u32],
) -> Result<Vec<u32>> {
    let report = pool(node)?
        .inspect(lease, Some(subkeys), ReportScope::UpdateGet)
        .await?;
    Ok(report
        .subkeys
        .iter()
        .zip(&report.local_seqs)
        .filter(|(_, seq)| seq.is_some())
        .map(|(subkey, _)| *subkey)
        .collect())
}

/// Watch `subkeys` of a leased record; the pool watches the union of every
/// borrower's subkeys and re-arms a dead watch while one is held.
pub async fn watch_leased(node: &TransportNode, lease: LeaseId, subkeys: &[u32]) -> Result<()> {
    Ok(pool(node)?.watch(lease, subkeys).await?)
}

/// Watch every subkey of a leased record (its schema's count).
pub async fn watch_all_leased(node: &TransportNode, lease: LeaseId) -> Result<()> {
    Ok(pool(node)?.watch_all(lease).await?)
}

/// Read one subkey of `record_key` with a borrow of its own: a table hit
/// when the session holds the record, else an open and a close.
pub async fn read_once_str(
    node: &TransportNode,
    record_key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>> {
    let value = pool(node)?
        .read_once(record_key, subkey, force_refresh)
        .await?;
    Ok(value.map(|v| v.data))
}
