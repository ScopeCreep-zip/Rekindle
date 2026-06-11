use std::sync::Arc;
use std::sync::atomic::Ordering;

use rekindle_transport_ipc::v3::bulk::counters::BulkCounters;
use rekindle_transport_ipc::v3::context::ServerConfig;

#[test]
fn new_produces_valid_defaults() {
    let config = ServerConfig::new();

    assert!(
        config.session.heartbeat_interval_ms > 0,
        "heartbeat_interval_ms must be positive, got {}",
        config.session.heartbeat_interval_ms,
    );
    assert!(
        config.session.heartbeat_miss_limit > 0,
        "heartbeat_miss_limit must be positive, got {}",
        config.session.heartbeat_miss_limit,
    );
    assert!(
        config.session.heartbeat_response_timeout_ms > 0,
        "heartbeat_response_timeout_ms must be positive, got {}",
        config.session.heartbeat_response_timeout_ms,
    );
    assert!(
        config.session.max_pending_bytes_per_session > 0,
        "max_pending_bytes_per_session must be positive, got {}",
        config.session.max_pending_bytes_per_session,
    );
    assert!(
        !config.handshake.capabilities.is_empty(),
        "handshake capabilities must not be empty — MANDATORY_V1 required at minimum",
    );
    assert_eq!(
        config.counters.frames_sent.load(Ordering::Relaxed), 0,
        "fresh counters must start at zero",
    );
}

#[test]
fn struct_update_preserves_injected_counters() {
    let custom_counters = BulkCounters::new();
    let config = ServerConfig {
        counters: Arc::clone(&custom_counters),
        ..ServerConfig::new()
    };

    assert!(
        Arc::ptr_eq(&config.counters, &custom_counters),
        "struct update must preserve the exact Arc pointer for counters — \
         got different Arc (server would use different counters than caller)",
    );
    assert!(
        config.session.heartbeat_interval_ms > 0,
        "non-overridden session defaults must survive struct update, got heartbeat_interval_ms={}",
        config.session.heartbeat_interval_ms,
    );
}

#[test]
fn injected_counters_share_single_allocation() {
    let counters = BulkCounters::new();
    let server_side = Arc::clone(&counters);

    server_side.frames_sent.fetch_add(42, Ordering::Relaxed);
    assert_eq!(
        counters.frames_sent.load(Ordering::Relaxed), 42,
        "write through server Arc must be visible through caller Arc — \
         got {}, indicating separate allocations",
        counters.frames_sent.load(Ordering::Relaxed),
    );
}

#[test]
fn for_test_config() {
    let config = ServerConfig::for_test();

    assert!(
        config.session.heartbeat_interval_ms <= 2_000,
        "for_test() heartbeat must be <= 2s for fast detection, got {}ms",
        config.session.heartbeat_interval_ms,
    );
    assert!(
        config.session.heartbeat_miss_limit <= 5,
        "for_test() miss limit must be <= 5, got {}",
        config.session.heartbeat_miss_limit,
    );
    assert!(
        config.session.heartbeat_response_timeout_ms <= 2_000,
        "for_test() response timeout must be <= 2s, got {}ms",
        config.session.heartbeat_response_timeout_ms,
    );
}

#[test]
fn for_bench_config() {
    let config = ServerConfig::for_bench();

    assert!(
        config.session.heartbeat_interval_ms >= 30_000,
        "for_bench() heartbeat must be >= 30s for sustained criterion load, got {}ms",
        config.session.heartbeat_interval_ms,
    );
    assert!(
        config.session.heartbeat_miss_limit >= 50,
        "for_bench() miss limit must be >= 50, got {}",
        config.session.heartbeat_miss_limit,
    );
}
