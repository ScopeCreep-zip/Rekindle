//! NoiseWriter: encrypt half of a Noise-encrypted connection.
//!
//! Uses `StatelessTransportState` (&self) — safe for concurrent use.
//! `enc_buf` is persistent — allocated once, reused for every frame.
//!
//! `max_frame_size` is checked BEFORE chunking. An oversized payload
//! is rejected with `FrameTooLarge` before any encryption or I/O.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWrite;

use crate::error::{IpcError, IpcResult};
use crate::frame::codec::write_frame;

use super::MAX_NOISE_PLAINTEXT;

/// Write half of a Noise-encrypted connection.
pub struct NoiseWriter {
    pub(super) state: Arc<snow::StatelessTransportState>,
    /// Persistent encrypt buffer. Reused across calls.
    pub(super) enc_buf: Vec<u8>,
    pub(super) send_nonce: AtomicU64,
    /// Wire-level maximum per-chunk size: config.max_frame_size +
    /// WIRE_OVERHEAD_PER_CHUNK (25 bytes: 9 app framing + 16 AEAD tag).
    /// Rejects payloads exceeding this limit before encryption.
    /// The application-level limit (without overhead) is enforced at
    /// send_frame for graceful rejection. This check is defense-in-depth
    /// for tagged frames entering the write task.
    pub(super) max_frame_size: u32,
}

static_assertions::assert_impl_all!(NoiseWriter: Send);

impl NoiseWriter {
    /// Write an encrypted application frame.
    ///
    /// Chunks payload into Noise messages with explicit nonces.
    /// Writes chunk count header, then each encrypted chunk.
    /// Does NOT flush — caller is responsible.
    ///
    /// Rejects payloads exceeding `max_frame_size` with `FrameTooLarge`
    /// before any encryption or I/O.
    #[allow(clippy::cast_possible_truncation)]
    pub async fn write_encrypted_frame<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        payload: &[u8],
    ) -> IpcResult<()> {
        if payload.len() > self.max_frame_size as usize {
            return Err(IpcError::FrameTooLarge {
                size: payload.len() as u32,
                max: self.max_frame_size,
            });
        }

        let chunk_count = if payload.is_empty() {
            1
        } else {
            payload.len().div_ceil(MAX_NOISE_PLAINTEXT)
        };

        let count_bytes = (chunk_count as u32).to_le_bytes();
        write_frame(writer, &count_bytes).await?;

        if self.enc_buf.len() < MAX_NOISE_PLAINTEXT + 16 {
            self.enc_buf.resize(MAX_NOISE_PLAINTEXT + 16, 0);
        }

        for i in 0..chunk_count {
            let start = i * MAX_NOISE_PLAINTEXT;
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

    pub fn send_nonce(&self) -> u64 {
        self.send_nonce.load(Ordering::Relaxed)
    }
}
