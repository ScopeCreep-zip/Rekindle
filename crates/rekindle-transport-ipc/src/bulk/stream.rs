//! Bulk stream: encrypt-side hot path for parallel chunk encryption.
//!
//! `BulkStream` accepts `Vec<u8>` (owned, zero copies).
//! `ZeroizingStream` accepts `&[u8]` (copies into Zeroizing<Vec<u8>>).
//! Both zeroize all plaintext after encryption — no exceptions.
//!
//! Bridge-free: rayon workers call `tokio::sync::mpsc::Sender::blocking_send`
//! directly. No OS thread per connection.
//!
//! Safety: ALWAYS pool.spawn(), NEVER pool.scope(). scope() would deadlock
//! because the scope owner waits for tasks while tasks wait for the channel
//! consumer which IS the scope owner.
//!
//! blocking_send is safe on rayon workers (plain OS threads, no Tokio runtime).
//! It parks the rayon worker when the channel is full — correct backpressure.
//! When blocking_send returns Err, the receiver was dropped — normal shutdown.

use std::sync::Arc;
use rayon::ThreadPool;
use tokio::sync::mpsc;

use super::cipher::BulkCipher;
use super::frame::{BulkFrameHeader, FrameKind, HEADER_LEN, MAX_CHUNK_PLAIN, TAG_LEN};
use super::nonce::NonceCounter;
use super::pool::BufferPool;

/// Identifier for a logical bulk stream (0-255).
pub type BulkStreamId = u8;

/// Bulk stream accepting owned Vec<u8> input.
pub struct BulkStream {
    id: BulkStreamId,
    cipher: Arc<BulkCipher>,
    nonce_ctr: Arc<NonceCounter>,
    pool: Arc<BufferPool>,
    out_tx: mpsc::Sender<Vec<u8>>,
}

impl BulkStream {
    pub fn new(
        id: BulkStreamId,
        cipher: Arc<BulkCipher>,
        nonce_ctr: Arc<NonceCounter>,
        pool: Arc<BufferPool>,
        out_tx: mpsc::Sender<Vec<u8>>,
    ) -> Self {
        Self { id, cipher, nonce_ctr, pool, out_tx }
    }

    pub fn id(&self) -> BulkStreamId {
        self.id
    }

    pub fn submit_chunk(&self, encrypt_pool: &ThreadPool, plain: Vec<u8>, is_last: bool, chunk_seq: u32) {
        debug_assert!(plain.len() <= MAX_CHUNK_PLAIN);
        let nonce = self.nonce_ctr.next();
        let cipher = Arc::clone(&self.cipher);
        let pool = Arc::clone(&self.pool);
        let out_tx = self.out_tx.clone();
        let stream_id = self.id;
        let kind = if is_last { FrameKind::BulkFin } else { FrameKind::BulkData };
        // ALWAYS pool.spawn(), NEVER pool.scope().
        encrypt_pool.spawn(move || {
            encrypt_chunk_inner(&cipher, &pool, &out_tx, stream_id, kind, nonce, chunk_seq, plain);
        });
    }
}

/// Zeroizing bulk stream accepting &[u8] (borrowed) input.
pub struct ZeroizingStream {
    id: BulkStreamId,
    cipher: Arc<BulkCipher>,
    nonce_ctr: Arc<NonceCounter>,
    pool: Arc<BufferPool>,
    out_tx: mpsc::Sender<Vec<u8>>,
}

impl ZeroizingStream {
    pub fn new(
        id: BulkStreamId,
        cipher: Arc<BulkCipher>,
        nonce_ctr: Arc<NonceCounter>,
        pool: Arc<BufferPool>,
        out_tx: mpsc::Sender<Vec<u8>>,
    ) -> Self {
        Self { id, cipher, nonce_ctr, pool, out_tx }
    }

    pub fn submit_chunk(&self, encrypt_pool: &ThreadPool, plain: &[u8], is_last: bool, chunk_seq: u32) {
        debug_assert!(plain.len() <= MAX_CHUNK_PLAIN);
        let nonce = self.nonce_ctr.next();
        let cipher = Arc::clone(&self.cipher);
        let pool = Arc::clone(&self.pool);
        let out_tx = self.out_tx.clone();
        let stream_id = self.id;
        let kind = if is_last { FrameKind::BulkFin } else { FrameKind::BulkData };
        let owned = plain.to_vec();
        // ALWAYS pool.spawn(), NEVER pool.scope().
        encrypt_pool.spawn(move || {
            encrypt_chunk_inner(&cipher, &pool, &out_tx, stream_id, kind, nonce, chunk_seq, owned);
        });
    }
}

/// Total frame body size for a given plaintext length.
pub const fn frame_body_size(plaintext_len: usize) -> usize {
    HEADER_LEN + plaintext_len + TAG_LEN
}

/// Shared encrypt closure body.
///
/// blocking_send parks the rayon worker if channel full — correct backpressure.
/// When blocking_send returns Err, receiver was dropped — normal shutdown.
pub(crate) fn encrypt_chunk_inner(
    cipher: &BulkCipher,
    pool: &BufferPool,
    out_tx: &mpsc::Sender<Vec<u8>>,
    stream_id: u8,
    kind: FrameKind,
    nonce: u64,
    chunk_seq: u32,
    plain: Vec<u8>,
) {
    let owned = zeroize::Zeroizing::new(plain);
    let mut slab = pool.acquire();

    let header = BulkFrameHeader::new(stream_id, kind, nonce, chunk_seq);
    let hdr = header.encode_array();
    slab.extend_from_slice(&hdr);

    let ct_start = slab.len();
    slab.extend_from_slice(&owned);
    let ct_len = owned.len();

    let tag = cipher
        .seal_in_place(nonce, &hdr, &mut slab[ct_start..ct_start + ct_len])
        .expect("AEAD seal cannot fail with valid key and nonce");
    slab.extend_from_slice(&tag);

    drop(owned); // Zeroizes the plaintext copy

    // blocking_send: parks rayon worker if full. Err means receiver dropped.
    if out_tx.blocking_send(slab).is_err() {
        tracing::debug!("bulk encrypt: receiver dropped, frame discarded (shutdown)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encrypt::build_encrypt_pool;

    // #[test] not #[tokio::test] — blocking_recv is safe on plain OS threads.
    // blocking_recv panics inside a tokio runtime.
    #[test]
    fn submit_produces_correct_frame_size() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = BufferPool::new(16);
        let encrypt_pool = build_encrypt_pool(0);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);

        let stream = BulkStream::new(0, cipher, Arc::new(NonceCounter::new()), pool, tx);
        stream.submit_chunk(&encrypt_pool, vec![0xAB; 1024], false, 0);

        // Drop stream (and its Sender clone) so rx sees closure after
        // the rayon worker's clone is also dropped.
        drop(stream);

        let frame = rx.blocking_recv().unwrap();
        assert_eq!(frame.len(), frame_body_size(1024));
    }
}
