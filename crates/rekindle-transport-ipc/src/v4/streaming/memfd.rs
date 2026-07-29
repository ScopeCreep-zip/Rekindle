//! Arena memfd primitives — Linux only.
//!
//! Creates sealed memfds for shared-memory arenas, maps them with THP and
//! pre-fault hints. All functions return `OwnedFd` — no raw fd leaks on
//! error paths.

use std::ffi::CString;
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};

// ── Error types ─────────────────────────────────────────────────

#[derive(Debug)]
pub enum MemfdError {
    InvalidName,
    CreateFailed(io::Error),
    TruncateFailed(io::Error),
    SealFailed(io::Error),
    SealQueryFailed(io::Error),
    InsufficientSeals { expected: i32, actual: i32 },
    MmapFailed(io::Error),
    FstatFailed(io::Error),
    SizeMismatch { expected: u64, actual: u64 },
}

impl std::fmt::Display for MemfdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(f, "memfd name contains null byte"),
            Self::CreateFailed(e) => write!(f, "memfd_create failed: {e}"),
            Self::TruncateFailed(e) => write!(f, "ftruncate failed: {e}"),
            Self::SealFailed(e) => write!(f, "F_ADD_SEALS failed: {e}"),
            Self::SealQueryFailed(e) => write!(f, "F_GET_SEALS failed: {e}"),
            Self::InsufficientSeals { expected, actual } => {
                write!(f, "insufficient seals: expected 0x{expected:x}, got 0x{actual:x}")
            }
            Self::MmapFailed(e) => write!(f, "mmap failed: {e}"),
            Self::FstatFailed(e) => write!(f, "fstat failed: {e}"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "size mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for MemfdError {}

// ── MADV_POPULATE_WRITE ─────────────────────────────────────────
//
// CRITICAL: This is 23, NOT 22.
// 22 is MADV_POPULATE_READ which only read-faults pages.
// Read-faulted pages still incur a write fault on first slot write,
// adding up to 30ms of latency on first frame for a 12 MiB slot.
// Available since Linux 5.14.
const MADV_POPULATE_WRITE: i32 = 23;

// Compile-time check: when the libc crate exports MADV_POPULATE_WRITE,
// this assertion activates and verifies our constant matches.
// Remove the #[cfg(any())] gate when libc adds the constant.
#[cfg(any())]
const _: () = assert!(MADV_POPULATE_WRITE == libc::MADV_POPULATE_WRITE,
    "MADV_POPULATE_WRITE value mismatch with libc");

// ── memfd creation ──────────────────────────────────────────────

/// Create a memfd for use as a persistent shared arena.
///
/// Seals: `F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL`.
/// Deliberately does NOT seal `F_SEAL_WRITE` — both processes must
/// write to their respective slots/states.
///
/// Uses `MFD_NOEXEC_SEAL | MFD_CLOEXEC` for container compatibility
/// (`vm.memfd_noexec=2`). Falls back to `MFD_ALLOW_SEALING | MFD_CLOEXEC`
/// on EINVAL (kernel < 6.1 where `MFD_NOEXEC_SEAL` is unknown).
/// EACCES is a hard error — the container policy forbids it and
/// fallback would violate that policy.
pub fn create_arena_memfd(name: &str, size: u64) -> Result<OwnedFd, MemfdError> {
    let c_name = CString::new(name)
        .map_err(|_| MemfdError::InvalidName)?;

    // Try MFD_NOEXEC_SEAL first (implies MFD_ALLOW_SEALING + clears exec bits)
    let raw = unsafe {
        // SAFETY: memfd_create is a safe Linux syscall with a valid C string.
        libc::memfd_create(
            c_name.as_ptr(),
            libc::MFD_NOEXEC_SEAL | libc::MFD_CLOEXEC,
        )
    };

    let raw = if raw < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINVAL) {
            // Kernel < 6.1: MFD_NOEXEC_SEAL unknown. Fallback.
            let fallback = unsafe {
                // SAFETY: Same syscall, different flags.
                libc::memfd_create(
                    c_name.as_ptr(),
                    libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC,
                )
            };
            if fallback < 0 {
                return Err(MemfdError::CreateFailed(io::Error::last_os_error()));
            }
            tracing::warn!(
                "MFD_NOEXEC_SEAL not supported, fell back to MFD_ALLOW_SEALING"
            );
            fallback
        } else {
            // EACCES (vm.memfd_noexec=2 policy) or other: hard error.
            return Err(MemfdError::CreateFailed(err));
        }
    } else {
        raw
    };

    // OwnedFd takes ownership immediately. All subsequent errors
    // drop OwnedFd, which closes the fd. No leak paths.
    let owned = unsafe {
        // SAFETY: raw is a valid fd returned by memfd_create (checked >= 0).
        OwnedFd::from_raw_fd(raw)
    };

    if size > 0 {
        // SAFETY: owned.as_raw_fd() is a valid memfd. ftruncate sets its size.
        if unsafe { libc::ftruncate(owned.as_raw_fd(), size as libc::off_t) } != 0 {
            return Err(MemfdError::TruncateFailed(io::Error::last_os_error()));
        }
    }

    let seal_flags = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    // Both arena and states memfds receive F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL.
    //
    // This locks out F_SEAL_FUTURE_WRITE permanently. The tradeoff:
    // a process with the fd could create a new writable mapping.
    // This is acceptable because:
    // - The fd is delivered via SCM_RIGHTS over the authenticated Noise IK
    //   control channel. If an attacker has the fd, they're already inside
    //   the trust boundary.
    // - Staged sealing (map → F_SEAL_FUTURE_WRITE → F_SEAL_SEAL) adds
    //   complexity and a timing window for no security gain against the
    //   threat model.
    // SAFETY: owned.as_raw_fd() is a valid memfd created with sealing enabled.
    if unsafe { libc::fcntl(owned.as_raw_fd(), libc::F_ADD_SEALS, seal_flags) } != 0 {
        return Err(MemfdError::SealFailed(io::Error::last_os_error()));
    }

    Ok(owned)
}

