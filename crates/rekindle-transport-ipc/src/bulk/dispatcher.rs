//! Bulk frame dispatcher: receive-side entry point.
//!
//! Parses header, checks replay filter, dispatches to rayon for
//! parallel AEAD decrypt + per-chunk digest computation.
//!
//! Rayon workers call `blocking_send` on a bounded channel. When the
//! channel is full, workers block — correct backpressure. Memory usage
//! is O(channel_capacity × chunk_size) regardless of total transfer size.
//!
//! Single-threaded by design — ReplayFilter is not thread-safe.
//! The dispatcher processes frames sequentially from the socket read loop.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use rayon::ThreadPool;
use tokio::sync::mpsc;

use crate::backpressure::{GlobalMemoryGuard, MemoryReservation};
use super::cipher::BulkCipher;
use super::frame::{BulkFrameHeader, FrameKind, HEADER_LEN, TAG_LEN};
use super::pool::ZeroizingBuf;
use super::replay::ReplayFilter;
use super::verify::DigestAlgorithm;

/// Decrypted chunk sent from rayon workers to the reassembly task.
pub struct DecryptedChunk {
    pub stream_id: u8,
    /// Per-stream reassembly ordering index (0-indexed per transfer).
    /// NOT the AEAD nonce — those are separate concerns.
    pub chunk_seq: u32,
    /// Decrypted plaintext. Zeroized on drop (full capacity, volatile writes).
    /// No pool reference — recv-path buffers are heap-allocated and freed.
    pub plaintext: ZeroizingBuf,
    pub is_last: bool,
    /// For BulkFin: the 32-byte sender-computed blob digest.
    pub fin_digest: Option<[u8; 32]>,
    /// Per-chunk digest (computed on rayon worker while data in L1).
    pub chunk_digest: [u8; 32],
    /// True if AEAD decryption failed.
    pub decrypt_failed: bool,
    /// RAII global memory reservation. Held from dispatch through reassembly
    /// to delivery. Released on drop (after on_bulk_chunk returns).
    pub reservation: Option<MemoryReservation>,
    /// RAII per-connection memory reservation. Prevents one connection from
    /// consuming the entire global budget.
    pub per_conn_reservation: Option<MemoryReservation>,
}

