//! BulkSession: lifecycle wrapper for a bulk transfer connection.
//!
//! Owns the cipher, nonce counter, and buffer pool reference for a
//! single connection's bulk plane. On drop, logs the session duration
//! and nonce usage.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::cipher::BulkCipher;
use super::pool::BufferPool;
use super::stream::BulkStream;

/// Bulk transfer session for a single IPC connection.
pub struct BulkSession {
    conn_id: u64,
    cipher: Arc<BulkCipher>,
    nonce_ctr: Arc<AtomicU64>,
    pool: Arc<BufferPool>,
    created_at: Instant,
    digest_algorithm: super::verify::DigestAlgorithm,
}

impl BulkSession {
    /// Create a new bulk session with default algorithm (BLAKE3).
    pub fn new(
        conn_id: u64,
        cipher: BulkCipher,
        pool: Arc<BufferPool>,
    ) -> Self {
        Self::with_algorithm(conn_id, cipher, pool, super::verify::DigestAlgorithm::default())
    }

    /// Create a bulk session with a specific digest algorithm.
    pub fn with_algorithm(
        conn_id: u64,
        cipher: BulkCipher,
        pool: Arc<BufferPool>,
        digest_algorithm: super::verify::DigestAlgorithm,
    ) -> Self {
        Self {
            conn_id,
            cipher: Arc::new(cipher),
            nonce_ctr: Arc::new(AtomicU64::new(0)),
            pool,
            created_at: Instant::now(),
            digest_algorithm,
        }
    }

    /// Create a BulkStream for sending data on a specific stream_id.
    pub fn create_stream(
        &self,
        stream_id: u8,
        out_tx: crossbeam::channel::Sender<Vec<u8>>,
    ) -> BulkStream {
        BulkStream::new(
            stream_id,
            Arc::clone(&self.cipher),
            Arc::clone(&self.nonce_ctr),
            Arc::clone(&self.pool),
            out_tx,
        )
    }

    /// The session cipher.
    pub fn cipher(&self) -> &Arc<BulkCipher> {
        &self.cipher
    }

    /// The session's nonce counter.
    pub fn nonce_counter(&self) -> &Arc<AtomicU64> {
        &self.nonce_ctr
    }

    /// The digest algorithm for this session.
    pub fn digest_algorithm(&self) -> super::verify::DigestAlgorithm {
        self.digest_algorithm
    }

    /// Session duration.
    pub fn elapsed(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::pool::BufferPool;

    #[test]
    fn session_lifecycle() {
        let pool = BufferPool::new();
        let cipher_key = [0x42u8; 32];
        let cipher = BulkCipher::new(&cipher_key);

        let session = BulkSession::new(99, cipher, pool);

        // cipher() returns a valid Arc that can be cloned.
        let cipher_ref = session.cipher();
        assert!(Arc::strong_count(cipher_ref) >= 1);

        // nonce_counter() starts at 0.
        assert_eq!(session.nonce_counter().load(Ordering::Relaxed), 0);

        // elapsed() returns a valid duration (session was just created).
        assert!(session.elapsed().as_secs() < 10);

        // Drop does not panic (nonces_used == 0, so no tracing::info).
        drop(session);
    }

    #[test]
    fn session_nonce_counter_shared_with_stream() {
        let pool = BufferPool::new();
        let cipher = BulkCipher::new(&[0x42; 32]);
        let session = BulkSession::new(1, cipher, pool);

        let (tx, _rx) = crossbeam::channel::bounded::<Vec<u8>>(16);
        let stream = session.create_stream(0, tx);

        // The stream shares the session's nonce counter.
        // Submitting a chunk increments the counter.
        let encrypt_pool = super::super::encrypt::build_encrypt_pool();
        stream.submit_chunk(&encrypt_pool, bytes::Bytes::from_static(b"test"), true);

        // Wait for the rayon task to complete.
        let _ = _rx.recv_timeout(std::time::Duration::from_secs(5));

        assert_eq!(session.nonce_counter().load(Ordering::Relaxed), 1);
    }
}

impl Drop for BulkSession {
    #[allow(clippy::cast_possible_truncation)] // session duration < 2^64 ms
    fn drop(&mut self) {
        let nonces_used = self.nonce_ctr.load(Ordering::Relaxed);
        if nonces_used > 0 {
            tracing::info!(
                conn_id = self.conn_id,
                nonces_used,
                duration_ms = self.created_at.elapsed().as_millis() as u64,
                "bulk session closed"
            );
        }
    }
}
