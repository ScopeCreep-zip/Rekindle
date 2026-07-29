//! SharedArena — persistent shared-memory arena with atomic CAS slot coordination.
//!
//! # Architecture
//!
//! The arena is two separate memfds:
//! - **Arena memfd**: `slot_count × slot_size` bytes of payload data. Both
//!   processes mmap this MAP_SHARED and read/write slot regions directly.
//! - **States memfd**: `slot_count × 64` bytes of cache-line-padded AtomicU64
//!   slot states. Separate from the arena to prevent false sharing between
//!   state cachelines and payload data.
//!
//! # Three-State Writer-Owned Model
//!
//! ```text
//!   FREE(N) ─→ ACQUIRED(N) ─→ IN_FLIGHT(N) ─→ FREE(N+1)
//!                   │
//!                   └── Drop(SlotGuard) ─→ FREE(N) [rollback, gen unchanged]
//! ```
//!
//! All state transitions are `compare_exchange` operations. The reader validates
//! via atomic load, reads the payload (zero-copy pointer), and sends SlotRelease
//! over the encrypted socket. The writer CAS's `IN_FLIGHT(N) → FREE(N+1)` on
//! receipt of SlotRelease.
//!
//! # Cross-Process Shared Memory and Rust's Aliasing Model
//!
//! MAP_SHARED memory is inherently outside Rust's aliasing model. Both processes
//! have writable mappings to the same physical pages. A misbehaving peer could
//! write to any offset at any time — Rust cannot enforce exclusivity across
//! process boundaries. This module uses raw pointer operations (`ptr::copy`,
//! `slice::from_raw_parts` for reads) instead of `&mut [u8]` references to
//! avoid asserting aliasing guarantees that the hardware does not enforce.
//! The CAS state machine provides the protocol-level exclusivity guarantee,
//! but a compromised peer can violate it silently.
//!
//! # Safety Invariants
//!
//! - `arena_base` is a non-null pointer to a MAP_SHARED mmap region valid for
//!   the arena's lifetime. `munmap` is called in `Drop`.
//! - `states_base` is a non-null pointer to a MAP_SHARED mmap region of
//!   `PaddedSlotState` entries. All access goes through `AtomicU64` operations.
//! - Slot regions are disjoint within the protocol. Only one `SlotGuard` can
//!   exist per slot at a time, enforced by the CAS transition `FREE → ACQUIRED`.
//! - Cross-process atomics on MAP_SHARED pages are correct because the kernel
//!   maps the same physical pages into both address spaces. `AtomicU64`
//!   operations use compiler atomic intrinsics that emit the correct barriers
//!   per architecture (LOCK CMPXCHG on x86-64, LDAXR/STLXR on ARM64).

use std::io;
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::memfd;

// ── Slot states ─────────────────────────────────────────────────

const FREE: u32 = 0;
const ACQUIRED: u32 = 1;
const IN_FLIGHT: u32 = 2;

/// Pack generation and state into a single u64.
/// Layout: `[generation:32][state:32]`
#[inline(always)]
const fn pack(generation: u32, state: u32) -> u64 {
    ((generation as u64) << 32) | (state as u64)
}

#[inline(always)]
const fn unpack_state(packed: u64) -> u32 {
    (packed & 0xFFFF_FFFF) as u32
}

#[inline(always)]
const fn unpack_generation(packed: u64) -> u32 {
    (packed >> 32) as u32
}

// ── Cache-line padded state ─────────────────────────────────────

/// Each slot state occupies one full cache line (64 bytes) to prevent
/// false sharing between adjacent slots under concurrent CAS operations.
#[repr(C, align(64))]
pub(crate) struct PaddedSlotState {
    pub(crate) state: AtomicU64,
    _pad: [u8; 56],
}

const _: () = assert!(core::mem::size_of::<PaddedSlotState>() == 64);
const _: () = assert!(core::mem::align_of::<PaddedSlotState>() == 64);

// ── Error type ──────────────────────────────────────────────────

#[derive(Debug)]
pub enum ArenaError {
    Memfd(memfd::MemfdError),
    MmapFailed(io::Error),
    InvalidConfig(String),
}

impl From<memfd::MemfdError> for ArenaError {
    fn from(e: memfd::MemfdError) -> Self {
        ArenaError::Memfd(e)
    }
}

