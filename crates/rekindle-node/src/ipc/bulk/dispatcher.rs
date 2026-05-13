//! Bulk frame dispatcher: receive-side entry point.
//!
//! Receives raw bulk frames from the connection handler's read loop,
//! parses the header, checks the nonce replay filter, and dispatches
//! the frame to a rayon worker for parallel decryption.
//!
//! The dispatcher is single-threaded by design: it processes frames
//! sequentially from the socket read loop. The replay filter is NOT
//! thread-safe and does not need to be — it is only accessed here.
//!
//! # Plaintext zeroization
//!
//! The `ct_and_tag` buffer is wrapped in `PooledBuf` inside the rayon
//! closure. After decryption, the buffer becomes `DecryptedChunk::plaintext`.
//! When the application drops `ReassembledChunk`, the `PooledBuf` drop
//! impl calls `BufferPool::replenish()` which zeroizes via volatile
//! writes and returns the slab to the receive pool. There is no code
//! path where receive-side plaintext survives in freed memory.

use std::sync::Arc;
use rayon::ThreadPool;

use super::cipher::BulkCipher;
use super::frame::{BulkFrameHeader, FrameKind, HEADER_LEN, TAG_LEN};
use super::replay::ReplayFilter;

/// Decrypted chunk metadata + plaintext, sent from rayon decrypt
/// workers to the reassembly task.
///
/// `plaintext` is a `PooledBuf` — zeroized and returned to the
/// receive pool on drop. There is no code path in the receive
/// pipeline where decrypted plaintext survives in freed memory.
pub struct DecryptedChunk {
    /// Which bulk stream this chunk belongs to.
    pub stream_id: u8,
    /// Chunk ordering index (the nonce from the wire header).
    pub chunk_index: u64,
    /// Decrypted plaintext bytes. Returned to receive pool on drop.
    pub plaintext: super::pooled_buf::PooledBuf,
    /// True if this is the final chunk of the transfer.
    pub is_last: bool,
    /// For BulkFin frames: the 32-byte sender-computed blob digest
    /// extracted from the plaintext. None for BulkData frames.
    pub fin_digest: Option<[u8; 32]>,
    /// Per-chunk digest (SHA-256 or BLAKE3, determined by session's
    /// DigestAlgorithm), computed on the rayon worker immediately after
    /// decryption while data is in L1 cache. Fed into MerkleDigest.
    pub chunk_digest: [u8; 32],
    /// True if AEAD decryption failed for this chunk.
    pub decrypt_failed: bool,
}

