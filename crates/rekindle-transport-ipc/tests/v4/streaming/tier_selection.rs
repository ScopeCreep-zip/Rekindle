//! TransferTier selection tests — validates the decision tree from the spec.
//!
//! select_tier(payload_len, arena, has_dmabuf_fd) → TransferTier
//!
//! Decision order:
//! 1. has_dmabuf_fd → DmaBuf (always, regardless of size)
//! 2. payload_len ≤ 65536 → Inline
//! 3. arena is Some and payload fits → SharedMem
//! 4. else → Inline (fallback)

use rekindle_transport_ipc::v4::streaming::TransferTier;
#[cfg(target_os = "linux")]
use rekindle_transport_ipc::v4::streaming::tier::select_tier;
#[cfg(target_os = "linux")]
use rekindle_transport_ipc::v4::streaming::shared_arena::SharedArena;

// ── DmaBuf takes priority over everything ───────────────────────

#[cfg(target_os = "linux")]
#[test]
fn dmabuf_fd_returns_dmabuf_regardless_of_size() {
    assert!(matches!(
        select_tier(0, None, true),
        TransferTier::DmaBuf
    ));
    assert!(matches!(
        select_tier(100_000_000, None, true),
        TransferTier::DmaBuf
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn dmabuf_fd_takes_priority_over_arena() {
    let arena = SharedArena::create(16 * 1024 * 1024, 4, false).expect("create");
    assert!(matches!(
        select_tier(1024, Some(&arena), true),
        TransferTier::DmaBuf
    ));
}

// ── Small payloads → Inline ─────────────────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn small_payload_returns_inline() {
    assert!(matches!(select_tier(0, None, false), TransferTier::Inline));
    assert!(matches!(select_tier(1, None, false), TransferTier::Inline));
    assert!(matches!(select_tier(65536, None, false), TransferTier::Inline));
}

#[cfg(target_os = "linux")]
#[test]
fn at_inline_threshold_returns_inline() {
    assert!(matches!(select_tier(65536, None, false), TransferTier::Inline));
}

#[cfg(target_os = "linux")]
#[test]
fn above_inline_threshold_without_arena_returns_inline() {
    assert!(matches!(select_tier(65537, None, false), TransferTier::Inline));
    assert!(matches!(select_tier(1_000_000, None, false), TransferTier::Inline));
}

// ── Payload fits in arena slot → SharedMem ──────────────────────

#[cfg(target_os = "linux")]
#[test]
fn payload_fits_arena_returns_shared_mem() {
    let arena = SharedArena::create(16 * 1024 * 1024, 4, false).expect("create");
    // 12 MiB payload fits in 16 MiB slot
    assert!(matches!(
        select_tier(12 * 1024 * 1024, Some(&arena), false),
        TransferTier::SharedMem
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn payload_exactly_slot_size_returns_shared_mem() {
    let arena = SharedArena::create(16 * 1024 * 1024, 4, false).expect("create");
    assert!(matches!(
        select_tier(arena.slot_size(), Some(&arena), false),
        TransferTier::SharedMem
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn payload_exceeds_slot_size_returns_inline() {
    let arena = SharedArena::create(4096, 4, false).expect("create");
    // 8192 > 4096 slot_size
    assert!(matches!(
        select_tier(8192, Some(&arena), false),
        TransferTier::Inline
    ));
}

// ── Edge cases ──────────────────────────────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn small_payload_with_arena_still_returns_inline() {
    let arena = SharedArena::create(16 * 1024 * 1024, 4, false).expect("create");
    // 1024 bytes ≤ 65536 — Inline wins even when arena is available
    assert!(matches!(
        select_tier(1024, Some(&arena), false),
        TransferTier::Inline
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn one_byte_above_inline_threshold_with_arena_returns_shared_mem() {
    let arena = SharedArena::create(16 * 1024 * 1024, 4, false).expect("create");
    assert!(matches!(
        select_tier(65537, Some(&arena), false),
        TransferTier::SharedMem
    ));
}

// ── NV12 4K frame size constant ─────────────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn nv12_4k_frame_fits_in_default_slot() {
    // 3840 × 2160 × 1.5 = 12,441,600 bytes
    const NV12_4K: usize = 3840 * 2160 * 3 / 2;
    assert_eq!(NV12_4K, 12_441_600);

    let arena = SharedArena::create(16 * 1024 * 1024, 8, false).expect("create");
    assert!(matches!(
        select_tier(NV12_4K, Some(&arena), false),
        TransferTier::SharedMem
    ));
}
