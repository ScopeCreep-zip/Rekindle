//! Receipted control frames after route loss and out-of-order delivery.

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, GameInvite};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_records::port::RouteClass;
use rekindle_transport::operations::dm::{handshake, send_to_peer_call};
use rekindle_types::game_server::ServerAddress;
use rekindle_types::subscription_events::FriendEvent;

use crate::harness::{eventually, start, Events, SimNode};
use crate::pair::{befriend, key, talk, LIMIT};

async fn friends(seed: u64) -> (Netsim, Vec<SimNode>) {
    let (sim, mut nodes) = start(2, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "ready").await;
    (sim, nodes)
}

fn outside_dead(sim: &Netsim, to: &SimNode) {
    sim.fault(Fault::OutsideDead {
        node: to.index,
        class: RouteClass::General,
    });
}

async fn attempted(sim: &Netsim, from: &SimNode, to: &SimNode, kind: DmTypeId) {
    let owner = key(from);
    let stream = format!("friend_handshake:{}", key(to));
    let prefix = handshake::client_prefix(kind);
    let db = from.host.identity_stores().db.current().expect("outbox DB");
    assert!(
        eventually(sim, Duration::from_secs(30), || async {
            let (owner, stream) = (owner.clone(), stream.clone());
            db.call(move |conn| rekindle_db::repo::outbox::prefix_rows(conn, &owner, &stream))
                .await
                .expect("handshake rows")
                .iter()
                .any(|row| row.client_id.starts_with(&prefix) && row.attempts > 0)
        })
        .await,
        "{kind:?} attempted while the route was dead"
    );
}

async fn send_again(from: &SimNode, to: &SimNode, body: &DirectBody) {
    let host = from.host.friend_host();
    let receipt = send_to_peer_call(host.as_ref(), &key(to), body)
        .await
        .expect("receipt for the re-encrypted body");
    assert!(receipt.is_none(), "expected an empty receipt");
}

fn game_seen(events: &Events, game: u32) -> bool {
    events
        .friend()
        .iter()
        .any(|event| matches!(event, FriendEvent::GameInvite { game_id, .. } if *game_id == game))
}

fn server() -> ServerAddress {
    ServerAddress::parse("203.0.113.7:27015").expect("valid server")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn game_invite_retries_after_route_death() {
    let (sim, nodes) = friends(292).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    ann.ok(IpcRequest::SetShareCurrentGame { enabled: true })
        .await;
    let seen = bea.events().await;
    outside_dead(&sim, bea);
    ann.ok(IpcRequest::GameInviteSend {
        public_key: key(bea),
        game_id: 4181,
        server: server(),
    })
    .await;
    attempted(&sim, ann, bea, DmTypeId::GameInvite).await;
    assert!(!game_seen(&seen, 4181));
    sim.fault(Fault::RouteDeath { node: bea.index });
    assert!(
        eventually(&sim, LIMIT, || async { game_seen(&seen, 4181) }).await,
        "the invite is delivered on the replacement route before it expires"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn game_invite_ignores_expired_retry() {
    let (sim, nodes) = friends(293).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    let seen = bea.events().await;
    let now = ann.host.friend_host().clock().now_ms_i64();
    for (game_id, expires_at_ms) in [(4181, now), (4102, now + 120_000)] {
        send_again(
            ann,
            bea,
            &DirectBody::GameInvite(GameInvite {
                game_id,
                server: server(),
                expires_at_ms,
            }),
        )
        .await;
    }
    assert!(eventually(&sim, LIMIT, || async { game_seen(&seen, 4102) }).await);
    assert!(!game_seen(&seen, 4181), "the expired invite was shown");
}
