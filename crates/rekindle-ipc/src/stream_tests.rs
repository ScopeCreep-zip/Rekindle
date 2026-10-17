//! Each subscribed connection's stream from the host's journal, over a
//! real Noise bus (plan D3.2).

use std::sync::Arc;
use std::time::Duration;

use rekindle_events::SessionJournal;
use rekindle_types::subscription_events::{
    Cursor, NetworkEvent, StreamFrame, SubscriptionEvent, SubscriptionFilter,
};
use uuid::Uuid;

use super::message::SecurityLevel;
use super::noise_keys::generate_keypair;
use super::protocol::{IpcRequest, IpcResponse};
use super::registry::ClearanceRegistry;
use super::server::BusServer;
use super::{AgentType, BusClient};

const WAIT: Duration = Duration::from_secs(5);

fn changed(key: &str) -> SubscriptionEvent {
    SubscriptionEvent::Network(NetworkEvent::ValueChanged {
        record_key: key.into(),
        changed_subkeys: vec![],
    })
}

struct Fixture {
    journal: Arc<SessionJournal>,
    client: BusClient,
    _server: Arc<BusServer>,
    _dir: tempfile::TempDir,
}

/// A bus whose server reads `journal`, and a client registered at
/// `clearance`.
async fn fixture(journal: Arc<SessionJournal>, clearance: SecurityLevel) -> Fixture {
    let server_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let client_kp = generate_keypair().unwrap();
    let mut registry = ClearanceRegistry::new();
    registry.register(
        "tui".into(),
        client_kp.public().try_into().unwrap(),
        clearance,
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
    server.start_event_delivery(Arc::clone(&journal));
    let accept = Arc::clone(&server);
    tokio::spawn(async move {
        let _ = accept.run().await;
    });
    let client = BusClient::connect(Uuid::now_v7(), &sock, &server_pub, client_kp.as_inner())
        .await
        .unwrap();
    Fixture {
        journal,
        client,
        _server: server,
        _dir: dir,
    }
}

async fn subscribe(client: &BusClient, from: Option<Cursor>) -> IpcResponse {
    client
        .request(
            IpcRequest::Subscribe {
                filters: vec![SubscriptionFilter::all()],
                from,
            },
            SecurityLevel::Open,
            WAIT,
        )
        .await
        .unwrap()
}

fn seq(frame: &StreamFrame) -> Option<u64> {
    match frame {
        StreamFrame::Event(s) => Some(s.cursor.seq),
        _ => None,
    }
}

/// A client that comes back with its last cursor gets what it missed, in
/// order, then `Resumed`, then live events.
#[tokio::test]
async fn a_returning_client_resumes_from_its_cursor() {
    let journal = Arc::new(SessionJournal::default());
    let seen = journal.publish(changed("a"));
    journal.publish(changed("b"));
    journal.publish(changed("c"));
    let mut f = fixture(journal, SecurityLevel::Authenticated).await;
    let mut frames = f.client.take_event_receiver().unwrap();
    assert!(matches!(subscribe(&f.client, Some(seen)).await, IpcResponse::Ok(_)));
    f.journal.publish(changed("d"));
    let mut got = Vec::new();
    for _ in 0..4 {
        got.push(tokio::time::timeout(WAIT, frames.recv()).await.unwrap().unwrap());
    }
    assert_eq!(seq(&got[0]), Some(2));
    assert_eq!(seq(&got[1]), Some(3));
    assert!(matches!(got[2], StreamFrame::Resumed(Cursor { seq: 3, .. })));
    assert_eq!(seq(&got[3]), Some(4));
}

/// A cursor from another epoch cannot be served: `InvalidSession`.
#[tokio::test]
async fn a_stale_cursor_is_an_invalid_session() {
    let journal = Arc::new(SessionJournal::default());
    let stale = journal.publish(changed("a"));
    journal.begin();
    let mut f = fixture(journal, SecurityLevel::Authenticated).await;
    let mut frames = f.client.take_event_receiver().unwrap();
    assert!(matches!(subscribe(&f.client, Some(stale)).await, IpcResponse::Ok(_)));
    assert!(matches!(
        tokio::time::timeout(WAIT, frames.recv()).await.unwrap(),
        Some(StreamFrame::InvalidSession)
    ));
}

/// A client without `Authenticated` clearance cannot subscribe.
#[tokio::test]
async fn an_unauthenticated_client_cannot_subscribe() {
    let f = fixture(Arc::new(SessionJournal::default()), SecurityLevel::Open).await;
    assert!(matches!(
        subscribe(&f.client, None).await,
        IpcResponse::Error { code: 403, .. }
    ));
}
