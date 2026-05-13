//! Transparent hugepage hints for bulk transport buffer pools.
//!
//! On Linux, requests THP promotion for the process's anonymous memory
//! via `prctl(PR_SET_THP_DISABLE, 0)`. The existing `BufferPool` uses
//! `vec![0u8; SLAB_SIZE]` which pre-faults pages at construction; THP
//! can coalesce these into 2 MiB hugepages automatically when the
//! system policy allows it.
//!
//! This module also provides socket buffer size tuning constants.

/// Default send buffer size for bulk AF_UNIX sockets (2 MiB).
///
/// The kernel doubles the value set by `setsockopt(SO_SNDBUF)`, so
/// setting 2 MiB results in an effective 4 MiB buffer per socket.
/// At 64 KiB chunks, this allows ~57 chunks in flight before
/// backpressure (sender blocks in `sock_alloc_send_pskb`).
///
/// Requires `net.core.wmem_max >= 4194304` in sysctl.
pub const BULK_SOCKET_SNDBUF: usize = 2 * 1024 * 1024;

/// Default receive buffer size for bulk AF_UNIX sockets (2 MiB).
pub const BULK_SOCKET_RCVBUF: usize = 2 * 1024 * 1024;

/// Apply process-level transparent hugepage hints.
///
/// Called once at daemon startup. Ensures THP is not disabled for this
/// process (it may have been disabled by a parent container runtime).
/// The actual hugepage promotion is decided by the kernel based on
/// system THP policy (`/sys/kernel/mm/transparent_hugepage/enabled`).
#[cfg(target_os = "linux")]
pub fn apply_thp_hints() {
    // SAFETY: prctl(PR_SET_THP_DISABLE, 0) is a simple integer flag.
    // Setting to 0 ensures THP is NOT disabled for this process.
    // No pointer arguments, no preconditions, returns -1 on failure.
    #[allow(unsafe_code)]
    unsafe {
        let rc = libc::prctl(libc::PR_SET_THP_DISABLE, 0, 0, 0, 0);
        if rc == 0 {
            tracing::debug!("THP enabled for this process");
        } else {
            tracing::debug!("prctl(PR_SET_THP_DISABLE, 0) returned {rc} — THP policy unchanged");
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply_thp_hints() {
    tracing::debug!("THP hints: not available on this platform");
}

/// Set socket buffer sizes on a raw fd for bulk transport throughput.
///
/// Increases `SO_SNDBUF` and `SO_RCVBUF` to allow more in-flight chunks
/// before backpressure. Silently ignores errors (e.g., if `wmem_max` is
/// too low, the kernel clamps to the maximum).
#[cfg(unix)]
pub fn tune_socket_buffers(raw_fd: std::os::unix::io::RawFd) {
    // SAFETY: setsockopt with SOL_SOCKET + SO_SNDBUF/SO_RCVBUF is safe
    // for any valid socket fd. The value is a c_int pointer to the size.
    #[allow(unsafe_code)]
    unsafe {
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let sndbuf = BULK_SOCKET_SNDBUF as libc::c_int;
        #[allow(clippy::cast_possible_truncation)]
        let socklen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let rc = libc::setsockopt(
            raw_fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
            (&raw const sndbuf).cast::<libc::c_void>(),
            socklen,
        );
        if rc != 0 {
            tracing::debug!(
                requested = BULK_SOCKET_SNDBUF,
                "setsockopt(SO_SNDBUF) failed — kernel clamped to wmem_max. \
                 Set net.core.wmem_max >= {} in sysctl for optimal throughput.",
                BULK_SOCKET_SNDBUF * 2
            );
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let rcvbuf = BULK_SOCKET_RCVBUF as libc::c_int;
        let rc = libc::setsockopt(
            raw_fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
            (&raw const rcvbuf).cast::<libc::c_void>(),
            socklen,
        );
        if rc != 0 {
            tracing::debug!(
                requested = BULK_SOCKET_RCVBUF,
                "setsockopt(SO_RCVBUF) failed — kernel clamped to rmem_max. \
                 Set net.core.rmem_max >= {} in sysctl for optimal throughput.",
                BULK_SOCKET_RCVBUF * 2
            );
        }
    }
}
