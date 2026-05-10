//! DHT record operations with confirmation lifecycle.
//!
//! Every write supports four confirmation levels: None, Accepted, Verified,
//! Propagated. The verification and propagation loops are implemented here —
//! services never implement read-back verification themselves.

use std::time::{Duration, Instant};

use rekindle_types::gossip_payload::GossipPayload;
use rekindle_types::transport::RecordSchema;

use super::{Confirm, PlatformIO, WriteReceipt};
use crate::ChatError;

/// Delay between write and verification read-back. Allows the DHT to
/// propagate the value to the node we'll read from.
const VERIFY_DELAY: Duration = Duration::from_millis(200);

/// Delay between first and second propagation check. Remote nodes need
/// time to receive and store the value.
const PROPAGATION_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Maximum time to wait for propagation confirmation before failing.
const PROPAGATION_TIMEOUT: Duration = Duration::from_secs(10);

impl PlatformIO {
    /// Write opaque bytes to a DHT record subkey with confirmation.
    ///
    /// - `Confirm::None`: fire and forget (transport.write_record only)
    /// - `Confirm::Accepted`: wait for transport Ok (default)
    /// - `Confirm::Verified`: read back after write, verify content matches
    /// - `Confirm::Propagated`: verified + inspect for remote holders
    pub async fn write_record(
        &self,
        key: &str,
        subkey: u32,
        data: &[u8],
        writer: Option<&[u8]>,
        confirm: Confirm,
    ) -> Result<WriteReceipt, ChatError> {
        let start = Instant::now();

        // Step 1: Write
        self.transport
            .write_record(key, subkey, data, writer)
            .await
            .map_err(ChatError::Transport)?;

        if confirm == Confirm::None {
            return Ok(WriteReceipt {
                key: key.to_string(),
                subkey,
                confirmed: Confirm::None,
                verified: false,
                remote_holders: 0,
                elapsed: start.elapsed(),
            });
        }

        if confirm == Confirm::Accepted {
            return Ok(WriteReceipt {
                key: key.to_string(),
                subkey,
                confirmed: Confirm::Accepted,
                verified: false,
                remote_holders: 0,
                elapsed: start.elapsed(),
            });
        }

        // Step 2: Verify (read back and compare)
        tokio::time::sleep(VERIFY_DELAY).await;

        let read_back = self
            .transport
            .read_record(key, subkey, true)
            .await
            .map_err(|e| ChatError::Internal(format!(
                "write verification read-back failed for {key} subkey {subkey}: {e}"
            )))?;

        let verified = if let Some(value) = &read_back {
            // The DHT may return a value with higher sequence number
            // from a concurrent writer. If the content doesn't match
            // our written data, it's a write conflict.
            if value.as_slice() != data {
                tracing::error!(
                    key = &key[..12.min(key.len())],
                    subkey,
                    written_len = data.len(),
                    readback_len = value.len(),
                    "DHT WRITE CONFLICT: read-back does not match written data"
                );
                return Err(ChatError::Internal(format!(
                    "DHT write conflict on {key} subkey {subkey}: \
                     written {} bytes, read back {} bytes — another writer superseded",
                    data.len(),
                    value.len(),
                )));
            }
            true
        } else {
            tracing::error!(
                key = &key[..12.min(key.len())],
                subkey,
                "DHT WRITE VERIFICATION FAILED: value not found after write"
            );
            return Err(ChatError::Internal(format!(
                "DHT write verification failed: {key} subkey {subkey} not found after write"
            )));
        };

        if confirm == Confirm::Verified {
            return Ok(WriteReceipt {
                key: key.to_string(),
                subkey,
                confirmed: Confirm::Verified,
                verified,
                remote_holders: 0,
                elapsed: start.elapsed(),
            });
        }

        // Step 3: Propagation (inspect for remote holders)
        let remote_holders = self
            .wait_for_propagation(key, subkey, start)
            .await?;

        Ok(WriteReceipt {
            key: key.to_string(),
            subkey,
            confirmed: Confirm::Propagated,
            verified,
            remote_holders,
            elapsed: start.elapsed(),
        })
    }

