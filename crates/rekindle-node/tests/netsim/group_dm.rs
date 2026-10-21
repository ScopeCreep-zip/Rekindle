//! Group DMs end to end (plan E2.6 Verify): three members, two of whom are
//! not friends, create, accept and talk; a leave and a block each rotate
//! the remaining members' keys, and the one gone reads nothing after it;
//! a group send reads only `queued` then `sent`; a stretch of a member's
//! chain retired before we read it is a stored history-gap row.
//!
//! The group's sends and reads are driven through the Host's own
//! operations (`HostApi::friend_host`), the ones the IPC router calls.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_transport::operations::group_dm;
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::GroupDmView;
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, LIMIT};

/// The group as `node` lists it (a pending invite included).
async fn view(node: &SimNode, group_id: &str) -> Option<GroupDmView> {
    group_dm::find(node.host.friend_host().as_ref(), group_id)
        .await
        .expect("the group list")
}

/// `node`'s stored history of the group, oldest first.
async fn history(node: &SimNode, group_id: &str) -> Vec<DmMessage> {
    group_dm::messages(node.host.friend_host().as_ref(), group_id, 100, None)
        .await
        .expect("the group history")
}

/// The bodies of `node`'s group history that are messages, oldest first.
async fn bodies(node: &SimNode, group_id: &str) -> Vec<String> {
    history(node, group_id)
        .await
        .into_iter()
        .filter(|m| m.history_gap_through.is_none())
        .map(|m| m.body)
        .collect()
}

async fn reads(node: &SimNode, group_id: &str, body: &str) -> bool {
    bodies(node, group_id).await.iter().any(|b| b == body)
}

async fn send(node: &SimNode, group_id: &str, body: &str) {
    group_dm::send(node.host.friend_host().as_ref(), group_id, body)
        .await
        .expect("the group send");
}

/// `node`'s epoch in the group.
async fn epoch(node: &SimNode, group_id: &str) -> u32 {
    view(node, group_id).await.map_or(0, |v| v.epoch)
}

