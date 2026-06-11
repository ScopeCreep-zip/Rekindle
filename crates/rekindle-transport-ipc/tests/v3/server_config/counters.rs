use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rekindle_transport_ipc::v3::bulk::counters::BulkCounters;
use rekindle_transport_ipc::v3::context::ServerConfig;
use rekindle_transport_ipc::v3::crypto::noise::generate_keypair;
use rekindle_transport_ipc::v3::router::{MockRouter, ReplyRouter};
use rekindle_transport_ipc::v3::server::{ConnectionHandle, IpcServer};
use rekindle_transport_ipc::v3::client::IpcClient;

/// The counters Arc passed to ServerConfig must be the same Arc the
/// server writes to. After sending frames, the caller's Arc must
/// reflect the traffic.
#[tokio::test]
async fn server_counters_reflect_control_traffic() {
    let counters = BulkCounters::new();

    assert_eq!(
        counters.frames_sent.load(Ordering::Relaxed), 0,
        "precondition: counters must start at zero",
    );
    assert_eq!(
        counters.frames_received.load(Ordering::Relaxed), 0,
        "precondition: counters must start at zero",
    );

    let tempdir = tempfile::tempdir().unwrap();
    let socket_path = tempdir.path().join("counters.sock");
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public.clone().try_into().unwrap();

    let config = ServerConfig {
        counters: Arc::clone(&counters),
        ..ServerConfig::for_test()
    };

    let router = MockRouter::new();
    let router_ref = Arc::clone(&router);

    let server = IpcServer::bind(
        &socket_path,
        server_kp,
        move |handle: ConnectionHandle| {
            ReplyRouter {
                router: Arc::clone(&router_ref),
                conn_handle: handle,
            }
        },
        config,
    ).await.unwrap();

    let cancel = server.cancel_token().clone();
    tokio::spawn(async move { let _ = server.run().await; });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = IpcClient::connect(
        &socket_path, &server_pub, &client_kp,
        ServerConfig::for_test().session,
        ServerConfig::for_test().handshake,
    ).await.unwrap();

    for i in 0..10u32 {
        client.send_request(b"hello", Duration::from_secs(5)).await
            .unwrap_or_else(|e| panic!("request {i} failed: {e:?}"));
    }

    let received = counters.frames_received.load(Ordering::Relaxed);
    let sent = counters.frames_sent.load(Ordering::Relaxed);

    assert!(
        received >= 10,
        "after 10 roundtrips, frames_received must be >= 10, got {received} — \
         server is using different counters than the caller injected",
    );
    assert!(
        sent > 0,
        "after 10 roundtrips, frames_sent must be > 0 (replies), got {sent}",
    );

    client.shutdown().await;
    cancel.cancel();
}
