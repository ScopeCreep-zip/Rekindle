//! Streaming transport — shared-memory arena, DMA-BUF pass-through, tier selection.
//!
//! This module replaces v3's per-transfer handoff lifecycle with a persistent
//! shared-memory arena using atomic CAS slot coordination. The arena is created
//! once per connection (when both sides negotiate SHARED_ARENA capability) and
//! persists for the connection's lifetime.

/// Transfer tier — available on all platforms for caller routing logic.
/// The decision function `select_tier()` is Linux-only because it
/// references `SharedArena` which requires memfd/mmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferTier {
    /// Payload ≤ 64 KiB or no arena. Existing v1 AEAD socket path.
    Inline,
    /// Payload > 64 KiB, arena available. Zero-copy reader.
    SharedMem,
    /// Caller holds a DMA-BUF fd. GPU-to-GPU, zero CPU copy.
    DmaBuf,
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod memfd;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod sidechannel;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod shared_arena;

#[cfg(target_os = "linux")]
pub mod tier;

#[cfg(target_os = "linux")]
pub mod send;

pub mod dmabuf;

// ── Platform-independent wire format types ──────────────────────
//
// Canonical definitions live in shared_arena.rs (Linux). On non-Linux,
// fallback definitions provide the same wire types for codec decode/reject.

#[cfg(target_os = "linux")]
pub use shared_arena::{SharedMemRef, SlotRelease};

#[cfg(not(target_os = "linux"))]
mod fallback_types {
    /// Control message: writer published a slot (wire: 15 or 47 bytes).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct SharedMemRef {
        pub arena_id: u8,
        pub slot: u16,
        pub generation: u32,
        pub offset: u32,
        pub length: u32,
        pub digest: Option<[u8; 32]>,
    }

    /// Control message: reader finished with a slot (wire: 7 bytes).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct SlotRelease {
        pub arena_id: u8,
        pub slot: u16,
        pub generation: u32,
    }
}
#[cfg(not(target_os = "linux"))]
pub use fallback_types::{SharedMemRef, SlotRelease};

/// 3840 × 2160 × 1.5 bytes/pixel (NV12, 12bpp)
pub const NV12_4K_FRAME_SIZE: usize = 3840 * 2160 * 3 / 2;
const _: () = assert!(NV12_4K_FRAME_SIZE == 12_441_600);

/// 3840 × 2160 × 3 bytes/pixel (P010, 24bpp)
pub const P010_4K_FRAME_SIZE: usize = 3840 * 2160 * 3;
const _: () = assert!(P010_4K_FRAME_SIZE == 24_883_200);

/// 3840 × 2160 × 4 bytes/pixel (BGRA, 32bpp)
pub const BGRA_4K_FRAME_SIZE: usize = 3840 * 2160 * 4;
const _: () = assert!(BGRA_4K_FRAME_SIZE == 33_177_600);