/// Dispatch errors.
#[derive(Debug)]
pub enum DispatchError {
    TooShort(usize),
    InvalidHeader,
    NotBulk,
    Replay(u64),
    Backpressure { buffered: u64 },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort(n) => write!(f, "frame too short: {n} bytes"),
            Self::InvalidHeader => write!(f, "invalid bulk frame header"),
            Self::NotBulk => write!(f, "not a bulk frame"),
            Self::Replay(n) => write!(f, "nonce replay: {n}"),
            Self::Backpressure { buffered } => write!(f, "backpressure: {buffered} bytes in flight"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Bulk frame dispatcher. Lives on the receive side of a connection.
///
/// Uses a bounded channel for backpressure. Rayon workers call
/// `blocking_send` which parks the OS thread when the channel is full.
/// This bounds memory to O(channel_capacity × chunk_size) regardless
/// of total transfer size.
pub struct BulkDispatcher {
    cipher: Arc<BulkCipher>,
    replay: ReplayFilter,
    decrypt_pool: Arc<ThreadPool>,
    reassembly_tx: mpsc::Sender<DecryptedChunk>,
    digest_algorithm: DigestAlgorithm,
    in_flight: Arc<AtomicUsize>,
    counters: Arc<super::BulkCounters>,
    memory_guard: Option<Arc<GlobalMemoryGuard>>,
    per_conn_guard: Option<Arc<GlobalMemoryGuard>>,
}

/// Default bounded channel capacity for reassembly.
/// 32 chunks × ~65KB = ~2MB max in-flight decrypted data.
/// Provides enough buffering for out-of-order rayon completion
/// without unbounded memory growth.
pub const DEFAULT_REASSEMBLY_CAPACITY: usize = 32;

impl BulkDispatcher {
    pub fn new(
        cipher: Arc<BulkCipher>,
        decrypt_pool: Arc<ThreadPool>,
        reassembly_tx: mpsc::Sender<DecryptedChunk>,
        digest_algorithm: DigestAlgorithm,
        counters: Arc<super::BulkCounters>,
    ) -> Self {
        Self {
            cipher,
            replay: ReplayFilter::new(),
            decrypt_pool,
            reassembly_tx,
            digest_algorithm,
            in_flight: Arc::new(AtomicUsize::new(0)),
            counters,
            memory_guard: None,
            per_conn_guard: None,
        }
    }

    /// Set the global memory guard for backpressure.
    /// When set, each dispatched frame reserves memory from the guard.
    /// The reservation is held through decrypt → reassembly → delivery.
    /// Released on drop (after on_bulk_chunk returns).
    pub fn with_memory_guard(mut self, guard: Arc<GlobalMemoryGuard>) -> Self {
        self.memory_guard = Some(guard);
        self
    }

    pub fn with_per_connection_guard(mut self, guard: Arc<GlobalMemoryGuard>) -> Self {
        self.per_conn_guard = Some(guard);
        self
    }

    pub fn replay_highest(&self) -> u64 {
        self.replay.highest()
    }

    pub fn replay_accepted_count(&self) -> u64 {
        self.replay.accepted_count()
    }

    /// Dispatch a raw bulk frame for decryption.
    ///
    /// `frame_body` is the body AFTER the lane byte has been stripped.
    /// Contains: `[10B header][ciphertext][16B tag]`
    ///
    /// Rayon workers call `blocking_send` on the bounded channel.
    /// When the channel is full, the rayon worker parks until the
    /// control loop drains a slot. This is correct backpressure —
    /// rayon workers are plain OS threads, not async tasks.
    #[allow(clippy::needless_pass_by_value)]
    pub fn dispatch(&mut self, frame_body: Vec<u8>) -> Result<(), DispatchError> {
        if frame_body.len() < HEADER_LEN + TAG_LEN {
            return Err(DispatchError::TooShort(frame_body.len()));
        }

        let header =
            BulkFrameHeader::decode(&frame_body).ok_or(DispatchError::InvalidHeader)?;

        if !header.kind.is_bulk() {
            return Err(DispatchError::NotBulk);
        }

        if let Err(_rejection) = self.replay.check_and_accept(header.nonce) {
            return Err(DispatchError::Replay(header.nonce));
        }

        let cipher = Arc::clone(&self.cipher);
        let tx = self.reassembly_tx.clone();
        let digest_algorithm = self.digest_algorithm;
        let stream_id = header.stream_id;
        let nonce = header.nonce;
        let chunk_seq = header.chunk_seq;
        let is_last = header.kind == FrameKind::BulkFin;

        let mut aad = [0u8; HEADER_LEN];
        aad.copy_from_slice(&frame_body[..HEADER_LEN]);

        let ct_and_tag = {
            let mut v = frame_body;
            v.drain(..HEADER_LEN);
            v
        };

        // Reserve memory from the global guard BEFORE spawning the rayon task.
        // The reservation travels through the pipeline and is released when
        // the chunk is delivered via on_bulk_chunk (RAII drop).
        let reservation = if let Some(ref guard) = self.memory_guard {
            match guard.try_reserve(ct_and_tag.len() as u64) {
                Ok(r) => Some(r),
                Err(_) => return Err(DispatchError::Backpressure {
                    buffered: guard.used(),
                }),
            }
        } else {
            None
        };

        let per_conn_reservation = if let Some(ref guard) = self.per_conn_guard {
            match guard.try_reserve(ct_and_tag.len() as u64) {
                Ok(r) => Some(r),
                Err(_) => return Err(DispatchError::Backpressure {
                    buffered: guard.used(),
                }),
            }
        } else {
            None
        };

        let in_flight = Arc::clone(&self.in_flight);
        let counters = Arc::clone(&self.counters);
        self.in_flight.fetch_add(1, Ordering::Relaxed);

        self.decrypt_pool.spawn_fifo(move || {
            struct Guard(Arc<AtomicUsize>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::Release);
                }
            }
            let _guard = Guard(in_flight);

            let mut ct_and_tag = ZeroizingBuf::new(ct_and_tag);

            let result = cipher.open_in_place(nonce, &aad, &mut ct_and_tag);

            let chunk = match result {
                Ok(pt_len) => {
                    counters.chunks_decrypted.fetch_add(1, Ordering::Relaxed);
                    ct_and_tag.truncate(pt_len);

                    if is_last {
                        let fin_digest = if pt_len >= 32 {
                            let mut digest = [0u8; 32];
                            digest.copy_from_slice(&ct_and_tag[..32]);
                            Some(digest)
                        } else {
                            None
                        };

                        DecryptedChunk {
                            stream_id,
                            chunk_seq,
                            plaintext: ZeroizingBuf::new(Vec::new()),
                            is_last: true,
                            fin_digest,
                            chunk_digest: [0u8; 32],
                            decrypt_failed: false,
                            reservation,
                            per_conn_reservation,
                        }
                    } else {
                        let chunk_digest =
                            super::verify::digest_oneshot(digest_algorithm, &ct_and_tag);

                        DecryptedChunk {
                            stream_id,
                            chunk_seq,
                            plaintext: ct_and_tag,
                            is_last: false,
                            fin_digest: None,
                            chunk_digest,
                            decrypt_failed: false,
                            reservation,
                            per_conn_reservation,
                        }
                    }
                }
                Err(_e) => {
                    DecryptedChunk {
                        stream_id,
                        chunk_seq,
                        plaintext: ZeroizingBuf::new(Vec::new()),
                        is_last: false,
                        fin_digest: None,
                        chunk_digest: [0u8; 32],
                        decrypt_failed: true,
                        reservation,
                        per_conn_reservation,
                    }
                }
            };

            // blocking_send: parks the rayon worker when the bounded channel
            // is full. This is correct backpressure — rayon workers are OS
            // threads, not async tasks. Memory is bounded to
            // channel_capacity × chunk_size regardless of transfer size.
            // Err means receiver dropped — connection shutting down.
            let _ = tx.blocking_send(chunk);
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::cipher::BulkCipher;
    use super::super::encrypt::build_encrypt_pool;
    use super::super::frame::BulkFrameHeader;

    fn make_frame(cipher: &BulkCipher, nonce: u64, plaintext: &[u8]) -> Vec<u8> {
        let header = BulkFrameHeader::new(0, FrameKind::BulkData, nonce, nonce as u32);
        let hdr = header.encode_array();
        let mut frame = Vec::new();
        frame.extend_from_slice(&hdr);
        frame.extend_from_slice(plaintext);
        let ct_start = HEADER_LEN;
        let tag = cipher
            .seal_in_place(nonce, &hdr, &mut frame[ct_start..ct_start + plaintext.len()])
            .unwrap();
        frame.extend_from_slice(&tag);
        frame
    }

    #[test]
    fn dispatch_and_decrypt_roundtrip() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool(0);
        let (tx, mut rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3,
            super::super::BulkCounters::new(),
        );

        let frame = make_frame(&cipher, 0, b"hello bulk");
        dispatcher.dispatch(frame).unwrap();

        let chunk = rx.blocking_recv().unwrap();
        assert_eq!(&*chunk.plaintext, b"hello bulk");
        assert!(!chunk.is_last);
    }

    #[test]
    fn replay_rejected() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool(0);
        let (tx, _rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3,
            super::super::BulkCounters::new(),
        );

        let frame = make_frame(&cipher, 42, b"data");
        dispatcher.dispatch(frame.clone()).unwrap();
        assert!(matches!(dispatcher.dispatch(frame), Err(DispatchError::Replay(42))));
    }
}