impl std::fmt::Display for ArenaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memfd(e) => write!(f, "memfd: {e}"),
            Self::MmapFailed(e) => write!(f, "mmap failed: {e}"),
            Self::InvalidConfig(msg) => write!(f, "invalid config: {msg}"),
        }
    }
}

impl std::error::Error for ArenaError {}

// ── Wire types ──────────────────────────────────────────────────

// Compile-time verification of wire overhead constants.
// If ENVELOPE_LEN or AEAD_TAG_LEN change, these assertions fail at compile time
// and the wire overhead documentation below must be updated.
const _: () = assert!(crate::v4::wire::constants::ENVELOPE_LEN == 32,
    "ENVELOPE_LEN changed — update SharedMemRef wire overhead documentation");
const _: () = assert!(crate::v4::wire::constants::AEAD_TAG_LEN == 16,
    "AEAD_TAG_LEN changed — update SharedMemRef wire overhead documentation");

/// Control message sent over the Noise IK encrypted socket when a slot is
/// published. Variable-length on wire: 15 bytes (no integrity) or 47 bytes
/// (with BLAKE3 digest). See codec::streaming::arena_write for encoding.
///
/// # Wire overhead
///
/// AEAD frame structure: 32-byte envelope + 2-byte prefix + payload + 16-byte tag.
///
/// No integrity (15 bytes payload): 32 + 2 + 15 + 16 = 65 bytes wire. 77% overhead.
/// With integrity (47 bytes payload): 32 + 2 + 47 + 16 = 97 bytes wire. 52% overhead.
/// SlotRelease (7 bytes payload): 32 + 2 + 7 + 16 = 57 bytes wire. 88% overhead.
///
/// At 120fps single stream: (65 + 57) × 120 = 14,640 bytes/sec = 14.3 KB/s.
/// At 8 streams: 117 KB/s. Data plane: 12,441,600 × 120fps = 1.49 GiB/s.
/// Control overhead is 0.008% of data bandwidth.
///
/// The overhead is inherent to AEAD authentication — the SharedMemRef must be
/// authenticated to prevent slot reference forgery by a malicious peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SharedMemRef {
    pub arena_id: u8,
    pub slot: u16,
    pub generation: u32,
    pub offset: u32,
    pub length: u32,
    /// Present only when integrity=1 negotiated in ArenaSetup.
    /// None when integrity=0 (production fast path).
    pub digest: Option<[u8; 32]>,
}

/// Control message sent over the Noise IK encrypted socket when the reader
/// is done with a slot. 7 bytes on wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotRelease {
    pub arena_id: u8,
    pub slot: u16,
    pub generation: u32,
}

// ── Frame size constants ────────────────────────────────────────

/// 3840 × 2160 × 1.5 bytes/pixel (NV12, 12bpp)
pub const NV12_4K_FRAME_SIZE: usize = 3840 * 2160 * 3 / 2;
const _: () = assert!(NV12_4K_FRAME_SIZE == 12_441_600);

/// 3840 × 2160 × 3 bytes/pixel (P010, 24bpp effective for 10-bit)
pub const P010_4K_FRAME_SIZE: usize = 3840 * 2160 * 3;
const _: () = assert!(P010_4K_FRAME_SIZE == 24_883_200);

/// 3840 × 2160 × 4 bytes/pixel (BGRA, 32bpp)
pub const BGRA_4K_FRAME_SIZE: usize = 3840 * 2160 * 4;
const _: () = assert!(BGRA_4K_FRAME_SIZE == 33_177_600);

// ── SharedArena ─────────────────────────────────────────────────

pub struct SharedArena {
    /// Pointer to the arena memfd mmap region.
    arena_base: NonNull<u8>,
    /// Total arena mmap size in bytes.
    arena_size: usize,
    /// Pointer to the states memfd mmap region.
    states_base: NonNull<PaddedSlotState>,
    /// Total states mmap size in bytes.
    states_size: usize,
    /// Number of bytes per slot (page-aligned).
    slot_size: usize,
    /// Number of slots.
    slot_count: usize,
    /// Whether to compute BLAKE3 on publish / verify on read.
    integrity_check: bool,
    /// Scan hint for try_acquire. Distributes slot usage evenly.
    /// Relaxed ordering: performance hint, not correctness invariant.
    /// Worst case of stale read: two threads start scanning from the
    /// same slot, slightly less efficient. The CAS on the slot state
    /// is the correctness mechanism.
    next_slot: AtomicUsize,
    /// Arena memfd — kept open for the arena's lifetime and accessible
    /// via `arena_fd()` for fd passing to the peer during setup.
    arena_fd: OwnedFd,
    /// States memfd — kept open for the arena's lifetime and accessible
    /// via `states_fd()` for fd passing to the peer during setup.
    states_fd: OwnedFd,
}

