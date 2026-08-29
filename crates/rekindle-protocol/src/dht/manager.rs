//! `DHTManager` record operations: create/open/close, get/set, SMPL,
//! open-or-create, and watches.

use std::collections::{HashMap, HashSet};

use veilid_core::{
    DHTSchema, DHTSchemaSMPLMember, RoutingContext, SetDHTValueOptions, ValueSubkeyRangeSet,
    CRYPTO_KIND_VLD0,
};

use super::retry::{classify_dht_open_error, parse_record_key, retry_on_unreachable};
use super::{DHTManager, OpenOrCreateResult, DEFAULT_DHT_OPEN_ATTEMPTS, DEFAULT_DHT_OPEN_DELAY};
use crate::error::ProtocolError;

impl DHTManager {
    pub fn new(routing_context: RoutingContext) -> Self {
        Self {
            routing_context,
            profile_key: None,
            friend_list_key: None,
            route_cache: HashMap::new(),
            open_records: HashSet::new(),
            imported_routes: HashMap::new(),
            route_id_to_pubkey: HashMap::new(),
            default_writer: None,
        }
    }

    /// Set a default writer keypair that will be used for all `set_value` calls.
    ///
    /// This ensures writes succeed even if the record's default writer is lost
    /// due to a concurrent read-only re-open by another code path.
    pub fn with_writer(mut self, writer: veilid_core::KeyPair) -> Self {
        self.default_writer = Some(writer);
        self
    }

    /// Create a new DHT record with DFLT schema (single owner).
    ///
    /// Returns `(record_key, owner_keypair)`. The `owner_keypair` is the randomly
    /// generated keypair that owns this record — it **must** be persisted and passed
    /// back to [`open_record_writable`] on subsequent sessions to retain write access.
    pub async fn create_record(
        &self,
        subkey_count: u32,
    ) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
        let count = u16::try_from(subkey_count).map_err(|_| {
            ProtocolError::DhtError(format!("subkey_count {subkey_count} exceeds u16::MAX"))
        })?;
        let schema = DHTSchema::dflt(count)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;

        let descriptor = self
            .routing_context
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create_dht_record: {e}")))?;

        let key_string = descriptor.key().to_string();
        // Extract the owner keypair so the caller can persist it for future writes.
        // owner_secret() is Some immediately after create — Veilid generates a random
        // keypair and passes it as the writer.
        let owner_keypair = descriptor.owner_secret().map(|secret| {
            veilid_core::KeyPair::new_from_parts(descriptor.owner().clone(), secret.value())
        });

