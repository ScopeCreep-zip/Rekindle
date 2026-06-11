//! SSOT test/bench infrastructure for IPC transport.
//!
//! Provides `IpcFixture` — a single type that owns the complete lifecycle
//! of a connected server+client pair. Used by both `#[tokio::test]` tests
//! and criterion benchmarks. No connection setup code exists outside this
//! module.

mod config;

pub use config::{IpcFixtureConfig, BENCH_TIMEOUT, TEST_TIMEOUT};

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::v3::client::{IpcClient, ReplyPayload, RequestReplyError};
use crate::v3::crypto::noise::generate_keypair;
use crate::v3::router::{MockRouter, ReplyRouter};
use crate::v3::server::{ConnectionHandle, IpcServer};
use crate::v3::session::handshake::HandshakeConfig;
use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;

/// A connected server+client pair with full lifecycle ownership.
///
/// All `IpcClient` methods are delegated through `IpcFixture` — no
/// direct field access to `client`. Tests call `f.send_request(...)`,
/// benches call `f.block_on(f.send_request(...))`.
///
/// For graceful shutdown (GOODBYE frame), use `f.take_client().shutdown().await`.
/// For abrupt cleanup, just drop the fixture — `Drop` cancels the server.
pub struct IpcFixture {
    /// The connected client. Private — accessed through delegation methods.
    /// Option to allow take_client() for graceful shutdown tests.
    client: Option<IpcClient>,
    /// Shared MockRouter — captures every delivery across all connections.
    pub router: Arc<MockRouter>,
    /// First connection's handle — exposes outbound_tx and bulk_sender.
    pub conn_handle: Arc<Mutex<Option<ConnectionHandle>>>,
    /// Server cancellation token.
    pub cancel: CancellationToken,
    /// Owned tokio runtime (Some for bench, None for test).
    runtime: Option<tokio::runtime::Runtime>,
    /// Server task handle.
    _task: tokio::task::JoinHandle<()>,
    /// Temp directory holding the Unix socket.
    _tempdir: tempfile::TempDir,
    /// Server public key (for additional client connections).
    server_pub: [u8; 32],
    /// Socket path (for additional client connections).
    socket_path: std::path::PathBuf,
    /// Config used (for additional client connections).
    config: IpcFixtureConfig,
}

