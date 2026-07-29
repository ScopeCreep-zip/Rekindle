//! Fixture configuration presets for test and bench execution contexts.

use std::time::Duration;

use crate::v4::bulk::counters::BulkCounters;
use crate::v4::config::{ServerConfig, SessionConfig};

/// Timeout for bench operations — generous to survive sustained iteration load.
pub const BENCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for test operations — tight to catch actual failures quickly.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Configuration for an IPC fixture connection.
///
/// Parameterizes the connection lifecycle so tests and benches share
/// the same factory with different tuning. Use `for_test()` or
/// `for_bench()` presets, or override individual fields.
#[derive(Debug, Clone)]
pub struct IpcFixtureConfig {
    pub heartbeat_interval_ms: u64,
    pub heartbeat_miss_limit: u32,
    pub heartbeat_response_timeout_ms: u64,
    pub max_connections: Option<usize>,
    pub worker_threads: usize,
    pub warmup_count: usize,
    pub retention_max_bytes: Option<usize>,
    pub retention_max_frames: Option<usize>,
}

impl IpcFixtureConfig {
    /// Preset for correctness tests: fast heartbeat, tight miss limit,
    /// few connections. Catches failures quickly.
    pub fn for_test() -> Self {
        Self {
            heartbeat_interval_ms: 1_000,
            heartbeat_miss_limit: 3,
            heartbeat_response_timeout_ms: 500,
            max_connections: Some(16),
            worker_threads: 4,
            warmup_count: 0,
            retention_max_bytes: None,
            retention_max_frames: None,
        }
    }

    /// Preset for benchmarks: long heartbeat, high miss tolerance,
    /// many connections, physical-core worker count. Survives sustained
    /// criterion iteration load without false failures.
    pub fn for_bench() -> Self {
        Self {
            heartbeat_interval_ms: 60_000,
            heartbeat_miss_limit: 100,
            heartbeat_response_timeout_ms: 60_000,
            max_connections: Some(65536),
            worker_threads: super::phys_cores(),
            warmup_count: 5,
            retention_max_bytes: None,
            retention_max_frames: None,
        }
    }

    /// Convert to ServerConfig for IpcServer::bind().
    pub fn to_server_config(&self) -> ServerConfig {
        ServerConfig {
            session: self.to_session_config(),
            handshake: super::handshake_config(),
            counters: BulkCounters::new(),
        }
    }

    /// Convert to the transport's SessionConfig.
    pub fn to_session_config(&self) -> SessionConfig {
        let mut c = SessionConfig::default();
        c.heartbeat_interval_ms = self.heartbeat_interval_ms;
        c.heartbeat_miss_limit = self.heartbeat_miss_limit;
        c.heartbeat_response_timeout_ms = self.heartbeat_response_timeout_ms;
        c.max_connections = self.max_connections.map(|n| n as u32);
        if let Some(max_bytes) = self.retention_max_bytes {
            c.retention_config.max_bytes = max_bytes;
        }
        if let Some(max_frames) = self.retention_max_frames {
            c.retention_config.max_frames = max_frames;
        }
        c
    }
}
