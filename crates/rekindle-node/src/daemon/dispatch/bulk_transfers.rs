//! Bulk transfer state tracking for control-plane observability.
//!
//! The actual bulk data flows through the lane 0x01–0x02 wire protocol,
//! not through IpcRequest. These data structures track transfer lifecycle
//! metadata so that `rekindle transfer status` can report progress.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// State of a single bulk transfer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransferState {
    pub transfer_id: String,
    pub stream_id: u8,
    pub direction: String,
    pub total_size: u64,
    pub bytes_transferred: u64,
    pub media_type: String,
    pub digest: String,
    pub status: TransferStatus,
    #[serde(skip)]
    pub started_at: Instant,
    /// Elapsed seconds since transfer started.
    pub elapsed_secs: f64,
    /// Connection that started this transfer.
    #[serde(skip)]
    pub conn_id: u64,
    /// Nonce counter from the connection's BulkSession.
    #[serde(skip)]
    pub nonce_counter: Option<Arc<crate::ipc::bulk::nonce::NonceCounter>>,
    /// Digest algorithm for this transfer.
    pub digest_algorithm: crate::ipc::bulk::verify::DigestAlgorithm,
}

/// Transfer lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferStatus {
    Active,
    Completed,
    Cancelled,
    Failed,
}

/// Registry of active and recent bulk transfers.
///
/// Lives on `DaemonContext` behind a `parking_lot::Mutex`.
/// Low contention: only the dispatch thread writes; status queries read.
pub struct BulkTransferRegistry {
    transfers: HashMap<String, TransferState>,
    next_stream_id: u8,
}

impl BulkTransferRegistry {
    pub fn new() -> Self {
        Self {
            transfers: HashMap::new(),
            next_stream_id: 0,
        }
    }

    /// Register a new transfer. Returns the allocated stream_id.
    pub fn start(
        &mut self,
        transfer_id: String,
        total_size: u64,
        media_type: String,
        digest: String,
        direction: String,
        conn_id: u64,
        nonce_counter: Option<Arc<crate::ipc::bulk::nonce::NonceCounter>>,
        digest_algorithm: crate::ipc::bulk::verify::DigestAlgorithm,
    ) -> u8 {
        self.next_stream_id = self.next_stream_id.wrapping_add(1);
        if self.next_stream_id == 0 {
            self.next_stream_id = 1;
        }
        let stream_id = self.next_stream_id;

        self.transfers.insert(
            transfer_id.clone(),
            TransferState {
                transfer_id,
                stream_id,
                direction,
                total_size,
                bytes_transferred: 0,
                media_type,
                digest,
                status: TransferStatus::Active,
                started_at: Instant::now(),
                conn_id,
                nonce_counter,
                digest_algorithm,
                elapsed_secs: 0.0,
            },
        );
        stream_id
    }

    /// Mark a transfer as completed.
    pub fn complete(&mut self, transfer_id: &str, bytes_transferred: u64) -> bool {
        if let Some(state) = self.transfers.get_mut(transfer_id) {
            state.status = TransferStatus::Completed;
            state.bytes_transferred = bytes_transferred;
            state.elapsed_secs = state.started_at.elapsed().as_secs_f64();
            true
        } else {
            false
        }
    }

    /// Mark a transfer as cancelled.
    pub fn cancel(&mut self, transfer_id: &str) -> bool {
        if let Some(state) = self.transfers.get_mut(transfer_id) {
            state.status = TransferStatus::Cancelled;
            state.elapsed_secs = state.started_at.elapsed().as_secs_f64();
            true
        } else {
            false
        }
    }

    /// Get a transfer's current state.
    pub fn status(&self, transfer_id: &str) -> Option<TransferState> {
        self.transfers.get(transfer_id).map(|s| {
            let mut snapshot = s.clone();
            snapshot.elapsed_secs = s.started_at.elapsed().as_secs_f64();
            snapshot
        })
    }

    /// List all transfers (active and recent).
    pub fn list(&self) -> Vec<TransferState> {
        self.transfers
            .values()
            .map(|s| {
                let mut snapshot = s.clone();
                snapshot.elapsed_secs = s.started_at.elapsed().as_secs_f64();
                snapshot
            })
            .collect()
    }

    /// Remove completed/cancelled/failed transfers older than `max_age`.
    pub fn gc(&mut self, max_age: std::time::Duration) {
        self.transfers.retain(|_, s| {
            s.status == TransferStatus::Active || s.started_at.elapsed() < max_age
        });
    }

    /// Number of active transfers.
    pub fn active_count(&self) -> usize {
        self.transfers
            .values()
            .filter(|s| s.status == TransferStatus::Active)
            .count()
    }
}

impl Default for BulkTransferRegistry {
    fn default() -> Self {
        Self::new()
    }
}
