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
use rekindle_transport::broadcast::dht_writes;

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
    /// Create the universal v2.0 community SMPL record.
    ///
    /// `o_cnt: 0` — the creation keypair owns no subkeys and is
    /// discarded after genesis, so `owner_keypair` comes back `None`.
    /// That absence is the Schwarzschild principle in effect, not a
    /// failure to capture something.
    pub(super) async fn create_smpl_record_impl(
        &self,
        member_pubkeys: &[[u8; 32]],
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        let node = self.transport()?;
        let (record_key, owner_keypair) = dht_writes::create_smpl_str(&node, member_pubkeys)
            .await
            .map_err(|e| dht_err("create SMPL record", e))?;
        Ok(DhtRecordInfo {
            record_key,
            owner_keypair,
        })
    }

    /// Create a single-owner DFLT(1) record for one invite's encrypted
    /// `InviteSecrets` blob. The owner keypair IS returned here — the
    /// caller needs it to write the blob to subkey 0.
    pub(super) async fn create_dflt_record_impl(
        &self,
    ) -> Result<DhtRecordInfo, GovernanceRuntimeError> {
        let node = self.transport()?;
        let (record_key, owner_keypair) = dht_writes::create_dflt_str(&node, 1, None)
            .await
            .map_err(|e| dht_err("create DFLT record", e))?;
        Ok(DhtRecordInfo {
            record_key,
            owner_keypair,
        })
    }

    /// Create a governance **overflow** page owned by an HKDF-derived
    /// keypair the caller already holds.
    ///
    /// Only the key is returned: the record key is not re-derivable
    /// (veilid mixes a random encryption key in and refuses to recreate
    /// an existing owner+schema record), so the caller persists it in
    /// the `overflow_next` chain and reuses it via `open_dht_record`.
    /// Create runs at most once per page.
    pub(super) async fn create_overflow_record_impl(
        &self,
        owner_keypair: String,
    ) -> Result<String, GovernanceRuntimeError> {
        let node = self.transport()?;
        let (record_key, _) = dht_writes::create_dflt_str(&node, 1, Some(&owner_keypair))
            .await
            .map_err(|e| dht_err("create overflow record", e))?;
        Ok(record_key)
    }

    pub(super) fn format_writer_keypair_impl(ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        dht_writes::format_keypair_str(ed_public, ed_secret)
    }

    pub(super) async fn get_dht_value_impl(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::get(&node, record_key, subkey, force_refresh)
            .await
            .map_err(|e| dht_err("get_dht_value", e))
    }

    /// Write a subkey.
    ///
    /// `Ok(None)` means the write landed; `Ok(Some(stale))` means the
    /// network already held a newer value and ours was superseded. That
    /// outcome only became observable when `record::set` stopped
    /// discarding veilid's return — without it the slot claim's
    /// compare-and-swap (join step 9) cannot detect a lost race.
    pub(super) async fn set_dht_value_impl(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::set_str(&node, record_key, subkey, value, writer.as_deref())
            .await
            .map_err(|e| dht_err("set_dht_value", e))
    }

    /// Sequence numbers from this node's local cache — no network I/O.
    pub(super) async fn inspect_local_seqs_impl(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::inspect_local_seqs(&node, record_key)
            .await
            .map_err(|e| dht_err("inspect (local)", e))
    }

    /// Network-confirmed sequence numbers. What a slot claim must use:
    /// the local cache can show a subkey free that another member has
    /// already taken.
    pub(super) async fn inspect_network_seqs_impl(
        &self,
        record_key: &str,
    ) -> Result<Vec<u64>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::inspect_network_seqs(&node, record_key)
            .await
            .map_err(|e| dht_err("inspect (network)", e))
    }

    /// Indices of subkeys that hold a value, network-confirmed.
    ///
    /// Keeps the "never written" vs "written at seq 0" distinction the
    /// seq forms flatten, so a cold join fetches only occupied slots
    /// instead of 255 serial round trips.
    pub(super) async fn inspect_present_subkeys_impl(
        &self,
        record_key: &str,
    ) -> Result<Vec<u32>, GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::inspect_present_subkeys(&node, record_key)
            .await
            .map_err(|e| dht_err("inspect (present subkeys)", e))
    }

    pub(super) async fn open_dht_record_impl(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<(), GovernanceRuntimeError> {
        let node = self.transport()?;
        dht_writes::open_str(&node, record_key, writer.as_deref())
            .await
            .map_err(|e| dht_err("open_dht_record", e))
    }

    /// Subscribe to a community's records: governance, registry, and
    /// every channel log we know about.
    ///
    /// Best-effort by design — a failed watch degrades to the poll and
    /// inspect paths (three-path delivery), so a single unreachable
    /// record must not abort the whole subscription pass.
    pub(super) async fn watch_community_records_impl(&self, community_id: &str) {
        let Ok(node) = self.transport() else {
            return;
        };
        let Some(membership) = self.community_membership_impl(community_id) else {
            return;
        };

        let mut keys: Vec<String> = Vec::new();
        keys.extend(membership.governance_key.clone());
        keys.extend(membership.member_registry_key.clone());
        keys.extend(membership.channel_log_keys.values().cloned());
        keys.extend(self.governance_overflow_keys_impl(community_id));

        for key in keys {
            // Watch every subkey: which member writes next is not known
            // in advance under SMPL.
            let subkeys: Vec<u32> = (0..super::SLOT_WATCH_WIDTH).collect();
            if let Err(e) = dht_writes::watch(&node, &key, &subkeys).await {
                tracing::debug!(
                    community_id,
                    record = %key,
                    error = %e,
                    "governance adapter: watch failed, falling back to poll/inspect"
                );
            }
        }
    }
}
