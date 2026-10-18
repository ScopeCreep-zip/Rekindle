//! The daemon's surface over the session's record pool.
//!
//! Every DHT record operation goes through the session's
//! [`RecordPort`](rekindle_records::port::RecordPort) (plans C7, C9.1): the
//! pool owns opens, closes, watches, retries and the honest-write outcome.
//! This module lets a host reach it through the transport node, and writes
//! our own profile.

use tracing::{debug, info, warn};

use super::dht::channel_log::DhtLog;
use super::node::TransportNode;
use crate::error::Result;

// ── Our own profile (record pool) ──────────────────────────────────────

/// Write one subkey of our own profile through the session's record pool,
/// which holds the profile writable from unlock (plan C7.4). A write that
/// did not land at consensus is logged; it is not an error.
pub async fn set_own_profile_subkey(
    node: &TransportNode,
    profile_key: &str,
    subkey: u32,
    data: Vec<u8>,
) -> Result<()> {
    let outcome = rekindle_protocol::dht::profile::set_own_profile_subkey(
        &*node.require_records()?,
        profile_key,
        subkey,
        data,
    )
    .await?;
    if outcome.missed() {
        warn!(key = %profile_key, subkey, ?outcome, "dht: profile subkey not stored at consensus");
    } else {
        debug!(key = %profile_key, subkey, ?outcome, "dht: profile subkey written");
    }
    Ok(())
}

/// Write our own STATUS subkey through the session's record pool: a plain
/// write, never held for re-push (present-tense, plan C7.7j). A miss is
/// logged; the next status heartbeat writes it again.
pub async fn set_own_profile_status(
    node: &TransportNode,
    profile_key: &str,
    data: Vec<u8>,
) -> Result<()> {
    let outcome = rekindle_protocol::dht::profile::set_own_profile_status(
        &*node.require_records()?,
        profile_key,
        data,
    )
    .await?;
    if outcome.missed() {
        warn!(key = %profile_key, ?outcome, "dht: status not stored at consensus");
    }
    Ok(())
}

/// Widen sequence numbers to a dense `Vec<Option<u64>>`, keeping
/// never-written as `None` (a first write lands at seq 0).
fn seqs_as_opt_u64(seqs: &[Option<u32>]) -> Vec<Option<u64>> {
    seqs.iter().map(|seq| seq.map(u64::from)).collect()
}

// ── DhtLog (append-only log built on DHT records) ──────────────────────

/// Create a new DhtLog in the session's record pool. Returns
/// `(DhtLog, owner_keypair)`, the keypair as text; the caller releases the
/// log (`DhtLog::release`) when done.
pub async fn create_dht_log(node: &TransportNode) -> Result<(DhtLog, String)> {
    debug!("dht: create_dht_log");
    let result = DhtLog::create(&*node.require_records()?).await;
    match &result {
        Ok((log, _)) => info!(spine_key = %log.spine_key(), "dht: DhtLog created"),
        Err(e) => warn!(error = %e, "dht: DhtLog create failed"),
    }
    result.map_err(Into::into)
}

mod leased;
pub use leased::*;
