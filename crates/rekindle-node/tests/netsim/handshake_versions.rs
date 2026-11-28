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

async fn create_with(ann: &SimNode, bea: &SimNode, name: &str) -> String {
    use rekindle_ipc::protocol::{CommunityRequest, LifecycleRequest};
    use rekindle_types::dto::community::CommunityCreated;

    let data = ann
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreate {
                name: name.into(),
                description: String::new(),
                approval_required: false,
                co_creators: vec![key(bea)],
            },
        )))
        .await;
    serde_json::from_value::<CommunityCreated>(data)
        .expect("reserved creation")
        .anchor
}

fn creator_invited(events: &Events, anchor: &str) -> bool {
    events.friend().iter().any(
        |event| matches!(event, FriendEvent::CreatorInvited { anchor: held, .. } if held == anchor),
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn creator_invite_retries_after_route_death() {
    let (sim, nodes) = friends(294).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    let seen = bea.events().await;
    outside_dead(&sim, bea);
    let anchor = create_with(ann, bea, "Route loss").await;
    attempted(&sim, ann, bea, DmTypeId::CreatorInvite).await;
    assert!(!creator_invited(&seen, &anchor));
    sim.fault(Fault::RouteDeath { node: bea.index });
    assert!(
        eventually(&sim, LIMIT, || async { creator_invited(&seen, &anchor) }).await,
        "the co-creator sees the invitation on the replacement route"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn creator_invite_ignores_a_withdrawal_of_an_unseen_invite() {
    use rekindle_transport::operations::friend::creator_round::CreatorFrame;

    let (sim, nodes) = friends(295).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    let seen = bea.events().await;
    let anchor = format!("VLD0:{}", "Q".repeat(43));
    send_again(
        ann,
        bea,
        &CreatorFrame::Withdrawn {
            anchor: anchor.clone(),
        }
        .body(),
    )
    .await;
    send_again(
        ann,
        bea,
        &CreatorFrame::Invite {
            anchor: anchor.clone(),
            community_name: "Abandoned".into(),
        }
        .body(),
    )
    .await;
    let active = create_with(ann, bea, "Active").await;
    assert!(eventually(&sim, LIMIT, || async { creator_invited(&seen, &active) }).await);
    assert!(
        !creator_invited(&seen, &anchor),
        "the withdrawn invite was shown"
    );
    let owner = key(bea);
    let db = bea.host.identity_stores().db.current().expect("invite DB");
    assert!(db
        .call(move |conn| rekindle_db::repo::creator_round::invite(conn, &owner, &anchor))
        .await
        .expect("read invite")
        .is_none());
}

async fn profile(node: &SimNode) -> String {
    node.ok(IpcRequest::IdentityShow).await["profileDhtKey"]
        .as_str()
        .expect("own profile")
        .to_owned()
}

async fn rotate(node: &SimNode) -> String {
    rekindle_transport::operations::friend::rotate_profile_key(node.host.friend_host().as_ref())
        .await
        .expect("rotate profile and queue notices");
    profile(node).await
}

fn profile_seen(events: &Events, profile: &str) -> bool {
    events.friend().iter().any(|event| {
        matches!(event, FriendEvent::ProfileKeyRotated { new_profile_dht_key, .. }
            if new_profile_dht_key == profile)
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn friend_profile_rotation_retries_after_route_death() {
    let (sim, nodes) = friends(296).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    let seen = bea.events().await;
    outside_dead(&sim, bea);
    let new = rotate(ann).await;
    attempted(&sim, ann, bea, DmTypeId::ProfileKeyRotated).await;
    assert!(!profile_seen(&seen, &new));
    sim.fault(Fault::RouteDeath { node: bea.index });
    assert!(
        eventually(&sim, LIMIT, || async {
            sim.watched_by(bea.index).contains_key(&new)
        })
        .await,
        "the retried rotation moves the receiver's watch"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn friend_profile_rotation_ignores_an_older_record() {
    use rekindle_codec::capnp_codec::direct::ProfileKeyRotated;

    let (sim, nodes) = friends(297).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    let old = profile(ann).await;
    let new = rotate(ann).await;
    assert_ne!(old, new);
    assert!(
        eventually(&sim, LIMIT, || async {
            sim.watched_by(bea.index).contains_key(&new)
        })
        .await
    );
    let seen = bea.events().await;
    send_again(
        ann,
        bea,
        &DirectBody::ProfileKeyRotated(ProfileKeyRotated {
            new_profile_dht_key: old.clone(),
            version: 1,
        }),
    )
    .await;
    assert!(
        !eventually(&sim, Duration::from_secs(10), || async {
            profile_seen(&seen, &old)
        })
        .await,
        "an older rotation was applied"
    );
    assert!(sim.watched_by(bea.index).contains_key(&new));
    assert!(!sim.watched_by(bea.index).contains_key(&old));
}

async fn group(seed: u64) -> (Netsim, Vec<SimNode>, String) {
    let (sim, mut nodes) = start(3, seed).await;
    let group_id = crate::group_dm::three_members(&sim, &mut nodes).await;
    (sim, nodes, group_id)
}

async fn set_timer(node: &SimNode, group_id: &str, seconds: u32) {
    node.ok(IpcRequest::DisappearingSet {
        conversation: group_id.into(),
        seconds,
    })
    .await;
}

async fn timer(node: &SimNode, group_id: &str) -> u32 {
    let view: rekindle_types::dto::dm::DisappearingTimerView = serde_json::from_value(
        node.ok(IpcRequest::DisappearingGet {
            conversation: group_id.into(),
        })
        .await,
    )
    .expect("current group timer");
    view.seconds
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_disappearing_timer_retries_after_route_death() {
    let (sim, nodes, group_id) = group(298).await;
    let (ann, bea, cat) = (&nodes[0], &nodes[1], &nodes[2]);
    outside_dead(&sim, bea);
    // Cat and Bea are roster peers with a pairwise session.
    set_timer(cat, &group_id, 30).await;
    attempted(&sim, cat, bea, DmTypeId::DisappearingTimer).await;
    assert_eq!(timer(bea, &group_id).await, 0);
    assert!(eventually(&sim, LIMIT, || async { timer(ann, &group_id).await == 30 }).await);
    sim.fault(Fault::RouteDeath { node: bea.index });
    assert!(
        eventually(&sim, LIMIT, || async { timer(bea, &group_id).await == 30 }).await,
        "the replacement route carries the group timer to Bea"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn group_disappearing_timer_ignores_an_older_change() {
    use rekindle_codec::capnp_codec::direct::DisappearingTimer;
    use rekindle_transport::operations::group_dm::parse_group_id;

    let (sim, nodes, group_id) = group(299).await;
    let (ann, bea) = (&nodes[0], &nodes[1]);
    set_timer(ann, &group_id, 30).await;
    assert!(eventually(&sim, LIMIT, || async { timer(bea, &group_id).await == 30 }).await);
    set_timer(bea, &group_id, 60).await;
    assert!(eventually(&sim, LIMIT, || async { timer(ann, &group_id).await == 60 }).await);
    send_again(
        ann,
        bea,
        &DirectBody::DisappearingTimer(DisappearingTimer {
            seconds: 30,
            group_id: Some(parse_group_id(&group_id).expect("group ID")),
            group_version: Some(1),
        }),
    )
    .await;
    assert!(
        !eventually(&sim, Duration::from_secs(10), || async {
            timer(bea, &group_id).await != 60
        })
        .await,
        "the old group timer overwrote a newer change"
    );
}
