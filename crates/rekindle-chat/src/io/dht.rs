//! DHT record operations with confirmation lifecycle.
//!
//! Every write supports four confirmation levels: None, Accepted, Verified,
//! Propagated. The verification and propagation loops are implemented here —
//! services never implement read-back verification themselves.

use std::time::{Duration, Instant};

use rekindle_types::gossip_payload::GossipPayload;
use rekindle_types::transport::{OpenRecord, RecordSchema};

use super::{Confirm, PlatformIO, WriteReceipt};
use crate::ChatError;

/// Delay between write and verification read-back. Allows the DHT to
/// propagate the value to the node we'll read from.
const VERIFY_DELAY: Duration = Duration::from_millis(200);

/// Delay between first and second propagation check. Remote nodes need
/// time to receive and store the value.
const PROPAGATION_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Maximum time to wait for propagation confirmation before failing.
const PROPAGATION_TIMEOUT: Duration = Duration::from_secs(30);

impl PlatformIO {
    // ── Record operations with OpenRecord handles ──────────────

    /// Write opaque bytes to a DHT record subkey with confirmation.
    ///
    /// Stale handle retry: if the transport returns RecordFailed with
    /// "not open" in the error message, re-opens the record and retries
    /// once. This handles Veilid's internal record eviction after ~5min
    /// of inactivity.
    pub async fn write_record(
        &self,
        record: &OpenRecord,
        subkey: u32,
        data: &[u8],
        writer: Option<&[u8]>,
        confirm: Confirm,
    ) -> Result<WriteReceipt, ChatError> {
        let start = Instant::now();

        match self.transport.write_record(record, subkey, data, writer).await {
            Ok(()) => {}
            Err(ref e) if Self::is_stale_handle_error(e) => {
                tracing::debug!(
                    key = &record.key()[..12.min(record.key().len())],
                    subkey,
                    "write_record: stale handle — re-opening and retrying"
                );
                let reopened = self.transport
                    .open_record(record.key(), writer)
                    .await
                    .map_err(ChatError::Transport)?;
                self.transport
                    .write_record(&reopened, subkey, data, writer)
                    .await
                    .map_err(ChatError::Transport)?;
            }
            Err(e) => return Err(ChatError::Transport(e)),
        }

        if confirm == Confirm::None {
            return Ok(WriteReceipt {
                key: record.key().to_string(),
                subkey,
                confirmed: Confirm::None,
                verified: false,
                remote_holders: 0,
                elapsed: start.elapsed(),
            });
        }

        if confirm == Confirm::Accepted {
            return Ok(WriteReceipt {
                key: record.key().to_string(),
                subkey,
                confirmed: Confirm::Accepted,
                verified: false,
                remote_holders: 0,
                elapsed: start.elapsed(),
            });
        }

        // Verify: read back and compare
        tokio::time::sleep(VERIFY_DELAY).await;

        let read_back = self
            .transport
            .read_record(record, subkey, true)
            .await
            .map_err(|e| ChatError::Internal(format!(
                "write verification read-back failed for {} subkey {subkey}: {e}",
                record.key(),
            )))?;

        let verified = if let Some(value) = &read_back {
            if value.as_slice() != data {
                tracing::error!(
                    key = &record.key()[..12.min(record.key().len())],
                    subkey,
                    written_len = data.len(),
                    readback_len = value.len(),
                    "DHT WRITE CONFLICT: read-back does not match written data"
                );
                return Err(ChatError::Internal(format!(
                    "DHT write conflict on {} subkey {subkey}: \
                     written {} bytes, read back {} bytes — another writer superseded",
                    record.key(),
                    data.len(),
                    value.len(),
                )));
            }
            true
        } else {
            tracing::error!(
                key = &record.key()[..12.min(record.key().len())],
                subkey,
                "DHT WRITE VERIFICATION FAILED: value not found after write"
            );
            return Err(ChatError::Internal(format!(
                "DHT write verification failed: {} subkey {subkey} not found after write",
                record.key(),
            )));
        };

        if confirm == Confirm::Verified {
            return Ok(WriteReceipt {
                key: record.key().to_string(),
                subkey,
                confirmed: Confirm::Verified,
                verified,
                remote_holders: 0,
                elapsed: start.elapsed(),
            });
        }

        // Propagation: inspect for remote holders
        let remote_holders = self
            .wait_for_propagation(record, subkey, start)
            .await?;

        Ok(WriteReceipt {
            key: record.key().to_string(),
            subkey,
            confirmed: Confirm::Propagated,
            verified,
            remote_holders,
            elapsed: start.elapsed(),
        })
    }

