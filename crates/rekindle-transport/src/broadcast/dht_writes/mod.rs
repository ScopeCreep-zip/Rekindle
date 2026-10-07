//! The daemon's string-typed surface over the session's record pool.
//!
//! Every DHT record operation goes through `rekindle_protocol`'s
//! `RecordPool` (plan C7): the pool owns opens, closes, watches, retries
//! and the honest-write outcome. This module only lets a host that cannot
//! name Veilid types (the daemon) reach it, and writes our own profile.

use tracing::{debug, info, warn};

use super::dht::channel_log::DhtLog;
use super::node::TransportNode;
use crate::error::{Result, TransportError};

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

/// Widen sequence numbers to a dense `Vec<Option<u64>>`, preserving the
/// never-written sentinel (`ValueSeqNum::NONE`) as `None`.
///
/// `ValueSeqNum` is a newtype over `Option<u32>`, so this goes through
/// `to_option()` rather than a numeric conversion.
fn seqs_as_opt_u64(seqs: &[veilid_core::ValueSeqNum]) -> Vec<Option<u64>> {
    seqs.iter()
        .map(|seq| seq.to_option().map(u64::from))
        .collect()
}

// ── DhtLog (append-only log built on DHT records) ──────────────────────

/// Create a new DhtLog in the session's record pool. Returns
/// `(DhtLog, owner_keypair)`; the caller releases the log
/// (`DhtLog::release`) when done.
pub async fn create_dht_log(node: &TransportNode) -> Result<(DhtLog, veilid_core::KeyPair)> {
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

// ── String-typed surface for hosts that cannot import veilid_core ─────
//
// `docs/architecture/daemon-cli.md` makes it a hard rule that only
// `rekindle-transport::broadcast/` and `subscriptions/` import
// `veilid_core`; `rekindle-node` and `rekindle-cli` reach Veilid only
// through this crate's public API. So the daemon cannot construct a
// `KeyPair` or name a `RecordKey`, yet it has to drive the same DHT
// operations as the Tauri host.
//
// `GovernanceRuntimeDeps` is built for exactly this: it exchanges every
// Veilid value as an opaque `String` / `Vec<u8>` ("Schwarzschild
// boundary — all Veilid types are exchanged as opaque String/Vec<u8>
// here"). These wrappers do the parsing on this side of the boundary so
// the daemon adapter can satisfy that trait without a veilid-core
// dependency. The Tauri host does its own parsing because it already
// holds a `RoutingContext` directly.

/// Parse a writer keypair in `KeyPair` string form.
fn parse_writer(s: &str) -> Result<veilid_core::KeyPair> {
    s.parse::<veilid_core::KeyPair>()
        .map_err(|e| TransportError::DhtError {
            reason: format!("invalid writer keypair: {e}"),
        })
}

/// Derive a member slot's writer keypair from the shared slot seed,
/// returned in string form.
///
/// Wraps `rekindle_protocol`'s derivation so the daemon does not need to
/// depend on that crate purely to obtain a Veilid keypair it cannot name.
pub fn derive_slot_keypair_str(seed: &[u8; 32], slot: u32) -> Result<String> {
    rekindle_protocol::dht::community::member_registry::derive_slot_veilid_keypair(seed, slot)
        .map(|kp| kp.to_string())
        .map_err(|e| TransportError::DhtError {
            reason: format!("derive slot keypair: {e}"),
        })
}

/// Convert an Ed25519 `(public, secret)` pair into writer-keypair string
/// form, for `GovernanceRuntimeDeps::format_writer_keypair`.
pub fn format_keypair_str(ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
    let bare_pub = veilid_core::BarePublicKey::new(&ed_public);
    let bare_secret = veilid_core::BareSecretKey::new(&ed_secret);
    let pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(pubkey, bare_secret).to_string()
}

#[cfg(test)]
mod seq_semantics_tests {
    use super::seqs_as_opt_u64;
    use veilid_core::ValueSeqNum;

    /// The fact every occupancy check in the workspace rests on.
    ///
    /// `ValueSeqNum` is `Option<u32>`, and veilid's **first** write to a
    /// subkey lands at seq 0 — so `Some(0)` is an occupied subkey, not an
    /// empty one. Two readers (`segments::highest_segment_full` and
    /// `dht_hydration`) had flattened `None` to `0` and then tested
    /// `!= 0`, which silently classified every write-once member as
    /// absent. If a future change reintroduces the flattening, this
    /// fails here rather than in a community that will not expand.
    #[test]
    fn seq_zero_is_written_not_empty() {
        let seqs = [
            ValueSeqNum::NONE,
            ValueSeqNum::from(0),
            ValueSeqNum::from(1),
            ValueSeqNum::NONE,
        ];
        assert_eq!(
            seqs_as_opt_u64(&seqs),
            vec![None, Some(0), Some(1), None],
            "seq 0 must survive as Some(0)"
        );
        assert_eq!(
            seqs_as_opt_u64(&seqs)
                .iter()
                .filter(|s| s.is_some())
                .count(),
            2,
            "occupancy counts the written-once subkey"
        );
    }
}