/// Dispatch errors.
#[derive(Debug)]
pub enum DispatchError {
    /// Frame is too short to contain a header + tag.
    TooShort(usize),
    /// The header's frame_kind byte is unrecognized.
    InvalidHeader,
    /// The frame is not a bulk frame (it is a control frame).
    NotBulk,
    /// The nonce was already seen (replay attack or duplicate).
    Replay(u64),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort(n) => write!(f, "frame too short: {n} bytes"),
            Self::InvalidHeader => write!(f, "invalid bulk frame header"),
            Self::NotBulk => write!(f, "not a bulk frame"),
            Self::Replay(n) => write!(f, "nonce replay detected: {n}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// The bulk dispatcher. Lives on the receive side of a connection.
///
/// Not `Send` — the `ReplayFilter` is single-threaded. The dispatcher
/// must be used from the same task that reads frames from the socket.
pub struct BulkDispatcher {
    cipher: Arc<BulkCipher>,
    replay: ReplayFilter,
    decrypt_pool: Arc<ThreadPool>,
    reassembly_tx: crossbeam::channel::Sender<DecryptedChunk>,
    digest_algorithm: super::verify::DigestAlgorithm,
    recv_pool: Arc<super::pool::BufferPool>,
}

impl BulkDispatcher {
    /// Create a new dispatcher with default digest algorithm (BLAKE3).
    ///
    /// `cipher`: the session's BulkCipher (Arc-shared with the send side).
    /// `decrypt_pool`: the rayon pool for parallel decryption (same pool
    ///   as encryption — AES-GCM encrypt and decrypt are the same cost).
    /// `reassembly_tx`: channel to the reassembly task.
    pub fn new(
        cipher: Arc<BulkCipher>,
        decrypt_pool: Arc<ThreadPool>,
        reassembly_tx: crossbeam::channel::Sender<DecryptedChunk>,
        recv_pool: Arc<super::pool::BufferPool>,
    ) -> Self {
        Self::with_algorithm(cipher, decrypt_pool, reassembly_tx, super::verify::DigestAlgorithm::default(), recv_pool)
    }

    /// Create a dispatcher with a specific digest algorithm.
    pub fn with_algorithm(
        cipher: Arc<BulkCipher>,
        decrypt_pool: Arc<ThreadPool>,
        reassembly_tx: crossbeam::channel::Sender<DecryptedChunk>,
        digest_algorithm: super::verify::DigestAlgorithm,
        recv_pool: Arc<super::pool::BufferPool>,
    ) -> Self {
        Self {
            cipher,
            replay: ReplayFilter::new(),
            decrypt_pool,
            reassembly_tx,
            digest_algorithm,
            recv_pool,
        }
    }

    /// Dispatch a raw bulk frame for decryption.
    ///
    /// `frame_body` is the frame body AFTER the lane byte has been
    /// stripped by the connection handler. It contains:
    /// `[1B stream_id][1B kind][8B nonce][ciphertext][16B tag]`
    ///
    /// This method:
    /// 1. Parses the 10-byte header (stream_id + kind + nonce).
    /// 2. Checks the nonce against the replay filter.
    /// 3. Dispatches to a rayon worker for AEAD decryption.
    /// 4. On success, the worker sends a `DecryptedChunk` to the
    ///    reassembly channel.
    ///
    /// Returns `Ok(())` if dispatched. Returns `Err` if the frame is
    /// malformed, not a bulk frame, or a replay.
    #[allow(clippy::needless_pass_by_value)] // Caller transfers ownership; avoids clone at call site
    pub fn dispatch(&mut self, frame_body: Vec<u8>) -> Result<(), DispatchError> {
        if frame_body.len() < HEADER_LEN + TAG_LEN {
            return Err(DispatchError::TooShort(frame_body.len()));
        }

        let header = BulkFrameHeader::decode(&frame_body)
            .ok_or(DispatchError::InvalidHeader)?;

        if !header.kind.is_bulk() {
            return Err(DispatchError::NotBulk);
        }

        // Replay check — single-threaded, no lock needed.
        if !self.replay.check_and_accept(header.nonce) {
            tracing::warn!(nonce = header.nonce, "bulk frame replay detected");
            return Err(DispatchError::Replay(header.nonce));
        }

        // Extract values for the rayon closure.
        let cipher = Arc::clone(&self.cipher);
        let tx = self.reassembly_tx.clone();
        let digest_algorithm = self.digest_algorithm;
        let stream_id = header.stream_id;
        let nonce = header.nonce;
        let is_last = header.kind == FrameKind::BulkFin;

        let mut aad = [0u8; HEADER_LEN];
        aad.copy_from_slice(&frame_body[..HEADER_LEN]);

        // The ciphertext + tag starts after the header. We drain the
        // 10-byte header from the owned Vec rather than copying 65 KiB
        // into a new allocation. drain(..10) does a memmove of the
        // remaining bytes, which is cheaper than alloc+copy+free.
        let ct_and_tag = {
            let mut v = frame_body;
            v.drain(..HEADER_LEN);
            v
        };
        let recv_pool = Arc::clone(&self.recv_pool);

        self.decrypt_pool.spawn(move || {
            // Wrap ct_and_tag in PooledBuf so the buffer is zeroized
            // and returned to the receive pool when this closure exits
            // (success, error, or panic unwind).
            let mut ct_and_tag = super::pooled_buf::PooledBuf::new(ct_and_tag, Arc::clone(&recv_pool));

            // Decrypt in place. On success, ct_and_tag[..pt_len] is plaintext.
            let result = cipher.open_in_place(nonce, &aad, &mut ct_and_tag);

            match result {
                Ok(pt_len) => {
                    ct_and_tag.truncate(pt_len);

                    // For BulkFin frames, the first 32 bytes of plaintext
                    // are the sender's SHA-256 digest of the complete blob.
                    let fin_digest = if is_last && pt_len >= 32 {
                        let mut digest = [0u8; 32];
                        digest.copy_from_slice(&ct_and_tag[..32]);
                        // Remove the digest prefix, leaving only the
                        // actual data payload (if any) after the digest.
                        ct_and_tag.drain(..32);
                        Some(digest)
                    } else {
                        None
                    };

                    // Compute per-chunk digest while plaintext is hot in L1.
                    // This runs on the rayon worker — parallelized across cores.
                    let chunk_digest = super::verify::digest_oneshot(digest_algorithm, &ct_and_tag);

                    // ct_and_tag is already a PooledBuf containing only
                    // plaintext after truncate+drain. Move it directly.
                    let plaintext = ct_and_tag;

                    let chunk = DecryptedChunk {
                        stream_id,
                        chunk_index: nonce,
                        plaintext,
                        is_last,
                        fin_digest,
                        chunk_digest,
                        decrypt_failed: false,
                    };

                    let _ = tx.send(chunk);
                }
                Err(e) => {
                    tracing::warn!(
                        nonce, stream_id, error = %e,
                        "bulk AEAD decryption failed"
                    );
                    // ct_and_tag is PooledBuf — zeroized and returned to pool on drop.
                    let _ = tx.send(DecryptedChunk {
                        stream_id,
                        chunk_index: nonce,
                        plaintext: super::pooled_buf::PooledBuf::new(Vec::new(), recv_pool),
                        is_last: false,
                        fin_digest: None,
                        chunk_digest: [0u8; 32],
                        decrypt_failed: true,
                    });
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::encrypt::build_encrypt_pool;
    use super::super::frame::BulkFrameHeader;
    use super::super::cipher::BulkCipher;

    /// Helper: encrypt a plaintext chunk into a raw frame body
    /// (without the lane byte, as the dispatcher expects).
    fn make_encrypted_frame(
        cipher: &BulkCipher,
        stream_id: u8,
        kind: FrameKind,
        nonce: u64,
        plaintext: &[u8],
    ) -> Vec<u8> {
        let header = BulkFrameHeader::new(stream_id, kind, nonce);
        let hdr_bytes = header.encode_array();

        let mut frame = Vec::new();
        frame.extend_from_slice(&hdr_bytes);
        frame.extend_from_slice(plaintext);

        let ct_start = HEADER_LEN;
        let ct_len = plaintext.len();
        let tag = cipher
            .seal_in_place(nonce, &hdr_bytes, &mut frame[ct_start..ct_start + ct_len])
            .unwrap();
        frame.extend_from_slice(&tag);
        frame
    }

    #[test]
    fn dispatch_and_decrypt_roundtrip() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool();
        let recv_pool = super::super::pool::BufferPool::new();
        let (tx, rx) = crossbeam::channel::bounded::<DecryptedChunk>(64);

        let mut dispatcher = BulkDispatcher::new(
            Arc::clone(&cipher),
            pool,
            tx,
            recv_pool,
        );

        let plaintext = b"hello bulk transport";
        let frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 0, plaintext);

        dispatcher.dispatch(frame).unwrap();

        let chunk = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(&*chunk.plaintext, plaintext);
        assert_eq!(chunk.stream_id, 0);
        assert_eq!(chunk.chunk_index, 0);
        assert!(!chunk.is_last);
        assert!(chunk.fin_digest.is_none());
    }

    #[test]
    fn replay_rejected() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool();
        let recv_pool = super::super::pool::BufferPool::new();
        let (tx, _rx) = crossbeam::channel::bounded::<DecryptedChunk>(64);

        let mut dispatcher = BulkDispatcher::new(Arc::clone(&cipher), pool, tx, recv_pool);

        let frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 42, b"data");
        dispatcher.dispatch(frame.clone()).unwrap();

        // Same nonce again — should be rejected.
        let result = dispatcher.dispatch(frame);
        assert!(matches!(result, Err(DispatchError::Replay(42))));
    }

    #[test]
    fn too_short_rejected() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool();
        let recv_pool = super::super::pool::BufferPool::new();
        let (tx, _rx) = crossbeam::channel::bounded::<DecryptedChunk>(64);

        let mut dispatcher = BulkDispatcher::new(Arc::clone(&cipher), pool, tx, recv_pool);
        let result = dispatcher.dispatch(vec![0u8; 10]);
        assert!(matches!(result, Err(DispatchError::TooShort(10))));
    }

    #[test]
    fn fin_frame_extracts_digest() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool();
        let recv_pool = super::super::pool::BufferPool::new();
        let (tx, rx) = crossbeam::channel::bounded::<DecryptedChunk>(64);

        let mut dispatcher = BulkDispatcher::new(Arc::clone(&cipher), pool, tx, recv_pool);

        // BulkFin plaintext: 32-byte digest followed by optional data.
        let mut fin_plaintext = vec![0xAA; 32]; // fake digest
        fin_plaintext.extend_from_slice(b"trailing data");

        let frame = make_encrypted_frame(
            &cipher, 1, FrameKind::BulkFin, 99, &fin_plaintext,
        );
        dispatcher.dispatch(frame).unwrap();

        let chunk = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(chunk.is_last);
        assert_eq!(chunk.fin_digest, Some([0xAA; 32]));
        assert_eq!(&*chunk.plaintext, b"trailing data");
    }

    #[test]
    fn tampered_frame_sends_decrypt_failed() {
        let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
        let pool = build_encrypt_pool();
        let recv_pool = super::super::pool::BufferPool::new();
        let (tx, rx) = crossbeam::channel::bounded::<DecryptedChunk>(64);

        let mut dispatcher = BulkDispatcher::new(Arc::clone(&cipher), pool, tx, recv_pool);

        // Encrypt a valid frame, then tamper the ciphertext.
        let mut frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 0, b"secret data");
        // Flip a byte in the ciphertext region (after the 10-byte header).
        frame[HEADER_LEN] ^= 0xFF;

        dispatcher.dispatch(frame).unwrap();

        let chunk = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(chunk.decrypt_failed, "tampered frame must set decrypt_failed");
        assert!(chunk.plaintext.is_empty(), "tampered frame must have empty plaintext");
        assert_eq!(chunk.stream_id, 0);
        assert_eq!(chunk.chunk_index, 0);
        assert!(!chunk.is_last);
        assert!(chunk.fin_digest.is_none());
    }
}
