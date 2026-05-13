//! Bulk stream: encrypt-side hot path for parallel chunk encryption.
//!
//! `BulkStream` accepts `Vec<u8>` (taking ownership, zero copies).
//! `ZeroizingStream` accepts `&[u8]` (copies into `Zeroizing<Vec<u8>>`).
//! Both provide identical security guarantees — all plaintext is zeroized
//! after encryption. Choose based on whether you have owned or borrowed data.
//!
//! # Security
//!
//! All plaintext is zeroized after encryption — no exceptions, no fast paths.
//!
//! - The `Vec<u8>` input is moved (not copied) into a `Zeroizing<Vec<u8>>`.
//!   After `seal_in_place`, the owned copy is dropped, zeroing it via `Zeroize`.
//!   No orphaned plaintext allocations exist at any point.
//! - Pool slabs are zeroized via volatile writes in `BufferPool::replenish`
//!   before returning to the free list.
//! - The `NonceCounter` aborts the process on exhaustion rather than wrapping.
//!
//! # Wire layout produced by submit_chunk
//!
//! Each slab sent through the channel contains the frame BODY only:
//! `[stream_id(1)][kind(1)][nonce(8)][ciphertext(N)][tag(16)]`
//!
//! The lane byte and 4-byte length prefix are added by the write path:
//! - The write_loop prepends the lane byte before the frame body.
//! - `write_frame()` adds the 4-byte length prefix automatically.
//!
//! This matches the read path: `read_lane_byte()` consumes the lane,
//! `read_frame()` consumes the length prefix and returns the body.

use std::sync::Arc;
use rayon::ThreadPool;

use super::cipher::BulkCipher;
use super::frame::{BulkFrameHeader, FrameKind, HEADER_LEN, TAG_LEN, MAX_CHUNK_PLAIN};
use super::nonce::NonceCounter;
use super::pool::BufferPool;

/// Identifier for a logical bulk stream (0–255).
pub type BulkStreamId = u8;

/// A logical bulk transfer stream.
///
/// Fields are private. Use `new()` to construct.
pub struct BulkStream {
    id: BulkStreamId,
    cipher: Arc<BulkCipher>,
    nonce_ctr: Arc<NonceCounter>,
    pool: Arc<BufferPool>,
    out_tx: crossbeam::channel::Sender<Vec<u8>>,
}

impl BulkStream {
    /// Construct a new bulk stream.
    pub fn new(
        id: BulkStreamId,
        cipher: Arc<BulkCipher>,
        nonce_ctr: Arc<NonceCounter>,
        pool: Arc<BufferPool>,
        out_tx: crossbeam::channel::Sender<Vec<u8>>,
    ) -> Self {
        Self { id, cipher, nonce_ctr, pool, out_tx }
    }

    /// The stream identifier.
    pub fn id(&self) -> BulkStreamId {
        self.id
    }

    /// Submit a plaintext chunk for encryption and transmission.
    ///
    /// Takes ownership of `plain` and wraps it directly in
    /// `Zeroizing<Vec<u8>>` — zero copies, zero orphaned allocations.
    /// After `seal_in_place` overwrites the slab's plaintext region
    /// with ciphertext, the `Zeroizing` wrapper is dropped, zeroing
    /// the only copy of plaintext. There is no code path where
    /// plaintext survives in memory after encryption completes.
    ///
    /// The slab written to the channel contains the frame body only:
    /// `[header(10)][ciphertext(N)][tag(16)]`
    ///
    /// The write path adds the lane byte and length prefix.
    pub fn submit_chunk(
        &self,
        encrypt_pool: &ThreadPool,
        plain: Vec<u8>,
        is_last: bool,
    ) {
        debug_assert!(
            plain.len() <= MAX_CHUNK_PLAIN,
            "chunk too large: {} > {}",
            plain.len(),
            MAX_CHUNK_PLAIN,
        );

        let nonce = self.nonce_ctr.next();

        let cipher = Arc::clone(&self.cipher);
        let pool = Arc::clone(&self.pool);
        let out_tx = self.out_tx.clone();
        let stream_id = self.id;
        let kind = if is_last {
            FrameKind::BulkFin
        } else {
            FrameKind::BulkData
        };

        encrypt_pool.spawn(move || {
            encrypt_chunk_inner(&cipher, &pool, &out_tx, stream_id, kind, nonce, plain);
        });
    }
}

