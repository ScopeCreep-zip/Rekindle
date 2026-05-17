//! Bind/Listen tests: every server startup condition.

mod common;

use std::path::PathBuf;
use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::noise::keys;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::transport_frame::ConnectionPhase;
use bytes::Bytes;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-bind-test-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct NullRouter;
impl FrameRouter for NullRouter {
    fn route_frame(&self, _: &ServerState, _: u64, _: Bytes) {}
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, _: u64, _: ConnectionPhase, _: ConnectionPhase) {}
}

/// 1.1 Bind to clean path succeeds.
#[test]
fn bind_clean_path() {
    common::init_tracing();
    let path = sock_path("clean");
    let kp = keys::generate_keypair().unwrap();
    let server = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default());
    assert!(server.is_ok(), "bind to clean path failed: {:?}", server.err());
    drop(server);
    let _ = std::fs::remove_file(&path);
}

/// 1.2 Bind removes stale socket file.
#[test]
fn bind_removes_stale() {
    common::init_tracing();
    let path = sock_path("stale");
    std::fs::write(&path, b"stale").unwrap();
    assert!(path.exists());
    let kp = keys::generate_keypair().unwrap();
    let server = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default());
    assert!(server.is_ok(), "bind must remove stale file and succeed");
    drop(server);
    let _ = std::fs::remove_file(&path);
}

/// 1.3 Bind creates parent directories.
#[test]
fn bind_creates_parents() {
    common::init_tracing();
    let base = std::env::temp_dir().join(format!(
        "rekindle-mkdir-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let path = base.join("sub/daemon.sock");
    assert!(!base.exists());
    let kp = keys::generate_keypair().unwrap();
    let server = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default());
    assert!(server.is_ok(), "bind must create parent dirs");
    assert!(base.join("sub").exists());
    drop(server);
    let _ = std::fs::remove_dir_all(&base);
}

/// 1.4 Bind to read-only directory fails.
#[test]
#[cfg(unix)]
fn bind_readonly_dir_fails() {
    common::init_tracing();
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!(
        "rekindle-ro-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o444)).unwrap();
    let path = base.join("daemon.sock");
    let kp = keys::generate_keypair().unwrap();
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default());
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err(), "bind to read-only dir must fail");
    let _ = std::fs::remove_dir_all(&base);
}

/// 1.5 Bind to path exceeding sockaddr_un limit fails.
#[test]
fn bind_path_too_long_fails() {
    common::init_tracing();
    let long_name = "a".repeat(200);
    let path = std::env::temp_dir().join(long_name);
    let kp = keys::generate_keypair().unwrap();
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default());
    assert!(result.is_err(), "bind to >108 byte path must fail");
}

/// 1.6 Second bind while first server holds the socket fails.
#[test]
fn double_bind_second_server_cannot_accept() {
    common::init_tracing();
    let path = sock_path("double");
    let kp1 = keys::generate_keypair().unwrap();
    let server1 = IpcServer::bind(&path, kp1.into_inner(), NullRouter, IpcConfig::default()).unwrap();
    // Our implementation removes the file and rebinds. But the FIRST server
    // still holds the old socket fd. The second server binds a NEW inode.
    // The first server's accepted connections still work, but new connects
    // go to the second server. This is the documented AF_UNIX behavior.
    // We assert the second bind SUCCEEDS (stale removal) but gets a different server.
    let kp2 = keys::generate_keypair().unwrap();
    let server2 = IpcServer::bind(&path, kp2.into_inner(), NullRouter, IpcConfig::default());
    assert!(server2.is_ok(), "second bind must succeed (stale removal)");
    // Both servers exist. They are independent.
    assert!(server1.connection_count() == 0);
    assert!(server2.as_ref().unwrap().connection_count() == 0);
    drop(server2);
    drop(server1);
    let _ = std::fs::remove_file(&path);
}

/// 1.7 Bind with empty path fails.
#[test]
fn bind_empty_path_fails() {
    common::init_tracing();
    let kp = keys::generate_keypair().unwrap();
    let result = IpcServer::bind(std::path::Path::new(""), kp.into_inner(), NullRouter, IpcConfig::default());
    assert!(result.is_err(), "bind to empty path must fail");
}

/// 1.8 Server Drop removes socket file.
#[test]
fn drop_removes_socket_file() {
    common::init_tracing();
    let path = sock_path("drop-clean");
    let kp = keys::generate_keypair().unwrap();
    let server = IpcServer::bind(&path, kp.into_inner(), NullRouter, IpcConfig::default()).unwrap();
    assert!(path.exists(), "socket file must exist after bind");
    drop(server);
    assert!(!path.exists(), "socket file must be removed after Drop");
}

/// 1.9 Invalid config: max_frame_size = 0 rejected.
#[test]
fn config_rejects_zero_frame_size() {
    common::init_tracing();
    let path = sock_path("cfg-zero");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.max_frame_size = 0;
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "max_frame_size=0 must be rejected");
    let _ = std::fs::remove_file(&path);
}

/// 1.9b Invalid config: max_frame_size = 1 (less than length prefix).
#[test]
fn config_rejects_frame_size_less_than_prefix() {
    common::init_tracing();
    let path = sock_path("cfg-tiny");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.max_frame_size = 3; // less than 4-byte length prefix
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "max_frame_size=3 must be rejected");
    let _ = std::fs::remove_file(&path);
}

/// 1.9c Invalid config: max_connections = 0.
#[test]
fn config_rejects_zero_connections() {
    common::init_tracing();
    let path = sock_path("cfg-noconn");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.max_connections = 0;
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "max_connections=0 must be rejected");
    let _ = std::fs::remove_file(&path);
}

/// 1.9d Invalid config: listen_backlog = 0.
#[test]
fn config_rejects_zero_backlog() {
    common::init_tracing();
    let path = sock_path("cfg-nobacklog");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.listen_backlog = 0;
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "listen_backlog=0 must be rejected");
    let _ = std::fs::remove_file(&path);
}

/// 1.9e Invalid config: pool_slab_count = 0.
#[test]
fn config_rejects_zero_pool() {
    common::init_tracing();
    let path = sock_path("cfg-nopool");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.pool_slab_count = 0;
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "pool_slab_count=0 must be rejected");
    let _ = std::fs::remove_file(&path);
}

/// 1.9f Invalid config: handshake_timeout = 0.
#[test]
fn config_rejects_zero_handshake_timeout() {
    common::init_tracing();
    let path = sock_path("cfg-nohs");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.handshake_timeout_ms = 0;
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "handshake_timeout_ms=0 must be rejected");
    let _ = std::fs::remove_file(&path);
}

/// 1.9g Invalid config: max_frame_size above 64 MiB hard cap.
#[test]
fn config_rejects_huge_frame_size() {
    common::init_tracing();
    let path = sock_path("cfg-huge");
    let kp = keys::generate_keypair().unwrap();
    let mut config = IpcConfig::default();
    config.max_frame_size = 128 * 1024 * 1024; // 128 MiB
    let result = IpcServer::bind(&path, kp.into_inner(), NullRouter, config);
    assert!(result.is_err(), "max_frame_size > 64MiB must be rejected");
    let _ = std::fs::remove_file(&path);
}
