//! Transfer tier selection — Linux only.
//!
//! Routes payloads to the optimal transport path based on size,
//! arena availability, and DMA-BUF fd presence.
//!
//! Decision order:
//! 1. Caller holds a DMA-BUF fd → DmaBuf (always, regardless of size)
//! 2. Payload ≤ 64 KiB → Inline (v1 AEAD socket path)
//! 3. Arena available and payload fits → SharedMem (1 memcpy writer, zero-copy reader)
//! 4. Else → Inline (fallback, v1 chunked bulk)

use super::TransferTier;
use super::shared_arena::SharedArena;

/// Payloads at or below this size use the v1 AEAD socket path.
pub const INLINE_THRESHOLD: usize = 65536;

/// Select the optimal transfer tier for a payload.
///
/// `payload_len`: size of the payload in bytes.
/// `arena`: the shared-memory arena, if negotiated for this connection.
/// `has_dmabuf_fd`: true if the caller holds a DMA-BUF fd for GPU-to-GPU transfer.
#[inline]
pub fn select_tier(
    payload_len: usize,
    arena: Option<&SharedArena>,
    has_dmabuf_fd: bool,
) -> TransferTier {
    if has_dmabuf_fd {
        return TransferTier::DmaBuf;
    }
    if payload_len <= INLINE_THRESHOLD {
        return TransferTier::Inline;
    }
    match arena {
        Some(a) if payload_len <= a.slot_size() => TransferTier::SharedMem,
        _ => TransferTier::Inline,
    }
}