// SAFETY: SharedArena's pointers are to MAP_SHARED mmap regions backed
// by memfds. These regions are valid for the arena's lifetime (munmap
// in Drop). All slot access is mediated by AtomicU64 CAS on the states
// array, which provides the necessary synchronization:
// - Only one SlotGuard can exist per slot (CAS FREE→ACQUIRED enforces)
// - Payload writes are ordered before state transitions via Release fence
// - Payload reads are ordered after state verification via Acquire fence
// The mmap region is MAP_SHARED so both processes see the same physical
// pages. AtomicUsize for next_slot is Sync. OwnedFd is Send + Sync.
unsafe impl Send for SharedArena {}
unsafe impl Sync for SharedArena {}

impl SharedArena {
    /// Create a new arena with the given slot parameters.
    ///
    /// `slot_size` is page-aligned upward. `slot_count` must be > 0 and <= 65535.
    pub fn create(
        slot_size: usize,
        slot_count: usize,
        integrity_check: bool,
    ) -> Result<Self, ArenaError> {
        if slot_size == 0 {
            return Err(ArenaError::InvalidConfig("slot_size must be > 0".into()));
        }
        if slot_count == 0 {
            return Err(ArenaError::InvalidConfig("slot_count must be > 0".into()));
        }
        if slot_count > u16::MAX as usize {
            return Err(ArenaError::InvalidConfig(
                format!("slot_count {slot_count} exceeds u16::MAX"),
            ));
        }

        let slot_size = memfd::page_align(slot_size);
        if slot_size > u32::MAX as usize {
            return Err(ArenaError::InvalidConfig(
                format!("page-aligned slot_size {slot_size} exceeds u32::MAX"),
            ));
        }
        let arena_size = slot_size
            .checked_mul(slot_count)
            .ok_or_else(|| ArenaError::InvalidConfig("arena size overflow".into()))?;
        let states_size = slot_count
            .checked_mul(core::mem::size_of::<PaddedSlotState>())
            .ok_or_else(|| ArenaError::InvalidConfig("states size overflow".into()))?;

        let arena_fd = memfd::create_arena_memfd("ipc_arena", arena_size as u64)?;
        let states_fd = memfd::create_arena_memfd("ipc_states", states_size as u64)?;

        // SAFETY: arena_fd is a valid sealed memfd of arena_size bytes.
        let arena_base = unsafe { memfd::map_readwrite(arena_fd.as_raw_fd(), arena_size)? };
        // SAFETY: states_fd is a valid sealed memfd of states_size bytes.
        let states_base = unsafe { memfd::map_readwrite(states_fd.as_raw_fd(), states_size)? };

        // Initialize all slots to FREE(generation=0)
        // SAFETY: states_base is a valid mmap'd region of states_size bytes,
        // which equals slot_count × sizeof(PaddedSlotState). Each entry is
        // accessed through AtomicU64 — no non-atomic writes.
        let states = unsafe {
            core::slice::from_raw_parts(
                states_base.cast::<PaddedSlotState>(),
                slot_count,
            )
        };
        for slot in states {
            slot.state.store(pack(0, FREE), Ordering::Relaxed);
        }
        // Ensure initialization is visible before any reader attaches.
        core::sync::atomic::fence(Ordering::Release);

        Ok(SharedArena {
            arena_base: NonNull::new(arena_base).expect("mmap returned null"),
            arena_size,
            states_base: NonNull::new(states_base.cast()).expect("mmap returned null"),
            states_size,
            slot_size,
            slot_count,
            integrity_check,
            next_slot: AtomicUsize::new(0),
            arena_fd,
            states_fd,
        })
    }

