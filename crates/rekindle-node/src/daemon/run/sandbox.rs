#![allow(unsafe_code)]
//! Landlock + seccomp sandbox for the rekindle daemon (Linux only).
//!
//! Applied AFTER socket bind and keypair read, BEFORE processing any
//! IPC traffic or network operations. Restricts filesystem access to
//! only the paths the daemon needs at runtime.
//!
//! After sandbox application, the daemon cannot:
//! - Read arbitrary files (config injection attacks blocked)
//! - Write outside its state/runtime directories (data exfiltration blocked)
//! - Execute new binaries (code execution attacks blocked)
//! - Access syscalls outside the allowlist (exploitation surface minimized)
//!
//! Panics on failure — the daemon refuses to run unsandboxed.

use std::path::Path;

/// Apply the full sandbox stack: process hardening + Landlock + seccomp.
///
/// Called in `run_daemon()` step 7, after the IPC socket is bound and the
/// bus keypair is loaded. All filesystem paths needed at runtime must be
/// enumerated here — anything not listed is blocked by Landlock.
///
/// # Panics
///
/// Panics if sandbox application fails on Linux. Non-Linux platforms log
/// a warning and continue (no Landlock/seccomp equivalent available).
#[cfg(target_os = "linux")]
pub fn apply(
    paths: &crate::state::StatePaths,
    socket_path: &Path,
    config: &rekindle_types::config::TransportConfig,
) {
    use std::path::PathBuf;

    use landlock::{
        ABI, Access, AccessFs, AccessNet, NetPort, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetStatus, Scope,
    };

    // Check for development bypass (NEVER set in production).
    if std::env::var("REKINDLE_DISABLE_SANDBOX").is_ok() {
        tracing::warn!(
            "REKINDLE_DISABLE_SANDBOX is set — sandbox BYPASSED. \
             Acceptable for development only. Never set in production."
        );
        return;
    }

    // Process hardening (idempotent — safe to call even if already called).
    super::hardening::harden_process();

    let runtime_dir = socket_path
        .parent()
        .unwrap_or(Path::new("/tmp"))
        .to_path_buf();

    // ── Landlock filesystem rules ───────────────────────────────────

    let abi = ABI::V6;

    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .expect("landlock handle_access(fs)")
        .handle_access(AccessNet::from_all(abi))
        .expect("landlock handle_access(net)")
        .scope(Scope::from_all(abi))
        .expect("landlock scope")
        .create()
        .expect("landlock create ruleset");

    let access_file = AccessFs::from_file(abi);

    let rules: Vec<(PathBuf, _)> = {
        let mut r = vec![
            // State directory: vault.db, session.json, audit.jsonl, vault.salt, vault.wrapped
            (paths.state_dir.clone(), AccessFs::from_all(abi)),
            // Config directories: system (/etc/rekindle/) and user (~/.config/rekindle/)
            (PathBuf::from("/etc/rekindle"), AccessFs::from_read(abi)),
            (paths.config_dir.clone(), AccessFs::from_read(abi)),
            // Veilid storage: DHT records, routing table, block store
            (paths.veilid_dir.clone(), AccessFs::from_all(abi)),
            // Log directory: rotating log files
            (paths.log_dir.clone(), AccessFs::from_all(abi)),
            // Runtime directory: IPC socket, bus keypair, per-agent keys
            (runtime_dir.clone(), AccessFs::from_all(abi)),
            // /proc/self: specific files enumerated to DENY /proc/self/mem.
            //
            // /proc/self/mem allows arbitrary process memory reads, defeating
            // all plaintext zeroization. By listing only the files our process
            // and dependencies actually read, Landlock denies openat() on
            // /proc/self/mem for any code running inside this process.
            //
            // Empirically verified against source of: tokio, veilid, aws-lc,
            // aws-lc-rs, snow, rustix, rayon, crossbeam, rusqlite, io-uring,
            // socket2, rust-landlock. None read /proc/self/mem.
            //
            // rustix linux_raw backend: /proc/self/auxv (kernel params),
            //   /proc/self/fd/<N> (terminal ioctl)
            // rekindle health.rs: /proc/self/status, /proc/self/stat,
            //   /proc/self/statm
            // tokio: /proc/self/cgroup (available_parallelism)
            // container detection: /proc/self/mountinfo
            // resource monitoring: /proc/self/limits
            (PathBuf::from("/proc/self/status"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/stat"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/statm"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/cgroup"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/mountinfo"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/limits"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/auxv"), AccessFs::from_read(abi)),
            (PathBuf::from("/proc/self/fd"), AccessFs::from_read(abi)),
            // DNS resolution (Veilid bootstrap)
            (PathBuf::from("/etc/resolv.conf"), AccessFs::from_read(abi)),
            // SSL/TLS certificates (Veilid HTTPS bootstrap)
            (PathBuf::from("/etc/ssl"), AccessFs::from_read(abi)),
            (PathBuf::from("/etc/pki"), AccessFs::from_read(abi)),
        ];

        // systemd notify socket (if not abstract).
        if let Ok(notify_socket) = std::env::var("NOTIFY_SOCKET") {
            if !notify_socket.starts_with('@') {
                let path = PathBuf::from(&notify_socket);
                if path.exists() {
                    r.push((path, AccessFs::from_file(abi)));
                }
            }
        }

        r
    };

    for (path, access) in &rules {
        if !path.exists() {
            tracing::info!(
                path = %path.display(),
                "landlock: skipping rule for non-existent path"
            );
            continue;
        }

        let path_fd = match PathFd::new(path) {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "landlock: PathFd::new failed — skipping rule"
                );
                continue;
            }
        };

        // fstat the already-open fd to avoid TOCTOU. Strip directory-only
        // flags on non-directory inodes to prevent PartiallyEnforced.
        let final_access = {
            use std::os::unix::io::{AsFd, AsRawFd};
            // SAFETY: zeroed() produces a valid libc::stat with all fields zero-initialized.
            let mut stat: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: fstat on a valid fd writes into the stat struct. The fd is valid
            // because PathFd::new succeeded above. The pointer is valid stack memory.
            let rc = unsafe { libc::fstat(path_fd.as_fd().as_raw_fd(), &raw mut stat) };
            if rc == 0 && (stat.st_mode & libc::S_IFMT) != libc::S_IFDIR {
                *access & access_file
            } else {
                *access
            }
        };

        ruleset = ruleset
            .add_rule(PathBeneath::new(path_fd, final_access))
            .unwrap_or_else(|e| {
                panic!(
                    "landlock add_rule({}) failed: {e}",
                    path.display()
                );
            });
    }

    // ── Landlock network rules ──────────────────────────────────────
    // Landlock NetPort rules are exact-port matches — port=0 is NOT a
    // wildcard for either BindTcp or ConnectTcp. The kernel looks up rules
    // by the actual port from the socket address (net.c:187).
    // Every port the daemon binds or connects to must be listed explicitly.

    // Parse the port from a Veilid listen_address string (e.g. ":40000",
    // "0.0.0.0:40000"). Returns None if empty or unparseable.
    fn parse_listen_port(addr: &str) -> Option<u16> {
        if addr.is_empty() {
            return None;
        }
        addr.rsplit(':').next()?.parse::<u16>().ok()
    }

    let veilid_tcp_port = parse_listen_port(&config.veilid.tcp_listen_address).unwrap_or(5150);

    // ── BindTcp: ports the daemon listens on ───────────────────────
    let mut bind_ports: Vec<u16> = Vec::new();

    // Veilid TCP listener.
    bind_ports.push(veilid_tcp_port);

    // Veilid WebSocket listener (if distinct from TCP port).
    if let Some(ws_port) = parse_listen_port(&config.veilid.ws_listen_address) {
        if !bind_ports.contains(&ws_port) {
            bind_ports.push(ws_port);
        }
    }

    // Metrics and health endpoints.
    for port in [config.metrics_port, config.health_port] {
        if port > 0 && !bind_ports.contains(&port) {
            bind_ports.push(port);
        }
    }

    for port in &bind_ports {
        ruleset = ruleset
            .add_rule(NetPort::new(*port, AccessNet::BindTcp))
            .unwrap_or_else(|e| panic!("landlock add_rule(BindTcp port={port}) failed: {e}"));
    }

    // ── ConnectTcp: ports the daemon connects to outbound ──────────
    // Veilid bootstrap peers use 5150 (default), 80, and 443.
    // The configured TCP port is included for peer-to-peer connections.
    let mut connect_ports: Vec<u16> = vec![5150, 80, 443];
    if !connect_ports.contains(&veilid_tcp_port) {
        connect_ports.push(veilid_tcp_port);
    }

    for port in &connect_ports {
        ruleset = ruleset
            .add_rule(NetPort::new(*port, AccessNet::ConnectTcp))
            .unwrap_or_else(|e| panic!("landlock add_rule(ConnectTcp port={port}) failed: {e}"));
    }

    let status = ruleset
        .restrict_self()
        .unwrap_or_else(|e| panic!("landlock restrict_self failed: {e}"));

    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            tracing::info!(rules = rules.len(), "landlock: fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            panic!(
                "landlock PARTIALLY ENFORCED — kernel ABI too old. \
                 Requires Linux 5.13+ with Landlock V1+. \
                 The daemon refuses to run with partial enforcement."
            );
        }
        RulesetStatus::NotEnforced => {
            panic!(
                "landlock NOT ENFORCED — kernel does not support Landlock. \
                 Requires Linux 5.13+. The daemon refuses to run without \
                 filesystem sandboxing."
            );
        }
    }

    // ── seccomp syscall filter ──────────────────────────────────────

    install_sigsys_handler();

    let allowed_syscalls = rekindle_daemon_syscalls();

    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    // Default action: return EPERM instead of KillThread. This lets the
    // application handle denied syscalls gracefully (Veilid logs errors,
    // retries with fallback paths) instead of silently killing threads.
    // KillThread causes zombie processes with no diagnostics. EPERM surfaces
    // the failure in application logs where it can be diagnosed and the
    // allowlist updated.
    let mut filter = ScmpFilterContext::new_filter(ScmpAction::Errno(libc::EPERM))
        .expect("seccomp new_filter");

    for name in &allowed_syscalls {
        let syscall = ScmpSyscall::from_name(name)
            .unwrap_or_else(|e| panic!("seccomp unknown syscall '{name}': {e}"));
        filter
            .add_rule(ScmpAction::Allow, syscall)
            .unwrap_or_else(|e| panic!("seccomp add_rule({name}) failed: {e}"));
    }

    filter.load().unwrap_or_else(|e| panic!("seccomp load failed: {e}"));

    tracing::info!(
        allowed_syscalls = allowed_syscalls.len(),
        default_action = "Errno(EPERM)",
        "seccomp: filter loaded"
    );
}