/// Total frame body size for a given plaintext length.
/// body = header(10) + ciphertext(N) + tag(16)
pub const fn frame_body_size(plaintext_len: usize) -> usize {
    HEADER_LEN + plaintext_len + TAG_LEN
}

/// Shared encrypt closure body for both `BulkStream` and `ZeroizingStream`.
///
/// Takes ownership of `plain`, wraps it in `Zeroizing<Vec<u8>>`, acquires
/// a pool slab, writes header + plaintext, encrypts in place, appends tag,
/// drops the `Zeroizing` wrapper (zeroizes), and sends the slab.
pub(crate) fn encrypt_chunk_inner(
    cipher: &BulkCipher,
    pool: &BufferPool,
    out_tx: &crossbeam::channel::Sender<Vec<u8>>,
    stream_id: u8,
    kind: FrameKind,
    nonce: u64,
    plain: Vec<u8>,
) {
    let owned_plain = zeroize::Zeroizing::new(plain);
    let mut slab = pool.acquire();

    let header = BulkFrameHeader::new(stream_id, kind, nonce);
    let hdr_bytes = header.encode_array();

    slab.extend_from_slice(&hdr_bytes);

    let ct_start = slab.len();
    slab.extend_from_slice(&owned_plain);
    let ct_len = owned_plain.len();

    let tag = cipher
        .seal_in_place(nonce, &hdr_bytes, &mut slab[ct_start..ct_start + ct_len])
        .expect("AEAD seal cannot fail with valid key and nonce");

    slab.extend_from_slice(&tag);

    drop(owned_plain);

    let _ = out_tx.send(slab);
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encrypt::build_encrypt_pool;

    #[test]
    fn submit_produces_correctly_sized_frames() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = BufferPool::new();
        let encrypt_pool = build_encrypt_pool();
        let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);

        let stream = BulkStream::new(
            0,
            cipher,
            Arc::new(NonceCounter::new()),
            pool,
            tx,
        );

        let n = 10;
        for i in 0..n {
            let plain = vec![0xAB; 1024];
            stream.submit_chunk(&encrypt_pool, plain, i == n - 1);
        }

        let mut frames = Vec::new();
        for _ in 0..n {
            let frame = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("timed out waiting for encrypted frame");
            frames.push(frame);
        }

        assert_eq!(frames.len(), n);

        // Each frame body: header(10) + ct(1024) + tag(16) = 1050
        let expected_size = frame_body_size(1024);
        for frame in &frames {
            assert_eq!(frame.len(), expected_size);
        }

        // All frames should be distinct (different nonces).
        for i in 0..frames.len() {
            for j in (i + 1)..frames.len() {
                assert_ne!(frames[i], frames[j], "frames {i} and {j} are identical");
            }
        }
    }

    #[test]
    fn header_kind_matches_is_last() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = BufferPool::new();
        let encrypt_pool = build_encrypt_pool();
        let (tx, rx) = crossbeam::channel::bounded::<Vec<u8>>(64);

        let stream = BulkStream::new(
            7,
            cipher,
            Arc::new(NonceCounter::new()),
            pool,
            tx,
        );

        stream.submit_chunk(&encrypt_pool, vec![0; 100], false);
        stream.submit_chunk(&encrypt_pool, vec![0; 100], true);

        let frame0 = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let frame1 = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

        // kind byte is at offset 1 in the header.
        // Sort by nonce (offset 2..10) to determine which is data vs fin.
        let nonce0 = u64::from_le_bytes(frame0[2..10].try_into().unwrap());
        let nonce1 = u64::from_le_bytes(frame1[2..10].try_into().unwrap());
        let (data_frame, fin_frame) = if nonce0 < nonce1 {
            (&frame0, &frame1)
        } else {
            (&frame1, &frame0)
        };

        assert_eq!(data_frame[1], FrameKind::BulkData as u8);
        assert_eq!(fin_frame[1], FrameKind::BulkFin as u8);
        assert_eq!(data_frame[0], 7); // stream_id
        assert_eq!(fin_frame[0], 7);
    }
}
