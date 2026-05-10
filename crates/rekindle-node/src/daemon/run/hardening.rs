#![allow(unsafe_code)]
//! Process-level security hardening.
//!
//! Applied early in daemon startup before any key material is loaded.
//! Disables core dumps, marks process non-dumpable (prevents ptrace-attach
//! by non-root), and enforces resource limits.

/// Disable core dumps and mark the process as non-dumpable.
///
/// - `PR_SET_DUMPABLE(0)`: prevents ptrace-attach by non-root processes.
///   Also prevents core dumps from containing process memory.
/// - `RLIMIT_CORE(0,0)`: belt-and-suspenders with PR_SET_DUMPABLE.
///   Prevents core files even if dumpable is re-enabled by setuid.
///
/// Logs errors but does not panic — these are defense-in-depth hardening.
/// The daemon still applies Landlock/seccomp even if these calls fail.
#[cfg(target_os = "linux")]
pub fn harden_process() {
    // SAFETY: prctl(PR_SET_DUMPABLE, 0) is a simple integer flag with no
    // pointer arguments, no preconditions, and no UB on failure (returns -1).
    unsafe {
        if libc::prctl(libc::PR_SET_DUMPABLE, 0) != 0 {
            tracing::error!(
                errno = *libc::__errno_location(),
                "prctl(PR_SET_DUMPABLE, 0) failed — process may be ptrace-attachable"
            );
        }
    }

    // SAFETY: setrlimit(RLIMIT_CORE, &rlimit{0,0}) is a simple struct-pointer
    // operation. Stack-allocated rlimit valid for the call duration.
    unsafe {
        let rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &raw const rlim) != 0 {
            tracing::error!(
                errno = *libc::__errno_location(),
                "setrlimit(RLIMIT_CORE, 0) failed — core dumps may still be enabled"
            );
        }
    }

    // Scrub sensitive environment variables after reading them.
    // Any env var that was needed during startup has already been consumed.
    for key in &["REKINDLE_PASSPHRASE", "REKINDLE_MASTER_KEY"] {
        if std::env::var_os(key).is_some() {
            // SAFETY: remove_var is safe but technically not thread-safe
            // per POSIX. We call this during single-threaded init before
            // spawning the tokio runtime.
            unsafe { std::env::remove_var(key); }
            tracing::debug!(key, "scrubbed sensitive environment variable");
        }
    }

    tracing::debug!("process hardening applied: non-dumpable, no core dumps, env scrubbed");
}

#[cfg(not(target_os = "linux"))]
pub fn harden_process() {
    tracing::debug!("process hardening: platform-specific hardening not available");
}

/// Resource limits for the daemon process.
pub struct ResourceLimits {
    /// Maximum open file descriptors (RLIMIT_NOFILE).
    pub nofile: u64,
    /// Maximum locked memory in bytes (RLIMIT_MEMLOCK). 0 = don't set.
    pub memlock_bytes: u64,
}

/// Apply resource limits via setrlimit.
///
/// Called early in daemon startup before sandbox application.
/// Logs warnings on failure — systemd LimitNOFILE= provides a fallback
/// on Linux, and the daemon can still run with defaults.
#[cfg(target_os = "linux")]
pub fn apply_resource_limits(limits: &ResourceLimits) {
    // SAFETY: setrlimit with valid rlimit structs on stack. No UB on failure
    // (returns -1). The rlimit values are validated by the caller.
    unsafe {
        let rlim = libc::rlimit {
            rlim_cur: limits.nofile,
            rlim_max: limits.nofile,
        };
        if libc::setrlimit(libc::RLIMIT_NOFILE, &raw const rlim) != 0 {
            tracing::warn!(
                nofile = limits.nofile,
                errno = *libc::__errno_location(),
                "setrlimit(RLIMIT_NOFILE) failed — using system default"
            );
        } else {
            tracing::info!(nofile = limits.nofile, "RLIMIT_NOFILE applied");
        }

        if limits.memlock_bytes > 0 {
            let rlim = libc::rlimit {
                rlim_cur: limits.memlock_bytes,
                rlim_max: limits.memlock_bytes,
            };
            if libc::setrlimit(libc::RLIMIT_MEMLOCK, &raw const rlim) != 0 {
                tracing::warn!(
                    memlock_bytes = limits.memlock_bytes,
                    errno = *libc::__errno_location(),
                    "setrlimit(RLIMIT_MEMLOCK) failed — using system default"
                );
            } else {
                tracing::info!(memlock_bytes = limits.memlock_bytes, "RLIMIT_MEMLOCK applied");
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply_resource_limits(_limits: &ResourceLimits) {
    tracing::debug!("resource limits: platform-specific limits not available");
}