/// Ann (node 0) befriends Bea and Cat, who are not friends; Ann creates
/// the group, both accept, and every member holds the others' keys once
/// each has read a message of each other member. Returns the group id.
async fn three_members(sim: &Netsim, nodes: &mut [SimNode]) -> String {
    for (node, name) in nodes.iter_mut().zip(["Ann", "Bea", "Cat"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(sim, &nodes[1], &nodes[0]).await;
    befriend(sim, &nodes[2], &nodes[0]).await;
    let members = vec![key(&nodes[1]), key(&nodes[2])];
    let gid = group_dm::create(nodes[0].host.friend_host().as_ref(), "raid night", &members)
        .await
        .expect("the group is created");
    for node in &nodes[1..] {
        assert!(
            eventually(sim, LIMIT, || async {
                view(node, &gid).await.is_some_and(|v| v.pending)
            })
            .await,
            "node {} holds the invite",
            node.index
        );
        group_dm::accept(node.host.friend_host().as_ref(), &gid)
            .await
            .expect("the accept");
    }
    // Each sends once the previous message is read by everyone: everyone
    // reads all three, in the order sent.
    let said: Vec<String> = ["Ann", "Bea", "Cat"]
        .iter()
        .map(|n| format!("{n} says hi"))
        .collect();
    for (i, body) in said.iter().enumerate() {
        send(&nodes[i], &gid, body).await;
        for reader in nodes.iter() {
            assert!(
                eventually(sim, LIMIT * 2, || reads(reader, &gid, body)).await,
                "node {} reads {body:?}",
                reader.index
            );
        }
    }
    for reader in nodes.iter() {
        assert_eq!(bodies(reader, &gid).await, said, "node {}", reader.index);
    }
    gid
}

/// A group send's own rows and its events read `queued` or `sent`, never
/// `delivered` or `read` (a group has no receipts).
fn only_queued_or_sent(state: SendState) -> bool {
    matches!(state, SendState::Queued | SendState::Sent)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_group_of_non_friends_talks_and_a_leave_and_a_block_rotate() {
    let (sim, mut nodes) = start(3, 91).await;
    let gid = three_members(&sim, &mut nodes).await;
    let (bea, cat) = (key(&nodes[1]), key(&nodes[2]));
    for node in &nodes {
        let own: Vec<DmMessage> = history(node, &gid)
            .await
            .into_iter()
            .filter(|m| m.is_own)
            .collect();
        assert_eq!(own.len(), 1, "node {} holds its own send", node.index);
        assert!(own.iter().all(|m| only_queued_or_sent(m.send_state)));
    }

    // Cat leaves: Ann and Bea rotate, and a message after the leave never
    // reaches Cat.
    let ann_events = nodes[0].events().await;
    let (ann_epoch, bea_epoch) = (epoch(&nodes[0], &gid).await, epoch(&nodes[1], &gid).await);
    group_dm::leave(nodes[2].host.friend_host().as_ref(), &gid)
        .await
        .expect("the leave");
    for node in &nodes[..2] {
        let before = if node.index == 0 {
            ann_epoch
        } else {
            bea_epoch
        };
        assert!(
            eventually(&sim, LIMIT, || async {
                view(node, &gid)
                    .await
                    .is_some_and(|v| v.epoch > before && !v.members.iter().any(|m| m.key == cat))
            })
            .await,
            "node {} rotated past Cat",
            node.index
        );
    }
    assert!(view(&nodes[2], &gid).await.is_none(), "Cat holds no group");
    let cat_held = history(&nodes[2], &gid).await.len();
    send(&nodes[0], &gid, "after Cat left").await;
    assert!(eventually(&sim, LIMIT, || reads(&nodes[1], &gid, "after Cat left")).await);
    assert!(
        !eventually(&sim, Duration::from_secs(60), || async {
            history(&nodes[2], &gid).await.len() > cat_held
        })
        .await,
        "nothing new lands in Cat's history"
    );

    // Ann blocks Bea: Ann rotates, and Bea's later frames are dropped.
    let before = epoch(&nodes[0], &gid).await;
    nodes[0]
        .call(IpcRequest::FriendBlock {
            public_key: bea.clone(),
        })
        .await
        .expect("the block");
    assert!(
        eventually(&sim, LIMIT, || async {
            view(&nodes[0], &gid)
                .await
                .is_some_and(|v| v.epoch > before && !v.members.iter().any(|m| m.key == bea))
        })
        .await,
        "Ann rotated past Bea"
    );
    send(&nodes[1], &gid, "Bea after the block").await;
    assert!(
        !eventually(&sim, LIMIT, || reads(
            &nodes[0],
            &gid,
            "Bea after the block"
        ))
        .await,
        "Bea's frame is dropped at Ann"
    );

    // Ann's own sends never read delivered or read.
    assert!(!ann_events.any(|e| matches!(
        e,
        SubscriptionEvent::GroupDm(GroupDmEvent::SendStatus { state, .. })
            if !only_queued_or_sent(*state)
    )));
    assert!(ann_events.any(|e| matches!(
        e,
        SubscriptionEvent::GroupDm(GroupDmEvent::SendStatus {
            state: SendState::Sent,
            ..
        })
    )));
    assert!(history(&nodes[0], &gid)
        .await
        .iter()
        .filter(|m| m.is_own)
        .all(|m| only_queued_or_sent(m.send_state)));
}

/// Bea's DHT is off while Ann sends; 31 days on, Ann's next append retires
/// her chain past it (age only, no acks), and Bea's diff stores the gap row
/// and publishes it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retired_group_stretch_is_a_stored_history_gap() {
    let (sim, mut nodes) = start(3, 93).await;
    let gid = three_members(&sim, &mut nodes).await;
    let ann = key(&nodes[0]);
    let bea_events = nodes[1].events().await;
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    send(&nodes[0], &gid, "unread on the chain").await;
    assert!(
        eventually(&sim, LIMIT, || reads(
            &nodes[2],
            &gid,
            "unread on the chain"
        ))
        .await
    );
    sim.fault(Fault::ClockSkew {
        node: 0,
        ms: 31 * 24 * 60 * 60 * 1000,
    });
    send(&nodes[0], &gid, "after the horizon").await;
    assert!(eventually(&sim, LIMIT, || reads(&nodes[2], &gid, "after the horizon")).await);
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: false,
    });
    send(&nodes[0], &gid, "once more").await;
    let gap = || async {
        history(&nodes[1], &gid)
            .await
            .iter()
            .any(|m| m.history_gap_through.is_some() && m.sender_id == ann)
    };
    assert!(
        eventually(&sim, LIMIT * 2, gap).await,
        "Bea's group history holds the gap marker"
    );
    assert!(eventually(&sim, LIMIT, || reads(&nodes[1], &gid, "once more")).await);
    assert!(!reads(&nodes[1], &gid, "unread on the chain").await);
    assert!(bea_events.any(|e| matches!(
        e,
        SubscriptionEvent::GroupDm(GroupDmEvent::HistoryGap { member, .. })
            if member.public_key == ann
    )));
}
