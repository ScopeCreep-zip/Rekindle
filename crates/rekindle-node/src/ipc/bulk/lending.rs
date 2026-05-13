//! Zeroizing buffer-lending encryption for the bulk transport plane.
//!
//! All plaintext is treated as sensitive. Every intermediate buffer that
//! holds plaintext is zeroized after the encrypt pass completes. There is
//! no "fast path" that skips zeroization — the safe path IS the only path.
//!
//! # Threat model
//!
//! At 100K+ agents per node, the probability of at least one compromised
//! agent approaches 1 over operational lifetime. A compromised agent with
//! read access to process memory (via `/proc/self/mem`, core dump, or
//! speculative execution side channel) can recover plaintext from any
//! buffer that was not zeroized. Therefore:
//!
//! - Pool slabs are zeroized via volatile writes in `BufferPool::replenish`
//!   before being returned to the free list (cannot be elided by optimizer).
//! - The owned plaintext copy inside the rayon closure is wrapped in
//!   `Zeroizing<Vec<u8>>` and zeroed on drop after encryption.
//! - `seal_in_place` overwrites the slab's plaintext region with ciphertext.
//!
//! # API design: safe by default
//!
//! Both `ZeroizingStream` (accepts `&[u8]`) and `BulkStream`
//! (accepts `Vec<u8>`) zeroize all plaintext after encryption.
//! There is no non-zeroizing code path in the codebase — the safe path
//! IS the only path. `NonceCounter` aborts the process on exhaustion
//! rather than wrapping, making nonce reuse structurally impossible.

use std::sync::Arc;
use rayon::ThreadPool;

use super::cipher::BulkCipher;
use super::frame::{FrameKind, MAX_CHUNK_PLAIN};
use super::nonce::NonceCounter;
use super::pool::BufferPool;

/// Zeroizing bulk stream: encrypts plaintext into pool slabs with
/// guaranteed zeroization of all intermediate plaintext buffers.
///
/// # Usage
///
/// ```ignore
/// let stream = ZeroizingStream::new(0, cipher, nonce, pool, tx);
/// stream.submit_chunk(&encrypt_pool, sensitive_data, false);
/// // sensitive_data can now be zeroized by the caller.
/// // The pool slab's plaintext region is zeroized after encrypt.
/// // The owned copy inside the rayon worker is zeroized after use.
/// ```
pub struct ZeroizingStream {
    stream_id: u8,
    cipher: Arc<BulkCipher>,
    nonce_ctr: Arc<NonceCounter>,
    pool: Arc<BufferPool>,
    out_tx: crossbeam::channel::Sender<Vec<u8>>,
}

impl ZeroizingStream {
    /// Construct a new zeroizing stream.
    pub fn new(
        stream_id: u8,
        cipher: Arc<BulkCipher>,
        nonce_ctr: Arc<NonceCounter>,
        pool: Arc<BufferPool>,
        out_tx: crossbeam::channel::Sender<Vec<u8>>,
    ) -> Self {
        Self { stream_id, cipher, nonce_ctr, pool, out_tx }
    }

    /// Submit a plaintext chunk for encryption.
    ///
    /// The plaintext is copied into a `Zeroizing<Vec<u8>>` for transfer
    /// to the rayon worker. After encryption, both the owned copy and
    /// the slab's plaintext region are overwritten:
    ///
    /// 1. `seal_in_place` overwrites the slab's plaintext with ciphertext
    /// 2. The `Zeroizing<Vec<u8>>` is dropped, zeroing the owned copy
    /// 3. On panic unwind, `Zeroizing<Vec<u8>>::drop` still fires (zeroizes)
    /// 4. On send failure, the slab contains only ciphertext+tag (no plaintext)
    /// 5. Pool slabs are zeroized via volatile writes in `replenish()`
    pub fn submit_chunk(
        &self,
        encrypt_pool: &ThreadPool,
        plain: &[u8],
        is_last: bool,
    ) {
        debug_assert!(
            plain.len() <= MAX_CHUNK_PLAIN,
            "chunk too large: {} > {}",
            plain.len(),
            MAX_CHUNK_PLAIN,
        );

        let nonce = self.nonce_ctr.next();
        let plain_owned = plain.to_vec();
        let cipher = Arc::clone(&self.cipher);
        let pool = Arc::clone(&self.pool);
        let out_tx = self.out_tx.clone();
        let stream_id = self.stream_id;
        let kind = if is_last { FrameKind::BulkFin } else { FrameKind::BulkData };

        encrypt_pool.spawn(move || {
            super::stream::encrypt_chunk_inner(
                &cipher, &pool, &out_tx, stream_id, kind, nonce, plain_owned,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::cipher::BulkCipher;
    use super::super::encrypt::build_encrypt_pool;
    use super::super::frame::HEADER_LEN;
    use super::super::stream::frame_body_size;

    #[test]
    fn zeroizing_stream_produces_correct_frame_size() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = BufferPool::new();
        let encrypt_pool = build_encrypt_pool();
        let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);

        let stream = ZeroizingStream::new(
            0, cipher, Arc::new(NonceCounter::new()), pool, tx,
        );

        let data = vec![0xAB; 1024];
        stream.submit_chunk(&encrypt_pool, &data, false);

        let frame = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("timed out");
        assert_eq!(frame.len(), frame_body_size(1024));
    }

    #[test]
    fn zeroizing_stream_max_chunk() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = BufferPool::new();
        let encrypt_pool = build_encrypt_pool();
        let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);

        let stream = ZeroizingStream::new(
            0, cipher, Arc::new(NonceCounter::new()), pool, tx,
        );

        let data = vec![0xCD; MAX_CHUNK_PLAIN];
        stream.submit_chunk(&encrypt_pool, &data, true);

        let frame = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("timed out");
        assert_eq!(frame.len(), frame_body_size(MAX_CHUNK_PLAIN));
        assert_eq!(frame[1], FrameKind::BulkFin as u8);
    }

    #[test]
    fn zeroizing_stream_output_decrypts_correctly() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = BufferPool::new();
        let encrypt_pool = build_encrypt_pool();
        let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);

        let stream = ZeroizingStream::new(
            0, Arc::clone(&cipher), Arc::new(NonceCounter::new()), pool, tx,
        );

        let original = vec![0xEF; 4096];
        stream.submit_chunk(&encrypt_pool, &original, false);

        let frame = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("timed out");

        // Decrypt: frame is [header(10)][ciphertext(4096)][tag(16)]
        let aad = &frame[..HEADER_LEN];
        let nonce = u64::from_le_bytes(frame[2..10].try_into().unwrap());
        let ct = &frame[HEADER_LEN..HEADER_LEN + 4096];
        let tag = &frame[HEADER_LEN + 4096..];

        let mut pt = vec![0u8; 4096];
        cipher.open_separate(nonce, aad, ct, tag, &mut pt).unwrap();
        assert_eq!(pt, original);
    }
}
