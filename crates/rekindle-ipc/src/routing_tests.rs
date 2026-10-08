//! Bus routing over a real Noise bus: who may answer a request, and what
//! the server stamps on the frames it forwards.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::message::{Message, MessageContext, SecurityLevel};
use super::noise_keys::generate_keypair;
use super::protocol::{BusPayload, IpcRequest, IpcResponse};
use super::registry::ClearanceRegistry;
use super::server::{BusServer, DAEMON_AGENT_NAME};
use super::{AgentType, BusClient};

const WAIT: Duration = Duration::from_secs(5);

/// A running bus with a registered daemon connection.
struct Bus {
    server: Arc<BusServer>,
    server_pub: [u8; 32],
    daemon: BusClient,
    _dir: tempfile::TempDir,
    sock: std::path::PathBuf,
}

impl Bus {
    async fn start() -> Self {
        let server_kp = generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
        let daemon_kp = generate_keypair().unwrap();
        let mut registry = ClearanceRegistry::new();
        registry.register(
            DAEMON_AGENT_NAME.to_string(),
            daemon_kp.public().try_into().unwrap(),
            SecurityLevel::Internal,
            AgentType::System,
            vec![],
        );
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("bus.sock");
        let server = Arc::new(
            BusServer::bind(
                &sock,
                server_kp.into_inner(),
                Arc::new(tokio::sync::RwLock::new(registry)),
            )
            .unwrap(),
        );
        let accept = Arc::clone(&server);
        tokio::spawn(async move {
            let _ = accept.run().await;
        });
        let daemon = connect(&sock, &server_pub, &daemon_kp).await;
        Self {
            server,
            server_pub,
            daemon,
            _dir: dir,
            sock,
        }
    }

    async fn client(&self) -> (BusClient, [u8; 32]) {
        let kp = generate_keypair().unwrap();
        let public = kp.public().try_into().unwrap();
        (connect(&self.sock, &self.server_pub, &kp).await, public)
    }

    /// The next request the daemon receives.
    async fn next_request(&mut self) -> Message<BusPayload> {
        tokio::time::timeout(WAIT, self.daemon.recv_bus_message())
            .await
            .expect("daemon got a request")
            .expect("bus open")
            .expect("frame decodes")
    }
}

async fn connect(
    sock: &Path,
    server_pub: &[u8; 32],
    kp: &super::noise_keys::ZeroizingKeypair,
) -> BusClient {
    BusClient::connect(Uuid::now_v7(), sock, server_pub, kp.as_inner())
        .await
        .unwrap()
}

fn json(response: &IpcResponse) -> Vec<u8> {
    serde_json::to_vec(response).unwrap()
}

#[tokio::test]
async fn only_the_daemon_can_answer_a_request() {
    let mut bus = Bus::start().await;
    let (victim, _) = bus.client().await;
    let (attacker, _) = bus.client().await;

    let pending = tokio::spawn(async move {
        victim
            .request(IpcRequest::Status, SecurityLevel::Open, WAIT)
            .await
    });
    let request = bus.next_request().await;

    // A forged answer from another client is dropped by the router...
    attacker
        .respond(
            json(&IpcResponse::error(418, "forged")),
            request.msg_id,
            SecurityLevel::Open,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // ...so the daemon's answer is the one the victim receives.
    bus.daemon
        .respond(
            json(&IpcResponse::ok(&"genuine")),
            request.msg_id,
            request.security_level,
        )
        .await
        .unwrap();

    let response = pending.await.unwrap().unwrap();
    assert!(
        matches!(&response, IpcResponse::Ok(v) if v == "genuine"),
        "victim got {response:?}"
    );
}

#[tokio::test]
async fn the_server_stamps_the_senders_real_name_and_key() {
    let mut bus = Bus::start().await;
    let (client, client_pub) = bus.client().await;

    let mut forged = Message::new(
        &MessageContext::new(client.sender_id()),
        BusPayload::Request(IpcRequest::Status),
        SecurityLevel::Open,
        client.epoch(),
    );
    forged.verified_sender_name = Some(DAEMON_AGENT_NAME.to_string());
    forged.verified_sender_key = Some([9; 32]);
    client.send(&forged).await.unwrap();

    let received = bus.next_request().await;
    assert_eq!(received.verified_sender_name, None);
    assert_eq!(received.verified_sender_key, Some(client_pub));
}

#[tokio::test]
async fn a_reply_in_flight_at_shutdown_still_arrives() {
    let mut bus = Bus::start().await;
    let (client, _) = bus.client().await;
    let pending = tokio::spawn(async move {
        client
            .request(IpcRequest::Shutdown, SecurityLevel::Open, WAIT)
            .await
    });
    let request = bus.next_request().await;

    // The daemon answers and closes its connection, as its subscriber does
    // when it drains; the server then closes the client's connection.
    bus.daemon
        .respond(
            json(&IpcResponse::ok(&"stopping")),
            request.msg_id,
            request.security_level,
        )
        .await
        .unwrap();
    let Bus { server, daemon, .. } = bus;
    daemon.shutdown().await;
    tokio::time::timeout(WAIT, server.shutdown(WAIT))
        .await
        .expect("drain finishes");

    let response = pending.await.unwrap().unwrap();
    assert!(matches!(&response, IpcResponse::Ok(v) if v == "stopping"));
}

#[tokio::test]
async fn before_the_daemon_connects_requests_get_503_not_silence() {
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
    let accept = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = accept.run().await;
    });
    let kp = generate_keypair().unwrap();
    let client = connect(&sock, &server_pub, &kp).await;

    let started = std::time::Instant::now();
    let response = client
        .request(IpcRequest::Status, SecurityLevel::Open, WAIT)
        .await
        .unwrap();
    assert!(
        matches!(response, IpcResponse::Error { code: 503, .. }),
        "{response:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "answered, not timed out"
    );
}