#[cfg(not(target_os = "linux"))]
pub fn apply(
    _paths: &crate::state::StatePaths,
    _socket_path: &Path,
    _config: &rekindle_types::config::TransportConfig,
) {
    tracing::warn!(
        "sandbox: not available on this platform. \
         Linux 5.13+ required for Landlock filesystem sandboxing. \
         Linux 3.17+ required for seccomp-bpf syscall filtering."
    );
}

/// Install a SIGSYS signal handler that logs the blocked syscall number
/// before the process terminates. seccomp KillThread sends SIGSYS to the
/// offending thread with si_syscall set to the syscall number. This handler
/// writes the number to stderr (async-signal-safe) before re-raising.
#[cfg(target_os = "linux")]
fn install_sigsys_handler() {
    extern "C" fn handler(
        _sig: libc::c_int,
        info: *mut libc::siginfo_t,
        _ctx: *mut libc::c_void,
    ) {
        // SAFETY: signal handler — async-signal-safe calls only.
        // No allocator, no locks, no tracing. Write directly to stderr fd 2.
        let syscall_nr: i32 = unsafe {
            if info.is_null() {
                -1
            } else {
                // si_syscall offset on Linux x86_64:
                // siginfo_t base fields (16 bytes) + _sigsys.si_call_addr (8 bytes)
                // = 24 bytes from struct start.
                let base = info.cast::<u8>();
                base.add(24).cast::<libc::c_int>().read_unaligned()
            }
        };

        let mut buf = *b"SECCOMP VIOLATION: syscall=          \n";
        let mut n = u64::from(syscall_nr.unsigned_abs());
        let mut pos = buf.len() - 2;
        if n == 0 {
            buf[pos] = b'0';
        } else {
            while n > 0 && pos > 27 {
                buf[pos] = b'0' + (n % 10) as u8;
                n /= 10;
                pos -= 1;
            }
        }
        // SAFETY: write(2, ...) and signal/raise are async-signal-safe per POSIX.
        // No allocations, no locks. This is the last thing the thread does before
        // re-raising SIGSYS with the default handler (which terminates the process).
        unsafe {
            let _ = libc::write(2, buf.as_ptr().cast(), buf.len());
            libc::signal(libc::SIGSYS, libc::SIG_DFL);
            libc::raise(libc::SIGSYS);
        }
    }

    // SAFETY: sigaction installs a signal handler. The handler function pointer
    // is valid for the process lifetime (static extern "C" fn). SA_RESETHAND
    // ensures one-shot — the handler runs once then reverts to default.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
        libc::sigemptyset(&raw mut sa.sa_mask);
        libc::sigaction(libc::SIGSYS, &raw const sa, std::ptr::null_mut());
    }
}