    /// Write to DHT then broadcast a gossip notification.
    ///
    /// If the DHT write fails, returns error immediately — no gossip sent.
    /// If the DHT write succeeds but gossip fails, logs warning and returns
    /// Ok — data is durable via DHT, peers discover via watch/poll.
    pub async fn write_and_notify(
        &self,
        community: &str,
        record: &OpenRecord,
        subkey: u32,
        data: &[u8],
        writer: Option<&[u8]>,
        notification: GossipPayload,
        confirm: Confirm,
    ) -> Result<WriteReceipt, ChatError> {
        let receipt = self.write_record(record, subkey, data, writer, confirm).await?;

        match self.broadcast_gossip_dedup(community, notification).await {
            Ok(broadcast) => {
                if broadcast.peers_failed > 0 {
                    tracing::debug!(
                        community = &community[..12.min(community.len())],
                        sent = broadcast.peers_sent,
                        failed = broadcast.peers_failed,
                        "write_and_notify: partial gossip delivery"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    community = &community[..12.min(community.len())],
                    key = &record.key()[..12.min(record.key().len())],
                    subkey,
                    error = %e,
                    "write_and_notify: gossip broadcast failed — DHT write succeeded"
                );
            }
        }

        Ok(receipt)
    }

    /// Read a DHT record subkey. Retries once on stale handle.
    pub async fn read_record(
        &self,
        record: &OpenRecord,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, ChatError> {
        match self.transport.read_record(record, subkey, force_refresh).await {
            Ok(data) => Ok(data),
            Err(ref e) if Self::is_stale_handle_error(e) => {
                tracing::debug!(
                    key = &record.key()[..12.min(record.key().len())],
                    subkey,
                    "read_record: stale handle — re-opening and retrying"
                );
                let reopened = self.transport
                    .open_record(record.key(), None)
                    .await
                    .map_err(ChatError::Transport)?;
                self.transport
                    .read_record(&reopened, subkey, force_refresh)
                    .await
                    .map_err(ChatError::Transport)
            }
            Err(e) => Err(ChatError::Transport(e)),
        }
    }

    /// Create a new DHT record. Returns (OpenRecord handle, owner_keypair_bytes).
    /// The record is automatically tracked in the open-record cache.
    pub async fn create_record(
        &self,
        schema: RecordSchema,
    ) -> Result<(OpenRecord, Vec<u8>), ChatError> {
        let (record, keypair) = self.transport
            .create_record(schema)
            .await
            .map_err(ChatError::Transport)?;
        self.open_keys.lock().insert(record.key().to_string());
        Ok((record, keypair))
    }

    /// Open an existing DHT record. Returns an OpenRecord handle.
    ///
    /// Checks the open-record cache first — if `writer` is `None` and the
    /// key is already cached, constructs a handle without calling transport.
    /// If `writer` is `Some`, always calls transport — the caller may be
    /// upgrading from read-only to writable.
    pub async fn open_record(
        &self,
        key: &str,
        writer: Option<&[u8]>,
    ) -> Result<OpenRecord, ChatError> {
        if writer.is_none() && self.open_keys.lock().contains(key) {
            return Ok(OpenRecord::new(key.to_string()));
        }
        let record = self.transport
            .open_record(key, writer)
            .await
            .map_err(ChatError::Transport)?;
        self.open_keys.lock().insert(key.to_string());
        Ok(record)
    }

    /// Close a DHT record. Consumes the handle — prevents use-after-close.
    /// Removes the key from the open-record cache.
    pub async fn close_record(&self, record: OpenRecord) -> Result<(), ChatError> {
        self.open_keys.lock().remove(record.key());
        self.transport
            .close_record(record)
            .await
            .map_err(ChatError::Transport)
    }

    /// Close a record by key string. Used during `destroy_identity` where
    /// the caller iterates key strings from session_meta, not OpenRecord
    /// handles. Opens the record to get a handle, then closes it.
    pub async fn close_record_by_key(&self, key: &str) -> Result<(), ChatError> {
        let record = self.open_record(key, None).await?;
        self.close_record(record).await
    }

    /// Watch a DHT record for changes. Retries once on stale handle.
    pub async fn watch_record(
        &self,
        record: &OpenRecord,
        subkeys: &[u32],
    ) -> Result<rekindle_types::transport::WatchToken, ChatError> {
        match self.transport.watch_record(record, subkeys).await {
            Ok(token) => Ok(token),
            Err(ref e) if Self::is_stale_handle_error(e) => {
                tracing::debug!(
                    key = &record.key()[..12.min(record.key().len())],
                    "watch_record: stale handle — re-opening and retrying"
                );
                let reopened = self.transport
                    .open_record(record.key(), None)
                    .await
                    .map_err(ChatError::Transport)?;
                self.transport
                    .watch_record(&reopened, subkeys)
                    .await
                    .map_err(ChatError::Transport)
            }
            Err(e) => Err(ChatError::Transport(e)),
        }
    }

    /// Cancel a watch.
    pub async fn cancel_watch(
        &self,
        token: rekindle_types::transport::WatchToken,
    ) -> Result<(), ChatError> {
        self.transport
            .cancel_watch(token)
            .await
            .map_err(ChatError::Transport)
    }

    /// Watch a DHT record and register in the watch registry atomically.
    ///
    /// Combines `watch_record()` + `watches.register()` into a single call.
    /// If the watch fails, nothing is registered. If it succeeds, registration
    /// is guaranteed — impossible to forget the second step.
    pub async fn watch_and_register(
        &self,
        record: &OpenRecord,
        subkeys: &[u32],
        kind: crate::events::registry::WatchKind,
        watches: &crate::events::registry::WatchRegistry,
    ) -> Result<(), ChatError> {
        let token = self.watch_record(record, subkeys).await?;
        watches.register(record.key(), kind, token);
        Ok(())
    }

    /// Join a community gossip mesh.
    pub async fn join_mesh(&self, community_id: &str) -> Result<(), ChatError> {
        self.transport
            .join_mesh(community_id)
            .await
            .map_err(ChatError::Transport)
    }

    /// Leave a community gossip mesh.
    pub async fn leave_mesh(&self, community_id: &str) -> Result<(), ChatError> {
        self.transport
            .leave_mesh(community_id)
            .await
            .map_err(ChatError::Transport)
    }

    /// Inspect a DHT record's subkey sequence numbers. Retries once on stale handle.
    ///
    /// Returns both local and network sequence numbers:
    /// - `result.local_seqs` for local catch-up
    /// - `result.network_seqs` for propagation/discovery
    pub async fn inspect_record(
        &self,
        record: &OpenRecord,
        subkeys: &[u32],
    ) -> Result<rekindle_types::transport::InspectResult, ChatError> {
        match self.transport.inspect_record(record, subkeys).await {
            Ok(result) => Ok(result),
            Err(ref e) if Self::is_stale_handle_error(e) => {
                tracing::debug!(
                    key = &record.key()[..12.min(record.key().len())],
                    "inspect_record: stale handle — re-opening and retrying"
                );
                let reopened = self.transport
                    .open_record(record.key(), None)
                    .await
                    .map_err(ChatError::Transport)?;
                self.transport
                    .inspect_record(&reopened, subkeys)
                    .await
                    .map_err(ChatError::Transport)
            }
            Err(e) => Err(ChatError::Transport(e)),
        }
    }

    // ── Compound methods ──────────────────────────────────────
    //
    // Open + operate in one call. The caller never holds an OpenRecord.
    // The open-record cache ensures the open is a local HashSet lookup
    // when the record is already open.

    /// Open a record and write to it.
    pub async fn open_and_write(
        &self,
        key: &str,
        subkey: u32,
        data: &[u8],
        writer: Option<&[u8]>,
        confirm: Confirm,
    ) -> Result<WriteReceipt, ChatError> {
        let record = self.open_record(key, writer).await?;
        self.write_record(&record, subkey, data, writer, confirm).await
    }

    /// Open a record and read from it.
    pub async fn open_and_read(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, ChatError> {
        let record = self.open_record(key, None).await?;
        self.read_record(&record, subkey, force_refresh).await
    }

    /// Open a record and watch it, registering in the watch registry.
    pub async fn open_and_watch(
        &self,
        key: &str,
        subkeys: &[u32],
        kind: crate::events::registry::WatchKind,
        watches: &crate::events::registry::WatchRegistry,
    ) -> Result<(), ChatError> {
        let record = self.open_record(key, None).await?;
        self.watch_and_register(&record, subkeys, kind, watches).await
    }

    /// Open a record and inspect it.
    pub async fn open_and_inspect(
        &self,
        key: &str,
        subkeys: &[u32],
    ) -> Result<rekindle_types::transport::InspectResult, ChatError> {
        let record = self.open_record(key, None).await?;
        self.inspect_record(&record, subkeys).await
    }

    // ── Internal ──────────────────────────────────────────────

    async fn wait_for_propagation(
        &self,
        record: &OpenRecord,
        subkey: u32,
        operation_start: Instant,
    ) -> Result<u32, ChatError> {
        let deadline = operation_start + PROPAGATION_TIMEOUT;

        loop {
            let result = self
                .transport
                .inspect_record(record, &[subkey])
                .await
                .map_err(|e| ChatError::Internal(format!(
                    "propagation inspect failed for {} subkey {subkey}: {e}",
                    record.key(),
                )))?;

            let holders = u32::try_from(
                result.network_seqs.iter().filter(|s| s.is_some()).count()
            ).unwrap_or(u32::MAX);

            if holders > 0 {
                tracing::debug!(
                    key = &record.key()[..12.min(record.key().len())],
                    subkey,
                    remote_holders = holders,
                    elapsed_ms = operation_start.elapsed().as_millis(),
                    "propagation confirmed by remote nodes"
                );
                return Ok(holders);
            }

            if Instant::now() >= deadline {
                tracing::warn!(
                    key = &record.key()[..12.min(record.key().len())],
                    subkey,
                    timeout_ms = PROPAGATION_TIMEOUT.as_millis(),
                    "propagation unconfirmed: no remote holders within timeout — \
                     value is written, discovery may be delayed"
                );
                return Ok(0);
            }

            tokio::time::sleep(PROPAGATION_RETRY_DELAY).await;
        }
    }

    /// Check if a transport error indicates a stale record handle.
    /// Veilid returns "record not open" when a record was evicted
    /// from its internal table after inactivity (~5 minutes).
    fn is_stale_handle_error(e: &rekindle_types::transport::TransportError) -> bool {
        let msg = e.to_string();
        msg.contains("not open") || msg.contains("record not found")
    }
}