    /// Import an arena from received fds. Verifies seals and sizes.
    ///
    /// Called by the peer after receiving arena_fd and states_fd via
    /// sidechannel SCM_RIGHTS, and ArenaSetup metadata via the encrypted
    /// control channel.
    pub fn receive_and_import(
        arena_fd: OwnedFd,
        states_fd: OwnedFd,
        slot_size: usize,
        slot_count: usize,
        integrity_check: bool,
    ) -> Result<Self, ArenaError> {
        if slot_size == 0 || slot_count == 0 {
            return Err(ArenaError::InvalidConfig("slot_size and slot_count must be > 0".into()));
        }
        if slot_count > u16::MAX as usize {
            return Err(ArenaError::InvalidConfig(
                format!("slot_count {slot_count} exceeds u16::MAX — peer sent invalid ArenaSetup"),
            ));
        }

        let slot_size = memfd::page_align(slot_size);
        let arena_size = slot_size
            .checked_mul(slot_count)
            .ok_or_else(|| ArenaError::InvalidConfig("arena size overflow".into()))?;
        let states_size = slot_count
            .checked_mul(core::mem::size_of::<PaddedSlotState>())
            .ok_or_else(|| ArenaError::InvalidConfig("states size overflow".into()))?;

        // Verify seals before mapping — prevents the owner from resizing
        // after we've mapped the region.
        memfd::verify_arena_seals(arena_fd.as_raw_fd())?;
        memfd::verify_arena_seals(states_fd.as_raw_fd())?;

        // Verify sizes match the ArenaSetup metadata.
        memfd::verify_fd_size(arena_fd.as_raw_fd(), arena_size as u64)?;
        memfd::verify_fd_size(states_fd.as_raw_fd(), states_size as u64)?;

        // SAFETY: arena_fd is a verified sealed memfd of arena_size bytes.
        let arena_base = unsafe { memfd::map_readwrite(arena_fd.as_raw_fd(), arena_size)? };
        // SAFETY: states_fd is a verified sealed memfd of states_size bytes.
        let states_base = unsafe { memfd::map_readwrite(states_fd.as_raw_fd(), states_size)? };

        Ok(SharedArena {
            arena_base: NonNull::new(arena_base).expect("mmap returned null"),
            arena_size,
            states_base: NonNull::new(states_base.cast()).expect("mmap returned null"),
            states_size,
            slot_size,
            slot_count,
            integrity_check,
            next_slot: AtomicUsize::new(0),
            arena_fd,
            states_fd,
        })
    }

    // ── Accessors ────────────────────────────────────────────────

    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    pub fn slot_count(&self) -> usize {
        self.slot_count
    }

    pub fn integrity_check(&self) -> bool {
        self.integrity_check
    }