/// Syscall allowlist for the rekindle daemon.
///
/// Includes all syscalls required by: tokio async runtime, Veilid UDP/TCP
/// network transport, SQLCipher (via rusqlite), IPC Unix socket, inotify
/// config watch, getrandom for crypto, mlock for secret material, systemd
/// sd_notify, Prometheus HTTP metrics endpoint.
///
/// Adding a syscall: if seccomp kills the daemon with "SECCOMP VIOLATION:
/// syscall=NNN", look up the name via `ausyscall NNN` and add it here
/// after verifying the daemon legitimately needs it.
#[cfg(target_os = "linux")]
fn rekindle_daemon_syscalls() -> Vec<&'static str> {
    vec![
        // I/O fundamentals
        "read", "write", "close", "openat", "lseek", "pread64", "pwrite64",
        "fstat", "stat", "newfstatat", "statx", "access", "unlink",
        "fcntl", "flock", "ftruncate", "fallocate", "fsync", "fdatasync",
        "mkdir", "getdents64", "rename", "readlink", "readlinkat",
        // Memory management (mlock/munlock for secret material)
        "mmap", "mprotect", "munmap", "mlock", "munlock", "madvise", "brk",
        // Process / threading (tokio multi-threaded runtime)
        "futex", "clone3", "clone", "set_robust_list", "set_tid_address",
        "rseq", "sched_getaffinity", "prlimit64", "prctl",
        "getpid", "gettid", "getuid", "geteuid", "getgid", "getegid",
        "getresuid", "getresgid",
        // Epoll / event loop (tokio reactor)
        "epoll_wait", "epoll_ctl", "epoll_create1", "eventfd2",
        "poll", "ppoll",
        // Timers (tokio time driver)
        "clock_gettime", "timer_create", "timer_settime", "timer_delete",
        "nanosleep", "clock_nanosleep",
        // Networking: IPC Unix socket + Veilid UDP/TCP + metrics HTTP + health TCP
        "socket", "connect", "bind", "listen", "accept4", "accept",
        "sendto", "recvfrom", "sendmsg", "recvmsg", "shutdown",
        "getsockopt", "setsockopt", "getsockname", "getpeername",
        "socketpair",
        // File descriptor manipulation
        "dup", "dup2", "pipe2", "ioctl",
        // Vectored I/O (tokio + rusqlite)
        "writev", "readv",
        // Signals (tokio signal handling + seccomp SIGSYS)
        "sigaltstack", "rt_sigaction", "rt_sigprocmask", "rt_sigreturn",
        "tgkill", "kill",
        // Config hot-reload (notify crate → inotify)
        "inotify_init1", "inotify_add_watch", "inotify_rm_watch",
        // Cryptographic random (aws-lc-rs + getrandom)
        "getrandom",
        // Memory-mapped secrets and anonymous file descriptors
        "memfd_secret", "memfd_create",
        // Process lifecycle
        "exit_group", "exit",
        // Filesystem metadata (Veilid sets permissions on storage files)
        "chmod", "fchmod", "fchmodat", "chown", "fchown", "fchownat",
        "utimensat", "linkat", "symlinkat", "rmdir", "unlinkat",
        "renameat", "renameat2",
        // Veilid networking: additional socket/IO ops for UDP hole-punching,
        // TCP relay, WS transport, and route allocation
        "sendmmsg", "recvmmsg", "select", "pselect6",
        // Thread synchronization (Veilid uses condvars, mutexes, barriers)
        "futex_waitv", "sched_yield", "sched_setaffinity",
        // Process introspection (Veilid + aws-lc-rs thread count detection)
        "sysinfo", "getrlimit", "setrlimit",
        // Required by Rust stdlib / tokio / veilid
        "restart_syscall", "uname", "getcwd",
        // mmap variants (Veilid LMDB-like storage, aws-lc-rs)
        "mremap", "msync", "mincore",
        // Filesystem info (Veilid checks disk space during bootstrap)
        "statfs", "fstatfs",
        // Modern kernel APIs (tokio 1.x + Veilid on Linux 5.10+)
        "mlock2", "pkey_mprotect", "close_range", "openat2", "epoll_pwait2",
    ]
}