    /// Write to DHT then broadcast a gossip notification.
    ///
    /// Enforces the invariant: every DHT write that should notify peers
    /// DOES notify peers. The notification is part of the write call.
    ///
    /// If the DHT write fails, returns error immediately. No gossip sent.
    /// If the DHT write succeeds but gossip fails, logs warning and returns
    /// Ok — data is durable via DHT, peers discover via watch/poll.
    pub async fn write_and_notify(
        &self,
        community: &str,
        key: &str,
        subkey: u32,
        data: &[u8],
        writer: Option<&[u8]>,
        notification: GossipPayload,
        confirm: Confirm,
    ) -> Result<WriteReceipt, ChatError> {
        let receipt = self.write_record(key, subkey, data, writer, confirm).await?;

        // Gossip is best-effort after successful write.
        // The DHT write is the durability guarantee.
        match self.broadcast_gossip_dedup(community, notification).await {
            Ok(broadcast) => {
                if broadcast.peers_failed > 0 {
                    tracing::debug!(
                        community = &community[..12.min(community.len())],
                        sent = broadcast.peers_sent,
                        failed = broadcast.peers_failed,
                        "write_and_notify: partial gossip delivery — peers will discover via watch/poll"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    community = &community[..12.min(community.len())],
                    key = &key[..12.min(key.len())],
                    subkey,
                    error = %e,
                    "write_and_notify: gossip broadcast failed — DHT write succeeded, \
                     peers will discover via watch/poll"
                );
            }
        }

        Ok(receipt)
    }

    /// Read a DHT record subkey.
    pub async fn read_record(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, ChatError> {
        self.transport
            .read_record(key, subkey, force_refresh)
            .await
            .map_err(ChatError::Transport)
    }

    /// Create a new DHT record. Returns (record_key, owner_keypair_bytes).
    pub async fn create_record(
        &self,
        schema: RecordSchema,
    ) -> Result<(String, Vec<u8>), ChatError> {
        self.transport
            .create_record(schema)
            .await
            .map_err(ChatError::Transport)
    }

    /// Open an existing DHT record for reading or writing.
    pub async fn open_record(
        &self,
        key: &str,
        writer: Option<&[u8]>,
    ) -> Result<(), ChatError> {
        self.transport
            .open_record(key, writer)
            .await
            .map_err(ChatError::Transport)
    }

    /// Close a DHT record.
    pub async fn close_record(&self, key: &str) -> Result<(), ChatError> {
        self.transport
            .close_record(key)
            .await
            .map_err(ChatError::Transport)
    }

    /// Watch a DHT record for changes.
    pub async fn watch_record(
        &self,
        key: &str,
        subkeys: &[u32],
    ) -> Result<rekindle_types::transport::WatchToken, ChatError> {
        self.transport
            .watch_record(key, subkeys)
            .await
            .map_err(ChatError::Transport)
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
    ///
    /// This is the preferred method for all watch establishment. Direct
    /// `watch_record()` should only be used when custom error handling is
    /// needed beyond the standard warn-and-continue pattern.
    pub async fn watch_and_register(
        &self,
        key: &str,
        subkeys: &[u32],
        kind: crate::events::registry::WatchKind,
        watches: &crate::events::registry::WatchRegistry,
    ) -> Result<(), ChatError> {
        let token = self.watch_record(key, subkeys).await?;
        watches.register(key, kind, token);
        Ok(())
    }

    /// Join a community gossip mesh.
    ///
    /// After joining, gossip broadcasts for this community are forwarded
    /// to/from mesh peers. The transport manages mesh membership state.
    pub async fn join_mesh(&self, community_id: &str) -> Result<(), ChatError> {
        self.transport
            .join_mesh(community_id)
            .await
            .map_err(ChatError::Transport)
    }

    /// Leave a community gossip mesh.
    ///
    /// After leaving, gossip broadcasts for this community are no longer
    /// forwarded. Existing watches on community records remain active
    /// (watches are independent of mesh membership).
    pub async fn leave_mesh(&self, community_id: &str) -> Result<(), ChatError> {
        self.transport
            .leave_mesh(community_id)
            .await
            .map_err(ChatError::Transport)
    }

    /// Inspect a DHT record's subkey sequence numbers.
    pub async fn inspect_record(
        &self,
        key: &str,
        subkeys: &[u32],
    ) -> Result<Vec<Option<u32>>, ChatError> {
        self.transport
            .inspect_record(key, subkeys)
            .await
            .map_err(ChatError::Transport)
    }

    // ── Internal: propagation wait loop ────────────────────────

    async fn wait_for_propagation(
        &self,
        key: &str,
        subkey: u32,
        operation_start: Instant,
    ) -> Result<u32, ChatError> {
        let deadline = operation_start + PROPAGATION_TIMEOUT;

        loop {
            let seqs = self
                .transport
                .inspect_record(key, &[subkey])
                .await
                .map_err(|e| ChatError::Internal(format!(
                    "propagation inspect failed for {key} subkey {subkey}: {e}"
                )))?;

            // Count remote holders that have a sequence number
            let holders = u32::try_from(seqs.iter().filter(|s| s.is_some()).count()).unwrap_or(u32::MAX);

            if holders > 0 {
                tracing::debug!(
                    key = &key[..12.min(key.len())],
                    subkey,
                    remote_holders = holders,
                    elapsed_ms = operation_start.elapsed().as_millis(),
                    "propagation confirmed"
                );
                return Ok(holders);
            }

            if Instant::now() >= deadline {
                tracing::error!(
                    key = &key[..12.min(key.len())],
                    subkey,
                    timeout_ms = PROPAGATION_TIMEOUT.as_millis(),
                    "PROPAGATION FAILED: no remote holders after timeout — \
                     value may not be discoverable by other nodes"
                );
                return Err(ChatError::Internal(format!(
                    "DHT propagation failed for {key} subkey {subkey}: \
                     0 remote holders after {}s",
                    PROPAGATION_TIMEOUT.as_secs(),
                )));
            }

            tokio::time::sleep(PROPAGATION_RETRY_DELAY).await;
        }
    }
}