impl IpcFixture {
    /// Connect a server+client pair asynchronously.
    /// For use inside `#[tokio::test]` where the caller owns the runtime.
    pub async fn connect(config: IpcFixtureConfig) -> Self {
        let tempdir = tempfile::tempdir().expect("tempdir creation failed");
        let socket_path = tempdir.path().join("fixture.sock");
        let server_keypair = generate_keypair().expect("server keypair failed");
        let client_keypair = generate_keypair().expect("client keypair failed");
        let server_pub: [u8; 32] = server_keypair.public.clone().try_into().expect("pubkey 32 bytes");

        let server_config = config.to_server_config();
        let session_config = server_config.session.clone();
        let hs_config = server_config.handshake.clone();

        let shared_router = MockRouter::new();
        let shared_router_ref = Arc::clone(&shared_router);
        let first_conn_handle: Arc<Mutex<Option<ConnectionHandle>>> =
            Arc::new(Mutex::new(None));
        let first_conn_ref = Arc::clone(&first_conn_handle);

        let server = IpcServer::bind(
            &socket_path,
            server_keypair,
            move |conn_handle: ConnectionHandle| {
                let mut slot = first_conn_ref.lock();
                if slot.is_none() {
                    *slot = Some(conn_handle.clone());
                }
                drop(slot);
                ReplyRouter {
                    router: Arc::clone(&shared_router_ref),
                    conn_handle,
                }
            },
            server_config,
        ).await.expect("server bind failed");

        let cancel = server.cancel_token().clone();
        let task = tokio::spawn(async move {
            if let Err(e) = server.run().await {
                panic!("server accept loop failed: {e:?}");
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let client = IpcClient::connect(
            &socket_path, &server_pub, &client_keypair,
            session_config, hs_config,
        ).await.expect("client connect failed");

        // Warmup
        for _ in 0..config.warmup_count {
            client.send_request(b"warmup", BENCH_TIMEOUT).await
                .expect("warmup request failed");
        }

        Self {
            client: Some(client),
            router: shared_router,
            conn_handle: first_conn_handle,
            cancel,
            runtime: None,
            _task: task,
            _tempdir: tempdir,
            server_pub,
            socket_path: socket_path.to_path_buf(),
            config,
        }
    }

    /// Connect a server+client pair synchronously with an owned runtime.
    /// For use in criterion bench closures where `b.iter()` needs `block_on`.
    pub fn connect_blocking(config: IpcFixtureConfig) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(config.worker_threads)
            .enable_all()
            .build()
            .expect("runtime build failed");

        let mut fixture = rt.block_on(Self::connect(config));
        fixture.runtime = Some(rt);
        fixture
    }

    /// Run an async future on the fixture's owned runtime.
    /// Panics if the fixture was created with `connect()` (no owned runtime).
    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.as_ref()
            .expect("block_on requires connect_blocking (owned runtime)")
            .block_on(f)
    }

    /// Connect an additional client to the same server.
    /// For multi-client tests (lifecycle, adversarial, handshake).
    pub async fn connect_additional_client(&self) -> IpcClient {
        let ckp = generate_keypair().expect("client keypair failed");
        let session_config = self.config.to_session_config();
        IpcClient::connect(
            &self.socket_path, &self.server_pub, &ckp,
            session_config, handshake_config(),
        ).await.expect("additional client connect failed")
    }

    /// Bind a server without connecting a client.
    /// For tests that need manual client connection (wrong-key, max-connections).
    pub async fn bind_server(
        socket_path: &Path,
        keypair: snow::Keypair,
        config: IpcFixtureConfig,
    ) -> BoundServer {
        let server_config = config.to_server_config();

        let shared_router = MockRouter::new();
        let shared_router_ref = Arc::clone(&shared_router);

        let server = IpcServer::bind(
            socket_path,
            keypair,
            move |conn_handle: ConnectionHandle| {
                ReplyRouter {
                    router: Arc::clone(&shared_router_ref),
                    conn_handle,
                }
            },
            server_config,
        ).await.expect("server bind failed");

        let cancel = server.cancel_token().clone();
        let task = tokio::spawn(async move {
            if let Err(e) = server.run().await {
                panic!("server accept loop failed: {e:?}");
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        BoundServer { router: shared_router, cancel, _task: task }
    }

    /// Access the first connection's BulkSender (for server→client bench).
    pub fn bulk_sender(&self) -> crate::v3::bulk::send::BulkSender {
        self.conn_handle.lock().as_ref()
            .expect("conn_handle not populated — connect a client first")
            .bulk_sender.clone()
    }

    /// Access the first connection's outbound_tx (for server→client datagram bench).
    pub fn outbound_tx(&self) -> tokio::sync::mpsc::Sender<crate::v3::context::OutboundFrame> {
        self.conn_handle.lock().as_ref()
            .expect("conn_handle not populated — connect a client first")
            .outbound_tx.clone()
    }

    /// Internal accessor — panics if client was taken.
    fn client(&self) -> &IpcClient {
        self.client.as_ref().expect("client already taken via take_client()")
    }

    /// Take ownership of the client for graceful shutdown (GOODBYE frame).
    /// The fixture continues to own the server — Drop still cancels it.
    /// Panics if called twice.
    pub fn take_client(&mut self) -> IpcClient {
        self.client.take().expect("client already taken via take_client()")
    }

    // ── IpcClient delegation ─────────────────────────────────────

    pub fn session_id(&self) -> uuid::Uuid { self.client().session_id() }
    pub fn agreed_clearance(&self) -> Clearance { self.client().agreed_clearance() }
    pub fn active_capabilities(&self) -> CapabilityBits { self.client().active_capabilities() }
    pub fn phase(&self) -> crate::v3::client::ClientPhase { self.client().phase() }

    pub async fn send_request(&self, payload: &[u8], ack_timeout: Duration) -> Result<crate::v3::client::SendDelivered, crate::v3::client::SendError> {
        self.client().send_request(payload, ack_timeout).await
    }

    pub async fn request_reply(&self, payload: &[u8], timeout: Duration) -> Result<ReplyPayload, RequestReplyError> {
        self.client().request_reply(payload, timeout).await
    }

    pub async fn send_notify(&self, payload: &[u8]) -> Result<(), crate::v3::client::ClientError> {
        self.client().send_notify(payload).await
    }

    pub async fn send_bulk(&self, stream_id: u8, payload: &[u8], ack_timeout: Duration) -> Result<crate::v3::client::BulkDelivered, crate::v3::client::BulkError> {
        self.client().send_bulk(stream_id, payload, ack_timeout).await
    }

    pub async fn rotate_keys(&self, timeout: Duration) -> Result<(), crate::v3::client::BulkError> {
        self.client().rotate_keys(timeout).await
    }

    pub async fn cancel_bulk(&self, stream_id: u8) {
        self.client().cancel_bulk(stream_id).await;
    }

    pub async fn recv(&self) -> Option<crate::v3::client::InboundFrame> {
        self.client().recv().await
    }

    pub async fn recv_bulk_chunk(&self) -> Option<crate::v3::client::BulkChunk> {
        self.client().recv_bulk_chunk().await
    }

    pub async fn recv_bulk(&self) -> Option<(u8, Vec<u8>)> {
        self.client().recv_bulk().await
    }

    pub fn cancel_recv_bulk(&self, stream_id: u8) {
        self.client().cancel_recv_bulk(stream_id);
    }

    pub async fn send_raw_outbound(&self, frame: crate::v3::context::OutboundFrame) -> Result<(), crate::v3::client::ClientError> {
        self.client().send_raw_outbound(frame).await
    }
}

impl Drop for IpcFixture {
    fn drop(&mut self) {
        self.cancel.cancel();
        self._task.abort();
    }
}

/// A bound server without a connected client.
/// For tests that manually connect clients (wrong-key, max-connections).
pub struct BoundServer {
    pub router: Arc<MockRouter>,
    pub cancel: CancellationToken,
    _task: tokio::task::JoinHandle<()>,
}

impl Drop for BoundServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        self._task.abort();
    }
}

/// The single handshake config used by all tests and benches.
pub fn handshake_config() -> HandshakeConfig {
    HandshakeConfig::new(
        CapabilityBits::MANDATORY_V1 | CapabilityBits::AEAD_AEGIS128L,
        Clearance::Internal,
    )
}

/// Generate a test/bench payload of the given size.
pub fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}

/// Read RSS from /proc/self/statm (Linux only).
#[allow(unsafe_code)]
pub fn get_rss_bytes() -> usize {
    #[cfg(target_os = "linux")]
    unsafe { libc::malloc_trim(0); }

    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<usize>().ok())
        .map(|pages| pages * 4096)
        .unwrap_or(0)
}

/// Number of physical cores (hyperthreads / 2, minimum 2).
pub fn phys_cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(2)
        .max(2)
}

/// Once-guarded tracing init for tests.
/// Uses `with_test_writer()` so output integrates with `cargo test --nocapture`.
pub fn init_tracing() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"))
            )
            .try_init();
    });
}