// ── mmap ────────────────────────────────────────────────────────

/// mmap a memfd as MAP_SHARED with read+write.
///
/// Applies `MADV_HUGEPAGE` for regions >= 2 MiB (best-effort THP).
/// Applies `MADV_POPULATE_WRITE` (value 23) to pre-fault all pages
/// with write access, eliminating first-frame page fault latency.
///
/// Returns a raw pointer to the mapped region. The caller must
/// call `munmap` with the exact `size` when done.
///
/// # Safety
///
/// `fd` must be a valid open file descriptor suitable for MAP_SHARED
/// mapping (e.g., a memfd). `size` must be > 0 and must not exceed
/// the file's actual size.
pub unsafe fn map_readwrite(fd: RawFd, size: usize) -> Result<*mut u8, MemfdError> {
    assert!(size > 0, "map_readwrite: size must be > 0");

    // SAFETY: mmap with MAP_SHARED on a valid fd. Checked for MAP_FAILED.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED | libc::MAP_POPULATE,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(MemfdError::MmapFailed(io::Error::last_os_error()));
    }

    // Request THP for regions >= 2 MiB. Best-effort — failure is fine.
    // On Android and some container configs, MADV_HUGEPAGE is a no-op.
    if size >= 2 * 1024 * 1024 {
        // SAFETY: ptr is a valid mmap'd region of size bytes.
        unsafe { libc::madvise(ptr, size, libc::MADV_HUGEPAGE) };
    }

    // Pre-fault all pages with WRITE access. Best-effort.
    // MAP_POPULATE may only read-fault shmem pages. MADV_POPULATE_WRITE
    // ensures write faults are resolved, eliminating first-frame latency.
    // SAFETY: ptr is a valid mmap'd region of size bytes.
    let rc = unsafe { libc::madvise(ptr, size, MADV_POPULATE_WRITE) };
    if rc != 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSYS) {
            tracing::debug!(
                "MADV_POPULATE_WRITE not supported, first-frame latency may be elevated"
            );
        } else {
            tracing::warn!(?err, "MADV_POPULATE_WRITE failed");
        }
    }

    // Force THP collapse (Linux 6.1+). More reliable than MADV_HUGEPAGE alone
    // for shmem-backed pages where khugepaged may not collapse proactively.
    // Best-effort — EINVAL on kernels < 6.1 or configs without THP for shmem.
    const MADV_COLLAPSE: i32 = 25;
    if size >= 2 * 1024 * 1024 {
        // SAFETY: ptr is a valid mmap'd region of size bytes.
        let rc = unsafe { libc::madvise(ptr, size, MADV_COLLAPSE) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINVAL)
                && err.raw_os_error() != Some(libc::ENOSYS)
            {
                tracing::debug!(?err, "MADV_COLLAPSE failed (non-fatal)");
            }
        }
    }

    Ok(ptr.cast())
}

// ── Seal verification ───────────────────────────────────────────

/// Verify that a memfd has the required seals before mapping.
///
/// Rejects if `F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL` are not set.
/// Prevents the owner from resizing the arena after the peer has mapped it.
pub fn verify_arena_seals(fd: RawFd) -> Result<(), MemfdError> {
    // SAFETY: fd is a valid file descriptor. F_GET_SEALS returns an int.
    let seals = unsafe { libc::fcntl(fd, libc::F_GET_SEALS) };
    if seals < 0 {
        return Err(MemfdError::SealQueryFailed(io::Error::last_os_error()));
    }

    let required = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    if (seals & required) != required {
        return Err(MemfdError::InsufficientSeals {
            expected: required,
            actual: seals,
        });
    }

    Ok(())
}

/// Verify a memfd's size matches expected, before mapping.
pub fn verify_fd_size(fd: RawFd, expected: u64) -> Result<(), MemfdError> {
    // SAFETY: fstat on a valid fd. stat is zeroed before use.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(MemfdError::FstatFailed(io::Error::last_os_error()));
    }
    if stat.st_size as u64 != expected {
        return Err(MemfdError::SizeMismatch {
            expected,
            actual: stat.st_size as u64,
        });
    }
    Ok(())
}

/// Get the system page size.
pub fn page_size() -> usize {
    // SAFETY: sysconf is always safe to call with _SC_PAGESIZE.
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

/// Round `size` up to the next page boundary.
pub fn page_align(size: usize) -> usize {
    let ps = page_size();
    (size + ps - 1) & !(ps - 1)
}
