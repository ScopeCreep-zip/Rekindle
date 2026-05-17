//! IPC transport configuration.
//!
//! All tunables that affect performance, security, and resource limits.
//! Hot-reloadable fields are documented; non-reloadable fields require restart.

use serde::Deserialize;
use std::time::Duration;

/// IPC transport configuration.
///
/// # Hot-reload safety
///
/// | Field | Reloadable | Reason |
/// |---|---|---|
/// | `max_frame_size` | Yes | Read per frame decode |
/// | `max_connections` (increase) | Yes | Semaphore add_permits |
/// | `max_connections` (decrease) | No | Existing connections retained |
/// | `*_timeout_ms` | Yes | Read per operation |
/// | `rate_limit_*` | Yes | Checked per accept |
/// | `uds_sndbuf/rcvbuf` | No | Applied at accept time |
/// | `listen_backlog` | No | Applied at bind time |
/// | `pool_slab_count` | No | Allocated at startup |
/// | `encrypt_workers` | No | Pool built at startup |
/// | `global_memory_limit` | Yes | Atomic read per reservation |
/// | `heartbeat_*` | No | Applied at connection start |
///
/// # NUMA Considerations
///
/// On multi-socket systems, pin both the server and all clients to the
/// same NUMA node for optimal throughput. The kernel allocates socket
/// buffers (`sk_buff`) on the sender's NUMA node; cross-node reads add
/// ~100ns per cache line miss (20-40% throughput reduction measured).
///
/// Use `taskset` or `sched_setaffinity` to pin sender and receiver
/// threads to the same node. The `start_handler` on the rayon encrypt
/// pool already pins workers via `sched_setaffinity` — ensure the tokio
/// runtime threads are also pinned.
///
/// On single-socket systems (most workstations, laptops, i5-9300H),
/// NUMA is not a concern and no pinning is needed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IpcConfig {
    /// Maximum control-plane frame payload size in bytes.
    /// Checked BEFORE allocation (reject-before-allocate).
    pub max_frame_size: u32,

    /// Hard cap on concurrent connections. Enforced via tokio::sync::Semaphore
    /// in the accept loop. New connections block until a slot opens.
    pub max_connections: u32,

    /// Time between accept and first complete frame.
    pub handshake_timeout_ms: u64,

    /// Idle time between frames after handshake. Connection transitions to
    /// Dead after this duration with no activity. 0 = no idle timeout.
    pub idle_timeout_ms: u64,

    /// Per-connection drain timeout on shutdown. The connection handler
    /// waits this long for in-flight frames to flush before aborting.
    pub drain_timeout_ms: u64,

    /// UDS SO_SNDBUF override (bytes, pre-kernel-doubling).
    /// None = kernel default. TCP: never set (disables autotuning).
    pub uds_sndbuf: Option<u32>,

    /// UDS SO_RCVBUF override (bytes, pre-kernel-doubling).
    pub uds_rcvbuf: Option<u32>,

    /// Token bucket: max requests per second per source identity.
    pub rate_limit_per_peer_per_sec: u32,

    /// Token bucket: max requests per second global.
    pub rate_limit_global_per_sec: u32,

    /// Kernel listen backlog. Capped by net.core.somaxconn (default 4096
    /// since kernel 5.4 per Eric Dumazet's commit).
    pub listen_backlog: u32,

    /// Global memory cap for in-flight bulk data (bytes).
    /// Enforced via GlobalMemoryGuard CAS — never exceeded.
    pub global_memory_limit: u64,

    /// Bulk buffer pool slab count. Each slab ~65.5 KiB.
    /// 256 slabs = ~16.4 MiB pinned. 64x headroom over steady-state.
    pub pool_slab_count: usize,

    /// Rayon encrypt worker count.
    /// 0 = auto-detect (physical_cores - 2, clamped 1..=4).
    pub encrypt_workers: usize,

    /// Heartbeat ping interval in milliseconds. Both client and server
    /// send pings after this duration of inactivity.
    pub heartbeat_interval_ms: u64,

    /// Heartbeat pong timeout in milliseconds. If a pong is not received
    /// within this duration after sending a ping, it counts as a miss.
    pub heartbeat_pong_timeout_ms: u64,

    /// Number of consecutive heartbeat misses before declaring the
    /// connection dead.
    pub heartbeat_max_misses: u32,

    /// Per-connection memory cap for in-flight bulk data (bytes).
    /// A single connection cannot consume more than this, preventing
    /// starvation of other connections under the global limit.
    /// 0 = no per-connection limit (only global limit applies).
    pub per_connection_memory_limit: u64,
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            max_frame_size: crate::frame::codec::MAX_FRAME_SIZE,
            max_connections: 10_000,
            handshake_timeout_ms: 5_000,
            idle_timeout_ms: 60_000,
            drain_timeout_ms: 5_000,
            uds_sndbuf: None,
            uds_rcvbuf: None,
            rate_limit_per_peer_per_sec: 10_000,
            rate_limit_global_per_sec: 100_000,
            listen_backlog: 4096,
            global_memory_limit: 512 * 1024 * 1024, // 512 MiB
            pool_slab_count: 256,
            encrypt_workers: 0,
            heartbeat_interval_ms: 5_000,
            heartbeat_pong_timeout_ms: 3_000,
            heartbeat_max_misses: 3,
            per_connection_memory_limit: 0,
        }
    }
}

impl IpcConfig {
    /// Validate configuration invariants. Call at startup before bind.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_frame_size < 4 {
            return Err("max_frame_size must be >= 4 (length prefix)".into());
        }
        if self.max_frame_size > 64 * 1024 * 1024 {
            return Err("max_frame_size must be <= 64 MiB".into());
        }
        if self.max_connections == 0 {
            return Err("max_connections must be > 0".into());
        }
        if self.listen_backlog == 0 {
            return Err("listen_backlog must be > 0".into());
        }
        if self.pool_slab_count == 0 {
            return Err("pool_slab_count must be > 0".into());
        }
        if self.handshake_timeout_ms == 0 {
            return Err("handshake_timeout_ms must be > 0".into());
        }
        if self.heartbeat_interval_ms == 0 {
            return Err("heartbeat_interval_ms must be > 0".into());
        }
        if self.heartbeat_pong_timeout_ms == 0 {
            return Err("heartbeat_pong_timeout_ms must be > 0".into());
        }
        if self.heartbeat_max_misses == 0 {
            return Err("heartbeat_max_misses must be > 0".into());
        }
        if self.global_memory_limit == 0 {
            return Err("global_memory_limit must be > 0".into());
        }
        Ok(())
    }

    pub fn handshake_timeout(&self) -> Duration {
        Duration::from_millis(self.handshake_timeout_ms)
    }

    pub fn idle_timeout(&self) -> Duration {
        Duration::from_millis(self.idle_timeout_ms)
    }

    pub fn drain_timeout(&self) -> Duration {
        Duration::from_millis(self.drain_timeout_ms)
    }

    pub fn heartbeat_interval(&self) -> Duration {
        Duration::from_millis(self.heartbeat_interval_ms)
    }

    pub fn heartbeat_pong_timeout(&self) -> Duration {
        Duration::from_millis(self.heartbeat_pong_timeout_ms)
    }
}
