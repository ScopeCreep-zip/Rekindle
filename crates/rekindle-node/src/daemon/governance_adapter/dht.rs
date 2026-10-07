//! DHT paths.
//!
//! Every function here goes through `rekindle_transport::broadcast::
//! dht_writes`' string-typed surface. That is not indirection for its
//! own sake: `docs/architecture/daemon-cli.md` makes it a hard rule that
//! only `rekindle-transport::broadcast/` and `subscriptions/` import
//! `veilid_core`, and `xtask check-boundaries` enforces it. This crate
//! therefore cannot construct a `KeyPair` or name a `RecordKey` — which
//! is exactly why `GovernanceRuntimeDeps` exchanges them as opaque
//! `String`s ("Schwarzschild boundary" in its own module docs).
//!
//! The Tauri adapter reaches `RoutingContext` directly and does its own
//! parsing; the two tracks meet at the trait, not at the Veilid types.

use rekindle_governance_runtime::deps::DhtRecordInfo;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_transport::broadcast::dht_writes::{self, LeaseId};

use super::DaemonGovernanceAdapter;

/// Map a transport failure into the trait's error type.
///
/// `Adapter` rather than a bespoke variant: these are environmental
/// failures (node down, record unreachable, bad writer) and the message
/// is what reaches the IPC client.
fn dht_err(context: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("{context}: {e}"))
}

impl DaemonGovernanceAdapter<'_> {
    /// Create the universal v2.0 community SMPL record (`o_cnt: 0`), held
    /// writable under its creator lease.
    pub(super) async fn create_smpl_record_impl(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        let node = self.transport()?;
        let (lease, record_key, owner) = dht_writes::create_smpl_leased(&node, member_pubkeys)
            .await
            .map_err(|e| dht_err("create SMPL record", e))?;
        Ok(DhtRecordInfo {
            record_key,
            owner_keypair: Some(owner),
            lease,
        })
    }

    /// Create a single-owner DFLT(1) record for one invite's encrypted
    /// `InviteSecrets` blob. The owner keypair is returned: the caller needs
    /// it to write the blob to subkey 0.
    pub(super) async fn create_dflt_record_impl(
        &self,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        let node = self.transport()?;
        let (lease, record_key, owner) = dht_writes::create_dflt_leased(&node, None)
            .await
            .map_err(|e| dht_err("create DFLT record", e))?;
        Ok(DhtRecordInfo {
            record_key,
            owner_keypair: Some(owner),
            lease,
        })
    }

    /// Create a governance **overflow** page owned by an HKDF-derived
    /// keypair the caller already holds. The record key is not
    /// re-derivable (veilid mixes a random encryption key in and refuses to
    /// recreate an existing owner+schema record), so the caller persists it
    /// in the `overflow_next` chain and reuses it. Create runs at most once
    /// per page.
    pub(super) async fn create_overflow_record_impl(
        &self,
        owner_keypair: String,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        let node = self.transport()?;
        let (lease, record_key, owner) =
            dht_writes::create_dflt_leased(&node, Some(&owner_keypair))
                .await
                .map_err(|e| dht_err("create overflow record", e))?;
        Ok(DhtRecordInfo {
            record_key,
            owner_keypair: Some(owner),
            lease,
        })
    }

    pub(super) fn format_writer_keypair_impl(ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        dht_writes::format_keypair_str(ed_public, ed_secret)
    }

    pub(super) async fn acquire_record_impl(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<LeaseId, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::acquire_str(&node, record_key, writer.as_deref())
            .await
            .map_err(|e| dht_err("acquire record", e))
    }

    pub(super) async fn release_record_impl(&self, lease: LeaseId) {
        if let Ok(node) = self.transport() {
            dht_writes::release(&node, lease).await;
        }
    }

    /// Hold the community's leases (releasing a repeat), then watch its
    /// records. The daemon runs no per-community loops: the
    /// `SubscriptionManager` covers its inspect and presence tiers.
    pub(super) async fn community_records_ready_impl(
        &self,
        community_id: &str,
        leases: rekindle_records::lease::CommunityLeases,
    ) {
        let Ok(node) = self.transport() else {
            return;
        };
        let merged = self
            .ctx
            .community_runtime
            .hold_leases(community_id, leases, |l| dht_writes::key_of(&node, l));
        for lease in merged.surplus {
            dht_writes::release(&node, lease).await;
        }
        self.watch_community_records_impl(community_id).await;
        crate::daemon::keepalive::start(self.ctx, community_id);
    }

    pub(super) async fn get_dht_value_impl(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::get_leased(&node, lease, subkey, force_refresh)
            .await
            .map_err(|e| dht_err("get_dht_value", e))
    }

    /// Write a subkey. `Ok(None)` means the value is stored; `Ok(Some(stale))`
    /// means the network already held a newer value and ours was superseded
    /// (the slot claim's compare-and-swap, join step 9). A write that did not
    /// reach consensus is an error.
    pub(super) async fn set_dht_value_impl(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::set_leased_str(&node, lease, subkey, value, writer.as_deref())
            .await
            .map_err(|e| dht_err("set_dht_value", e))
    }

    /// Sequence numbers from this node's local cache — no network I/O.
    pub(super) async fn inspect_local_seqs_impl(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::inspect_leased_local_seqs(&node, lease)
            .await
            .map_err(|e| dht_err("inspect (local)", e))
    }

    /// Network-confirmed sequence numbers. What a slot claim must use: the
    /// local cache can show a subkey free that another member has taken.
    pub(super) async fn inspect_network_seqs_impl(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<Option<u64>>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::inspect_leased_network_seqs(&node, lease)
            .await
            .map_err(|e| dht_err("inspect (network)", e))
    }

    /// Indices of subkeys that hold a value, network-confirmed. Keeps the
    /// "never written" vs "written at seq 0" distinction, so a cold join
    /// fetches only occupied slots.
    pub(super) async fn inspect_present_subkeys_impl(
        &self,
        lease: LeaseId,
    ) -> Result<Vec<u32>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::inspect_leased_present_subkeys(&node, lease)
            .await
            .map_err(|e| dht_err("inspect (present subkeys)", e))
    }

    /// Watch every record the community holds (governance, registry,
    /// channels, segments, overflow) on its lease, every subkey of each:
    /// which member writes next is not known in advance under SMPL, and a
    /// DFLT overflow page has its own count. The pool re-arms a dead watch
    /// while the lease is held (plan C7.7i).
    ///
    /// Best-effort by design — a failed watch degrades to the poll and
    /// inspect paths (three-path delivery), so a single unreachable
    /// record must not abort the whole subscription pass.
    pub(super) async fn watch_community_records_impl(&self, community_id: &str) {
        let Ok(node) = self.transport() else {
            return;
        };
        let scope = self.ctx.unlock_scope_or_closed();
        for lease in self.ctx.community_runtime.held_leases(community_id) {
            // Before each watch call: the unlock is ending, or the join
            // phase this runs in is out of budget (plan C4.L1b).
            if scope.is_closed() || rekindle_governance_runtime::phase_expired() {
                break;
            }
            if let Err(e) = dht_writes::watch_all_leased(&node, lease).await {
                tracing::debug!(
                    community_id,
                    record = ?dht_writes::key_of(&node, lease),
                    error = %e,
                    "governance adapter: watch failed, falling back to poll/inspect"
                );
            }
        }
    }
}
