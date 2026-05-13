//! NoiseWriter: encrypt half of a Noise-encrypted connection.
//!
//! Uses `StatelessTransportState` which takes `&self` for encrypt,
//! enabling concurrent use from a dedicated write task without
//! blocking reads. Nonce management is explicit via `AtomicU64`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWrite;

use crate::ipc::error::{IpcError, Result};
use crate::ipc::framing::{write_frame, MAX_FRAME_SIZE};
use super::MAX_NOISE_PLAINTEXT;

/// Write half of a Noise-encrypted connection.
pub struct NoiseWriter {
    pub(super) state: Arc<snow::StatelessTransportState>,
    pub(super) enc_buf: Vec<u8>,
    pub(super) send_nonce: AtomicU64,
}

static_assertions::assert_impl_all!(NoiseWriter: Send);

impl NoiseWriter {
    /// Write an encrypted application frame.
    ///
    /// Payload is chunked into Noise messages with explicit nonces.
    /// The send nonce counter auto-increments atomically per chunk.
    pub async fn write_encrypted_frame<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        payload: &[u8],
    ) -> Result<()> {
        #[allow(clippy::cast_possible_truncation)]
        if payload.len() > MAX_FRAME_SIZE as usize {
            return Err(IpcError::FrameTooLarge {
                size: payload.len() as u32,
                max: MAX_FRAME_SIZE,
            });
        }

        let chunk_count = if payload.is_empty() {
            1
        } else {
            payload.len().div_ceil(MAX_NOISE_PLAINTEXT)
        };

        let count_bytes = u32::try_from(chunk_count).map_err(|_| IpcError::TooManyChunks {
            count: chunk_count,
            max: 256,
        })?;
        write_frame(writer, &count_bytes.to_be_bytes()).await?;

        if self.enc_buf.len() < MAX_NOISE_PLAINTEXT + 16 {
            self.enc_buf.resize(MAX_NOISE_PLAINTEXT + 16, 0);
        }

        for chunk_idx in 0..chunk_count {
            let start = chunk_idx * MAX_NOISE_PLAINTEXT;
            let end = (start + MAX_NOISE_PLAINTEXT).min(payload.len());
            let chunk = &payload[start..end];

            let nonce = self.send_nonce.fetch_add(1, Ordering::Relaxed);
            let len = self
                .state
                .write_message(nonce, chunk, &mut self.enc_buf)
                .map_err(|e| IpcError::EncryptFailed {
                    reason: e.to_string(),
                })?;

            write_frame(writer, &self.enc_buf[..len]).await?;
        }

        Ok(())
    }

    /// Current send nonce value (for diagnostics/testing).
    pub fn sending_nonce(&self) -> u64 {
        self.send_nonce.load(Ordering::Relaxed)
    }
}
