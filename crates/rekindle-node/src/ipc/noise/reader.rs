//! NoiseReader: decrypt half of a Noise-encrypted connection.
//!
//! Uses `StatelessTransportState` which takes `&self` for decrypt,
//! enabling concurrent use from a separate task without blocking writes.
//! Nonce management is explicit via `AtomicU64`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncRead;

use crate::ipc::error::{IpcError, Result};
use crate::ipc::framing::{read_frame, MAX_FRAME_SIZE};
use super::MAX_NOISE_PLAINTEXT;

/// Read half of a Noise-encrypted connection.
pub struct NoiseReader {
    pub(super) state: Arc<snow::StatelessTransportState>,
    pub(super) read_buf: bytes::BytesMut,
    pub(super) dec_buf: Vec<u8>,
    pub(super) remote_static: Option<Vec<u8>>,
    pub(super) recv_nonce: AtomicU64,
}

static_assertions::assert_impl_all!(NoiseReader: Send);

impl NoiseReader {
    /// Read and decrypt an application frame.
    ///
    /// Reads chunk count, then reads and decrypts each chunk with
    /// explicit nonces. Returns `Bytes` via persistent `BytesMut`.
    pub async fn read_encrypted_frame<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> Result<bytes::Bytes> {
        let count_frame = read_frame(reader).await?;
        if count_frame.len() != 4 {
            let preview: Vec<u8> = count_frame.iter().copied().take(32).collect();
            tracing::error!(
                got = count_frame.len(),
                preview = %hex::encode(&preview),
                "read_encrypted_frame: chunk count header misaligned"
            );
            return Err(IpcError::InvalidChunkHeader {
                got: count_frame.len(),
            });
        }
        let chunk_count = u32::from_be_bytes([
            count_frame[0], count_frame[1], count_frame[2], count_frame[3],
        ]) as usize;

        let max_chunks = (MAX_FRAME_SIZE as usize).div_ceil(MAX_NOISE_PLAINTEXT);
        if chunk_count > max_chunks {
            return Err(IpcError::TooManyChunks {
                count: chunk_count,
                max: max_chunks,
            });
        }

        let needed = chunk_count * MAX_NOISE_PLAINTEXT;
        if self.read_buf.capacity() < needed {
            self.read_buf.reserve(needed - self.read_buf.capacity());
        }

        if self.dec_buf.len() < MAX_NOISE_PLAINTEXT + 16 {
            self.dec_buf.resize(MAX_NOISE_PLAINTEXT + 16, 0);
        }

        for _ in 0..chunk_count {
            let ciphertext = read_frame(reader).await?;
            let nonce = self.recv_nonce.fetch_add(1, Ordering::Relaxed);
            let len = self
                .state
                .read_message(nonce, &ciphertext, &mut self.dec_buf)
                .map_err(|e| IpcError::DecryptFailed {
                    reason: e.to_string(),
                })?;
            self.read_buf.extend_from_slice(&self.dec_buf[..len]);
        }

        zeroize::Zeroize::zeroize(&mut self.dec_buf[..]);

        let plaintext_len = self.read_buf.len();
        if let Ok(size) = u32::try_from(plaintext_len) {
            if size > MAX_FRAME_SIZE {
                self.read_buf.clear();
                return Err(IpcError::FrameTooLarge {
                    size,
                    max: MAX_FRAME_SIZE,
                });
            }
        } else {
            self.read_buf.clear();
            return Err(IpcError::FrameTooLarge {
                size: u32::MAX,
                max: MAX_FRAME_SIZE,
            });
        }

        Ok(self.read_buf.split().freeze())
    }

    /// The remote party's static public key (after handshake).
    pub fn remote_static(&self) -> Option<&[u8]> {
        self.remote_static.as_deref()
    }

    /// Current receive nonce value (for diagnostics/testing).
    pub fn receiving_nonce(&self) -> u64 {
        self.recv_nonce.load(Ordering::Relaxed)
    }
}