        tracing::debug!(key = %key_string, has_keypair = owner_keypair.is_some(), "created DHT record");
        Ok((key_string, owner_keypair))
    }

    /// Create a new DHT record with DFLT schema using a specific owner keypair.
    ///
    /// Unlike [`create_record`] which uses a random keypair, this uses the
    /// provided `owner` keypair, making the record key deterministic for that keypair.
    /// Returns `(record_key, owner_keypair)`.
    pub async fn create_record_with_owner(
        &self,
        subkey_count: u32,
        owner: veilid_core::KeyPair,
    ) -> Result<(String, veilid_core::KeyPair), ProtocolError> {
        let count = u16::try_from(subkey_count).map_err(|_| {
            ProtocolError::DhtError(format!("subkey_count {subkey_count} exceeds u16::MAX"))
        })?;
        let schema = DHTSchema::dflt(count)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;

        let descriptor = self
            .routing_context
            .create_dht_record(CRYPTO_KIND_VLD0, schema, Some(owner.clone()))
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create_dht_record_with_owner: {e}")))?;

        let key_string = descriptor.key().to_string();
        tracing::debug!(key = %key_string, "created DHT record with specific owner");
        Ok((key_string, owner))
    }

    /// Open an existing DHT record for **reading only** (no writer set).
    ///
    /// Use [`open_record_writable`] instead when you need to write to records you own.
    pub async fn open_record(&self, key: &str) -> Result<(), ProtocolError> {
        let record_key = parse_record_key(key)?;

        let _descriptor = self
            .routing_context
            .open_dht_record(record_key, None)
            .await
            .map_err(|e| classify_dht_open_error("open_dht_record", &e))?;

        tracing::debug!(key, "opened DHT record (read-only)");
        Ok(())
    }

    /// Open an existing DHT record **with write access** by providing the owner keypair.
    ///
    /// The `writer` must be the same keypair returned by [`create_record`] when the
    /// record was originally created. Without it, Veilid's `set_dht_value` will fail
    /// with "value is not writable".
    pub async fn open_record_writable(
        &self,
        key: &str,
        writer: veilid_core::KeyPair,
    ) -> Result<(), ProtocolError> {
        let record_key = parse_record_key(key)?;

        let _descriptor = self
            .routing_context
            .open_dht_record(record_key, Some(writer))
            .await
            .map_err(|e| classify_dht_open_error("open_dht_record (writable)", &e))?;

        tracing::debug!(key, "opened DHT record (writable)");
        Ok(())
    }

    /// [`open_record_writable`] with bounded retry on the TRANSIENT
    /// `DhtRecordUnreachable` case (veilid `KeyNotFound`/`TryAgain` from a
    /// sparse routing table on a freshly-attached node). Hard errors return
    /// immediately. After `attempts` exhaust, the last (still-transient)
    /// error is returned so the caller can treat the record as genuinely gone
    /// (and recreate). Mirrors `allocate_route_with_retry` — the default is
    /// 15 × 3 s ≈ 45 s. Pass `delay = Duration::ZERO` to disable sleeping (tests).
    pub async fn open_record_writable_with_retry(
        &self,
        key: &str,
        writer: veilid_core::KeyPair,
        attempts: u32,
        delay: std::time::Duration,
    ) -> Result<(), ProtocolError> {
        retry_on_unreachable(attempts, delay, || {
            self.open_record_writable(key, writer.clone())
        })
        .await
    }

    /// Read-only sibling of [`open_record_writable_with_retry`].
    pub async fn open_record_with_retry(
        &self,
        key: &str,
        attempts: u32,
        delay: std::time::Duration,
    ) -> Result<(), ProtocolError> {
        retry_on_unreachable(attempts, delay, || self.open_record(key)).await
    }

    /// Close a DHT record.
    pub async fn close_record(&self, key: &str) -> Result<(), ProtocolError> {
        let record_key = parse_record_key(key)?;

        self.routing_context
            .close_dht_record(record_key)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("close_dht_record: {e}")))?;

        tracing::debug!(key, "closed DHT record");
        Ok(())
    }

    /// Get a subkey value from a DHT record.
    ///
    /// Returns `None` if the subkey has not been set yet.
    pub async fn get_value(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<Vec<u8>>, ProtocolError> {
        let record_key = parse_record_key(key)?;

        let value = self
            .routing_context
            .get_dht_value(record_key, subkey, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("get_dht_value: {e}")))?;

        Ok(value.map(|v| v.data().to_vec()))
    }

    /// Read a subkey value, bypassing the local DHT cache.
    ///
    /// Uses `force_refresh = true` to always fetch from the network.
    /// Use this for data that changes frequently (e.g. member presence)
    /// where stale cached values cause incorrect behavior.
    pub async fn get_value_fresh(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<Vec<u8>>, ProtocolError> {
        let record_key = parse_record_key(key)?;

        let value = self
            .routing_context
            .get_dht_value(record_key, subkey, true)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("get_dht_value(fresh): {e}")))?;

        Ok(value.map(|v| v.data().to_vec()))
    }

    /// Set a subkey value on a DHT record we own.
    ///
    /// If a [`default_writer`](Self::with_writer) is set, it will be passed
    /// explicitly via `SetDHTValueOptions` to avoid relying on the record's
    /// default writer (which can be clobbered by concurrent read-only opens).
    pub async fn set_value(
        &self,
        key: &str,
        subkey: u32,
        value: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        let record_key = parse_record_key(key)?;

        let options = self
            .default_writer
            .as_ref()
            .map(|writer| SetDHTValueOptions {
                writer: Some(writer.clone()),
                ..Default::default()
            });

        self.routing_context
            .set_dht_value(record_key, subkey, value, options)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("set_dht_value: {e}")))?;

        Ok(())
    }

    /// Create a new DHT record with SMPL schema (multi-writer).
    ///
    /// The owner controls `owner_subkey_count` subkeys. Each member in `members`
    /// can write to `m_cnt` subkeys assigned after the owner's subkeys.
    ///
    /// Returns `(record_key, owner_keypair)`. The `owner_keypair` is the randomly
    /// generated keypair that owns this record — it **must** be persisted.
    pub async fn create_smpl_record(
        &self,
        owner_subkey_count: u16,
        members: Vec<DHTSchemaSMPLMember>,
    ) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
        let schema = DHTSchema::smpl(owner_subkey_count, members)
            .map_err(|e| ProtocolError::DhtError(format!("SMPL schema: {e}")))?;

        let descriptor = self
            .routing_context
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create_dht_record (SMPL): {e}")))?;

        let key_string = descriptor.key().to_string();
        let owner_keypair = descriptor.owner_secret().map(|secret| {
            veilid_core::KeyPair::new_from_parts(descriptor.owner().clone(), secret.value())
        });

        tracing::debug!(
            key = %key_string,
            has_keypair = owner_keypair.is_some(),
            "created SMPL DHT record"
        );
        Ok((key_string, owner_keypair))
    }

    /// Set a subkey value using an explicit writer keypair (for SMPL multi-writer).
    ///
    /// Used when a member needs to write to their assigned subkey in a SMPL
    /// record — the `writer` must match the `BareMemberId` in the schema.
    pub async fn set_value_with_writer(
        &self,
        key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: veilid_core::KeyPair,
    ) -> Result<(), ProtocolError> {
        let record_key = parse_record_key(key)?;

        let options = SetDHTValueOptions {
            writer: Some(writer),
            ..Default::default()
        };

        self.routing_context
            .set_dht_value(record_key, subkey, value, Some(options))
            .await
            .map_err(|e| ProtocolError::DhtError(format!("set_dht_value (writer): {e}")))?;

        Ok(())
    }

    /// Try to reopen an existing DHT record with write access, falling back to
    /// creating a new one if the open fails or no key/keypair is available.
    ///
    /// Returns the resolved key, effective keypair, and whether a new record was
    /// created. When `is_new` is true, the caller **must** persist the keypair.
    pub async fn open_or_create_record(
        &self,
        existing_key: Option<&str>,
        owner_keypair: Option<veilid_core::KeyPair>,
        subkey_count: u32,
        label: &str,
    ) -> Result<OpenOrCreateResult, ProtocolError> {
        if let (Some(key), Some(keypair)) = (existing_key, owner_keypair) {
            // Retry the open on TRANSIENT unreachability (sparse routing table
            // on a fresh node) before giving up — otherwise a momentary
            // KeyNotFound would orphan the real record and churn its key.
            match self
                .open_record_writable_with_retry(
                    key,
                    keypair.clone(),
                    DEFAULT_DHT_OPEN_ATTEMPTS,
                    DEFAULT_DHT_OPEN_DELAY,
                )
                .await
            {
                Ok(()) => {
                    tracing::info!(key, label, "reusing existing DHT record");
                    return Ok(OpenOrCreateResult {
                        key: key.to_string(),
                        keypair: Some(keypair),
                        is_new: false,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        key, label, error = %e,
                        "failed to open existing DHT record after retries — creating new one"
                    );
                }
            }
        } else if existing_key.is_some() {
            tracing::warn!(
                label,
                "no owner keypair for existing record — creating new one"
            );
        }

        let (key, keypair) = self.create_record(subkey_count).await?;
        tracing::info!(key = %key, label, "created new DHT record");
        Ok(OpenOrCreateResult {
            key,
            keypair,
            is_new: true,
        })
    }

    /// Watch specific subkeys on a DHT record for changes.
    ///
    /// Returns `true` if the watch is active, `false` if it was cancelled.
    pub async fn watch_record(&self, key: &str, subkeys: &[u32]) -> Result<bool, ProtocolError> {
        let record_key = parse_record_key(key)?;

        // Build a ValueSubkeyRangeSet from the provided subkey indices
        let subkey_range: ValueSubkeyRangeSet = subkeys.iter().copied().collect();

        let active = self
            .routing_context
            .watch_dht_values(record_key, Some(subkey_range), None, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("watch_dht_values: {e}")))?;

        tracing::debug!(key, ?subkeys, active, "watching DHT record");
        Ok(active)
    }
}
