//! `connect_with_retry` asks for the server key at every attempt: a daemon
//! that starts between two attempts serves a new key (live A/B finding 4).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::error::IpcError;
use super::noise_keys::generate_keypair;
use super::registry::ClearanceRegistry;
use super::server::BusServer;
use super::BusClient;

/// A bus serving a fresh key: (its socket's directory, socket, the key).
fn serving() -> (tempfile::TempDir, std::path::PathBuf, [u8; 32]) {
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("bus.sock");
    let server = Arc::new(
        BusServer::bind(
            &sock,
            server_kp.into_inner(),
            Arc::new(tokio::sync::RwLock::new(ClearanceRegistry::new())),
        )
        .unwrap(),
    );
    tokio::spawn(async move {
        let _ = server.run().await;
    });
    (dir, sock, server_pub)
}

#[tokio::test]
async fn a_retry_reads_the_key_the_new_daemon_published() {
    let (_dir, sock, current) = serving();
    let previous: [u8; 32] = generate_keypair().unwrap().public().try_into().unwrap();
    let reads = AtomicU32::new(0);
    // The first read is the previous run's bus.pub; the next is this run's.
    let key = async || {
        Ok(if reads.fetch_add(1, Ordering::SeqCst) == 0 {
            previous
        } else {
            current
        })
    };
    let client_kp = generate_keypair().unwrap();
    let connected = BusClient::connect_with_retry(
        Uuid::now_v7(),
        &sock,
        key,
        client_kp.as_inner(),
        2,
        Duration::from_millis(10),
    )
    .await;
    assert!(connected.is_ok(), "the retry handshakes with the new key");
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_key_the_daemon_does_not_serve_never_connects() {
    let (_dir, sock, _current) = serving();
    let previous: [u8; 32] = generate_keypair().unwrap().public().try_into().unwrap();
    let client_kp = generate_keypair().unwrap();
    let connected = BusClient::connect_with_retry(
        Uuid::now_v7(),
        &sock,
        async || Ok::<_, IpcError>(previous),
        client_kp.as_inner(),
        2,
        Duration::from_millis(10),
    )
    .await;
    assert!(connected.is_err());
}
