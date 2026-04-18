//! SMPL write retry queue with exponential backoff.
//!
//! Addresses Gap A from rekindle-architecture-v2.md §15.
//!
//! Messages are enqueued for DHT write and processed by a background task.
//! On failure: exponential backoff (1s → 2s → 4s), max 3 attempts.
//! After exhaustion, the write is reported as failed (caller updates message status).

use std::time::Duration;

use tokio::sync::mpsc;
use tracing;

/// A pending SMPL write waiting to be executed.
#[derive(Debug, Clone)]
pub struct PendingWrite {
    /// DHT record key (string-encoded TypedKey).
    pub record_key: String,
    /// Subkey index within the SMPL record.
    pub subkey: u32,
    /// Serialized data to write.
    pub data: Vec<u8>,
    /// Current attempt number (0-indexed).
    pub attempt: u32,
}

/// Result of a write attempt.
#[derive(Debug)]
pub enum WriteResult {
    /// Write succeeded.
    Success { record_key: String, subkey: u32 },
    /// Write failed after all retries.
    Exhausted {
        record_key: String,
        subkey: u32,
        last_error: String,
    },
}

/// Handle for enqueuing writes. Cheap to clone.
#[derive(Clone)]
pub struct WriteQueueHandle {
    tx: mpsc::Sender<PendingWrite>,
}

impl WriteQueueHandle {
    /// Enqueue a write for background processing. Returns immediately.
    pub async fn enqueue(&self, record_key: String, subkey: u32, data: Vec<u8>) {
        let pending = PendingWrite {
            record_key,
            subkey,
            data,
            attempt: 0,
        };
        if self.tx.send(pending).await.is_err() {
            tracing::error!("write retry queue closed — write dropped");
        }
    }
}

/// Maximum retry attempts before giving up.
pub const MAX_RETRIES: u32 = 3;

/// Backoff duration for a given attempt (0-indexed).
/// 0 → 1s, 1 → 2s, 2 → 4s.
pub fn backoff_duration(attempt: u32) -> Duration {
    Duration::from_secs(1 << attempt.min(4))
}

/// Create a write queue channel pair.
///
/// Returns `(handle, receiver)`. The caller should spawn a background task
/// that drains the receiver and performs DHT writes via the routing context.
///
/// The actual write logic lives in the Tauri layer (Phase 2) because it
/// needs a `RoutingContext`, which is a veilid-core runtime type. This crate
/// provides the queue infrastructure and backoff policy.
pub fn create_write_queue(buffer: usize) -> (WriteQueueHandle, mpsc::Receiver<PendingWrite>) {
    let (tx, rx) = mpsc::channel(buffer);
    (WriteQueueHandle { tx }, rx)
}

/// Re-enqueue a failed write with incremented attempt counter.
///
/// Returns `None` if max retries exhausted (caller should report failure).
pub fn retry_or_fail(write: &PendingWrite) -> Option<PendingWrite> {
    if write.attempt + 1 >= MAX_RETRIES {
        return None;
    }
    Some(PendingWrite {
        record_key: write.record_key.clone(),
        subkey: write.subkey,
        data: write.data.clone(),
        attempt: write.attempt + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_schedule() {
        assert_eq!(backoff_duration(0), Duration::from_secs(1));
        assert_eq!(backoff_duration(1), Duration::from_secs(2));
        assert_eq!(backoff_duration(2), Duration::from_secs(4));
    }

    #[test]
    fn retry_increments_attempt() {
        let write = PendingWrite {
            record_key: "k".into(),
            subkey: 0,
            data: vec![1, 2, 3],
            attempt: 0,
        };
        let retried = retry_or_fail(&write).unwrap();
        assert_eq!(retried.attempt, 1);
        assert_eq!(retried.data, write.data);
    }

    #[test]
    fn retry_exhausted_returns_none() {
        let write = PendingWrite {
            record_key: "k".into(),
            subkey: 0,
            data: vec![],
            attempt: MAX_RETRIES - 1,
        };
        assert!(retry_or_fail(&write).is_none());
    }

    #[tokio::test]
    async fn queue_enqueue_and_receive() {
        let (handle, mut rx) = create_write_queue(16);
        handle.enqueue("key1".into(), 5, vec![0xAB]).await;

        let received = rx.recv().await.unwrap();
        assert_eq!(received.record_key, "key1");
        assert_eq!(received.subkey, 5);
        assert_eq!(received.data, vec![0xAB]);
        assert_eq!(received.attempt, 0);
    }
}
