//! NoiseReader: decrypt half of a Noise-encrypted connection.
//!
//! Uses `StatelessTransportState` (&self) — safe for concurrent use
//! from a separate task. Nonce independently managed via AtomicU64.
//!
//! `dec_buf` is persistent — allocated once at handshake, reused for
//! every frame. Zeroized after each read for defense in depth.
//!
//! `max_frame_size` is checked at the length-prefix level BEFORE
//! allocation. An oversized frame is rejected without allocating
//! the payload buffer — this prevents allocation-based DoS.
//!
//! Nonce safety: the recv_nonce counter is advanced ONLY after a
//! successful decrypt. If AEAD verification fails, the nonce is NOT
//! consumed — the receiver can retry with the correct ciphertext at
//! the same nonce.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncRead;

use crate::error::{IpcError, IpcResult};
use crate::frame::codec::read_frame;

use super::MAX_NOISE_PLAINTEXT;

/// Read half of a Noise-encrypted connection.
pub struct NoiseReader {
    pub(super) state: Arc<snow::StatelessTransportState>,
    /// Persistent decrypt buffer. Reused across calls. Zeroized after each.
    pub(super) dec_buf: Vec<u8>,
    pub(super) remote_static: Option<Vec<u8>>,
    pub(super) recv_nonce: AtomicU64,
    /// Wire-level maximum per-chunk size: config.max_frame_size +
    /// WIRE_OVERHEAD_PER_CHUNK (25 bytes: 9 app framing + 16 AEAD tag).
    /// Checked BEFORE allocation in read_frame. Rejects oversized frames
    /// without allocating — prevents DoS via allocation pressure from
    /// malicious length prefixes.
    pub(super) max_frame_size: u32,
}

static_assertions::assert_impl_all!(NoiseReader: Send);

impl NoiseReader {
    /// Read and decrypt an application frame.
    ///
    /// Reads chunk count header, then reads and decrypts each chunk.
    /// Returns assembled plaintext as `Bytes`.
    ///
    /// Frame size is checked at the length-prefix level BEFORE allocation.
    /// An oversized frame returns `FrameTooLarge` without allocating.
    pub async fn read_encrypted_frame<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> IpcResult<bytes::Bytes> {
        // Read chunk count header (4 bytes, LE u32)
        let count_frame = read_frame(reader, self.max_frame_size).await?;
        if count_frame.len() != 4 {
            return Err(IpcError::InvalidChunkHeader {
                got: count_frame.len(),
            });
        }
        let chunk_count = u32::from_le_bytes(
            count_frame[..4].try_into().expect("checked len==4"),
        ) as usize;

        let max_chunks = (self.max_frame_size as usize).div_ceil(MAX_NOISE_PLAINTEXT);
        if chunk_count > max_chunks {
            return Err(IpcError::TooManyChunks {
                count: chunk_count,
                max: max_chunks,
            });
        }

        let mut output = bytes::BytesMut::with_capacity(chunk_count * MAX_NOISE_PLAINTEXT);

        if self.dec_buf.len() < MAX_NOISE_PLAINTEXT + 16 {
            self.dec_buf.resize(MAX_NOISE_PLAINTEXT + 16, 0);
        }

        for _ in 0..chunk_count {
            let ciphertext = read_frame(reader, self.max_frame_size).await?;
            let nonce = self.recv_nonce.load(Ordering::Relaxed);
            let len = self
                .state
                .read_message(nonce, &ciphertext, &mut self.dec_buf)
                .map_err(|e| IpcError::DecryptFailed {
                    reason: e.to_string(),
                })?;
            self.recv_nonce.fetch_add(1, Ordering::Relaxed);
            output.extend_from_slice(&self.dec_buf[..len]);
        }

        zeroize::Zeroize::zeroize(&mut self.dec_buf[..]);

        Ok(output.freeze())
    }

    pub fn remote_static(&self) -> Option<&[u8]> {
        self.remote_static.as_deref()
    }

    pub fn recv_nonce(&self) -> u64 {
        self.recv_nonce.load(Ordering::Relaxed)
    }
}