    /// Borrow the arena memfd for fd passing via sidechannel.
    /// Used by the server to send the arena fd to the client during setup.
    pub fn arena_fd(&self) -> BorrowedFd<'_> {
        self.arena_fd.as_fd()
    }

    /// Borrow the states memfd for fd passing via sidechannel.
    /// Used by the server to send the states fd to the client during setup.
    pub fn states_fd(&self) -> BorrowedFd<'_> {
        self.states_fd.as_fd()
    }

    /// Get the base pointer of the arena mmap region.
    /// Used by tests to verify zero-copy (pointer range assertions).
    pub fn arena_base_ptr(&self) -> *const u8 {
        self.arena_base.as_ptr()
    }

    /// Total arena mmap size in bytes.
    pub fn arena_size(&self) -> usize {
        self.arena_size
    }

    // ── Internal helpers ─────────────────────────────────────────

    fn states(&self) -> &[PaddedSlotState] {
        // SAFETY: states_base points to slot_count × PaddedSlotState entries
        // in a MAP_SHARED mmap region valid for the arena's lifetime.
        // PaddedSlotState contains AtomicU64 which has interior mutability,
        // so shared reference (&) with mutation through atomics is sound.
        unsafe {
            core::slice::from_raw_parts(
                self.states_base.as_ptr(),
                self.slot_count,
            )
        }
    }

    /// Return a raw pointer to the start of a slot's memory region.
    /// Caller is responsible for bounds and aliasing. Does NOT create
    /// a Rust reference — avoids asserting aliasing guarantees that
    /// cross-process MAP_SHARED cannot enforce.
    #[inline]
    fn slot_ptr(&self, slot: u16) -> *mut u8 {
        // SAFETY: slot < slot_count (caller must verify).
        // offset is within [0, arena_size).
        let offset = slot as usize * self.slot_size;
        unsafe { self.arena_base.as_ptr().add(offset) }
    }

    // ── Writer API ───────────────────────────────────────────────

    /// Try to acquire a free slot for writing.
    ///
    /// Scans from a rotating hint to distribute slot usage evenly.
    /// Returns `None` if all slots are busy — the caller decides policy
    /// (drop frame, backpressure, spin). Never blocks.
    ///
    /// The returned `SlotGuard` provides write access to the slot's
    /// memory region via raw pointers. On drop without calling `release()`,
    /// the slot reverts to `FREE` with the same generation (rollback).
    #[inline]
    pub fn try_acquire(&self) -> Option<SlotGuard<'_>> {
        let start = self.next_slot.load(Ordering::Relaxed);

        for i in 0..self.slot_count {
            let idx = (start + i) % self.slot_count;
            let slot_state = &self.states()[idx];
            let current = slot_state.state.load(Ordering::Acquire);

            if unpack_state(current) == FREE {
                let gen = unpack_generation(current);
                let desired = pack(gen, ACQUIRED);

                // Strong CAS — no retry loop on same slot. On failure
                // (another writer grabbed it), try next slot.
                match slot_state.state.compare_exchange(
                    current,
                    desired,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        // Advance hint past this slot for next acquire.
                        self.next_slot.store(
                            (idx + 1) % self.slot_count,
                            Ordering::Relaxed,
                        );
                        return Some(SlotGuard {
                            arena: self,
                            slot: idx as u16,
                            generation: gen,
                        });
                    }
                    Err(_) => continue,
                }
            }
        }

        // All slots busy.
        None
    }

    // ── Reader API ───────────────────────────────────────────────

    /// Validate a SharedMemRef against the arena state and return a
    /// zero-copy slice to the slot data.
    ///
    /// Returns `None` if:
    /// - slot index is out of bounds
    /// - slot state is not IN_FLIGHT
    /// - generation doesn't match (stale/replayed SharedMemRef)
    /// - offset + length exceeds slot_size
    /// - BLAKE3 integrity check fails (when enabled)
    ///
    /// The returned slice borrows the arena. The caller must send
    /// `SlotRelease` over the control channel when done, and the writer
    /// calls `return_slot()` on receipt.
    ///
    /// # Safety note on the returned slice
    ///
    /// The `&[u8]` is constructed from `slice::from_raw_parts` on the mmap'd
    /// region. The writer has transitioned the slot to IN_FLIGHT and will not
    /// modify the data until `return_slot()` is called. A misbehaving peer
    /// process could write to this memory concurrently — the protocol prevents
    /// this but the hardware does not enforce it. This is an accepted property
    /// of cross-process shared memory.
    #[inline]
    pub fn read_ref<'a>(&'a self, shmref: &SharedMemRef) -> Option<&'a [u8]> {
        if shmref.slot as usize >= self.slot_count {
            return None;
        }

        let current = self.states()[shmref.slot as usize]
            .state
            .load(Ordering::Acquire);

        if unpack_state(current) != IN_FLIGHT {
            return None;
        }
        if unpack_generation(current) != shmref.generation {
            return None;
        }

        // Acquire fence pairs with writer's Release fence in release().
        // All payload writes by the writer are visible after this point.
        core::sync::atomic::fence(Ordering::Acquire);

        let offset = shmref.offset as usize;
        let length = shmref.length as usize;

        // Bounds check with overflow protection.
        if offset.saturating_add(length) > self.slot_size {
            return None;
        }

        let slot_base = shmref.slot as usize * self.slot_size;
        // SAFETY: slot < slot_count (checked above). offset + length <= slot_size
        // (checked above). The slot region is within the arena mmap.
        //
        // Aliasing correctness: we create &[u8] (shared reference), not &mut [u8].
        // Under Stacked Borrows / Tree Borrows, &[u8] requires that no *exclusive*
        // write (via &mut or raw pointer within this process) occurs to this memory
        // while the reference is live. The writer has CAS'd to IN_FLIGHT — it will
        // not write until return_slot() is called after the reader sends SlotRelease.
        // Within our process's aliasing domain, this invariant holds.
        //
        // A misbehaving peer process with MAP_SHARED writable access could write to
        // these pages concurrently. This would violate the protocol but NOT the Rust
        // aliasing model — Stacked Borrows tracks borrows per-allocation within a
        // single process's address space. The peer's writes go through a separate
        // page table entry and are not modeled by Rust's borrow checker. This is the
        // fundamental property of MAP_SHARED: the hardware allows cross-process
        // concurrent access that Rust's aliasing model does not track.
        let data = unsafe {
            core::slice::from_raw_parts(
                self.arena_base.as_ptr().add(slot_base + offset),
                length,
            )
        };

        // Integrity verification when negotiated and digest is present.
        if self.integrity_check {
            if let Some(digest) = shmref.digest {
                let computed: [u8; 32] = blake3::hash(data).into();
                if computed != digest {
                    return None;
                }
            }
        }

        Some(data)
    }

    /// Reclaim a slot after the reader is done. Called by the writer
    /// after receiving SlotRelease from the reader over the control channel.
    ///
    /// Returns `true` if the slot was successfully transitioned
    /// `IN_FLIGHT(N) → FREE(N+1)`. Returns `false` if the CAS fails
    /// (double return, stale release, or slot already reclaimed).
    #[inline]
    pub fn return_slot(&self, release: &SlotRelease) -> bool {
        let idx = release.slot as usize;
        if idx >= self.slot_count {
            return false;
        }

        let expected = pack(release.generation, IN_FLIGHT);
        let desired = pack(release.generation.wrapping_add(1), FREE);

        self.states()[idx]
            .state
            .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Force-reclaim all IN_FLIGHT slots. Called when the reader
    /// disconnects (socket EOF) to prevent permanent slot leaks.
    ///
    /// Each IN_FLIGHT(N) slot is transitioned to FREE(N+1).
    /// Best-effort CAS — if a slot was already transitioned by a
    /// concurrent return_slot(), the CAS harmlessly fails.
    pub fn reclaim_all_inflight(&self) {
        for i in 0..self.slot_count {
            let current = self.states()[i].state.load(Ordering::Acquire);
            if unpack_state(current) == IN_FLIGHT {
                let gen = unpack_generation(current);
                let desired = pack(gen.wrapping_add(1), FREE);
                let _ = self.states()[i].state.compare_exchange(
                    current,
                    desired,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
    }
}

impl Drop for SharedArena {
    fn drop(&mut self) {
        // SAFETY: arena_base and states_base were obtained from mmap with
        // the exact sizes stored in arena_size and states_size. munmap
        // must be called with exactly the same pointer and length.
        unsafe {
            libc::munmap(self.arena_base.as_ptr().cast(), self.arena_size);
            libc::munmap(self.states_base.as_ptr().cast(), self.states_size);
        }
        // OwnedFd fields (arena_fd, states_fd) are dropped automatically,
        // closing the fds. The kernel keeps the shmem inode alive until all
        // mmaps are unmapped AND all fds are closed. Our munmap + fd close
        // is the full cleanup.
    }
}

// ── SlotGuard ───────────────────────────────────────────────────

/// RAII guard for exclusive write access to an arena slot.
///
/// Provides write access via raw pointer operations (`write_from_slice`,
/// `as_mut_ptr`) instead of `&mut [u8]` references. This avoids asserting
/// Rust aliasing guarantees that cross-process MAP_SHARED memory cannot
/// enforce. See module-level doc on the aliasing model.
///
/// On drop without calling `release()`: CAS rollback `ACQUIRED(N) → FREE(N)`
/// with the same generation (no SharedMemRef was sent, no stale reference
/// to invalidate).
///
/// On `release()`: CAS transition `ACQUIRED(N) → IN_FLIGHT(N)`, returns a
/// `SharedMemRef` for sending over the control channel. Consumes the guard
/// to prevent double-publish.
pub struct SlotGuard<'a> {
    arena: &'a SharedArena,
    slot: u16,
    generation: u32,
}

impl<'a> SlotGuard<'a> {
    /// Copy `src` into the slot at offset 0.
    ///
    /// Uses `ptr::copy_nonoverlapping` — no `&mut` reference is created,
    /// avoiding aliasing assertions on MAP_SHARED memory.
    ///
    /// # Panics
    ///
    /// Panics if `src.len()` exceeds slot_size.
    #[inline]
    pub fn write_from_slice(&mut self, src: &[u8]) {
        assert!(
            src.len() <= self.arena.slot_size,
            "write_from_slice: src.len() {} exceeds slot_size {}",
            src.len(),
            self.arena.slot_size,
        );
        let dst = self.arena.slot_ptr(self.slot);
        // SAFETY: dst is a valid mmap'd region of slot_size bytes.
        // src.len() <= slot_size (checked above). The regions do not
        // overlap (src is caller stack/heap, dst is mmap'd arena).
        // No &mut reference exists — we use raw pointer copy.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
        }
    }

    /// Raw pointer for direct writes from capture backends that
    /// want to write directly into the arena (e.g., V4L2 USERPTR).
    ///
    /// The caller must not write beyond `slot_size` bytes from this pointer.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.arena.slot_ptr(self.slot)
    }

    /// Slot size in bytes — convenience for callers that need to know
    /// the maximum writable extent.
    pub fn slot_size(&self) -> usize {
        self.arena.slot_size
    }

    #[inline]
    pub fn slot_index(&self) -> u16 {
        self.slot
    }

    #[inline]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Publish the slot: CAS `ACQUIRED(N) → IN_FLIGHT(N)`.
    ///
    /// Consumes the guard to prevent double-publish. Returns a
    /// `SharedMemRef` for sending over the control channel.
    ///
    /// `arena_id` identifies which arena on the connection (for multi-stream).
    /// `length` is the number of payload bytes written (must be ≤ slot_size).
    pub fn release(self, arena_id: u8, length: usize) -> SharedMemRef {
        assert!(
            length <= self.arena.slot_size,
            "release: length {length} exceeds slot_size {}",
            self.arena.slot_size,
        );

        let digest = if self.arena.integrity_check {
            // SAFETY: slot_ptr returns a valid pointer within the arena mmap.
            // length <= slot_size (checked above). We create a temporary
            // read-only slice for BLAKE3 hashing — the writer (us) holds
            // ACQUIRED so no protocol-compliant peer modifies this region.
            let data = unsafe {
                core::slice::from_raw_parts(
                    self.arena.slot_ptr(self.slot),
                    length,
                )
            };
            Some(blake3::hash(data).into())
        } else {
            None
        };

        // Release fence: all writes to the slot must be visible to
        // the reader before the state transitions to IN_FLIGHT.
        core::sync::atomic::fence(Ordering::Release);

        // CAS: ACQUIRED(N) → IN_FLIGHT(N).
        // The writer holds exclusive ACQUIRED — no other thread can hold
        // ACQUIRED for this slot. The CAS is for spec compliance (all
        // transitions are compare_exchange) and as a debug assertion
        // that the state is still ACQUIRED.
        let expected = pack(self.generation, ACQUIRED);
        let desired = pack(self.generation, IN_FLIGHT);
        let result = self.arena.states()[self.slot as usize]
            .state
            .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire);
        assert!(
            result.is_ok(),
            "SlotGuard::release: CAS ACQUIRED→IN_FLIGHT failed; \
             expected 0x{expected:016x}, found 0x{:016x}. \
             State machine invariant violated — this is a bug.",
            result.unwrap_err(),
        );

        let shmref = SharedMemRef {
            arena_id,
            slot: self.slot,
            generation: self.generation,
            offset: 0,
            length: length as u32,
            digest,
        };

        // Forget self to prevent Drop from rolling back to FREE.
        core::mem::forget(self);

        shmref
    }
}

impl<'a> Drop for SlotGuard<'a> {
    fn drop(&mut self) {
        // Rollback: CAS ACQUIRED(N) → FREE(N).
        // Generation is NOT incremented. No SharedMemRef was sent,
        // therefore no stale reference exists to invalidate.
        // CAS for spec compliance — all transitions are compare_exchange.
        let expected = pack(self.generation, ACQUIRED);
        let desired = pack(self.generation, FREE);
        let result = self.arena.states()[self.slot as usize]
            .state
            .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire);
        if let Err(actual) = result {
            // This should never happen — the writer holds exclusive ACQUIRED.
            // If it does, the slot is leaked until reclaim_all_inflight on disconnect.
            // Log but do not panic — we are in Drop.
            tracing::error!(
                slot = self.slot,
                generation = self.generation,
                expected = format_args!("0x{expected:016x}"),
                actual = format_args!("0x{actual:016x}"),
                "SlotGuard::drop: CAS rollback failed — state corruption, slot leaked"
            );
        }
    }
}
