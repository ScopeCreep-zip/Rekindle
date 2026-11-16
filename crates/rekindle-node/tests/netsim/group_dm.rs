//! Group DMs end to end (plan E2.6 Verify): three members, two of whom are
//! not friends, create, accept and talk; a leave and a block each rotate
//! the remaining members' keys, and the one gone reads nothing after it;
//! a group send reads only `queued` then `sent`; a stretch of a member's
//! chain retired before we read it is a stored history-gap row.
//!
//! Every group operation is an IPC request through the harness
//! (`GroupDmCreate`, `GroupDmAccept`, `GroupDmLeave`, `GroupDmSend`,
//! `GroupDmList`, `GroupDmMessages`), as a frontend sends it.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::GroupDmView;
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{GroupDmEvent, SubscriptionEvent};

use crate::harness::{eventually, start, SimNode};
use crate::pair::{befriend, key, retired, unretired, LIMIT};

/// The group as `node` lists it (a pending invite included).
async fn view(node: &SimNode, group_id: &str) -> Option<GroupDmView> {
    let groups: Vec<GroupDmView> =
        serde_json::from_value(node.ok(IpcRequest::GroupDmList).await).expect("the group list");
    groups.into_iter().find(|g| g.group_id == group_id)
}

/// `node`'s stored history of the group, oldest first.
async fn history(node: &SimNode, group_id: &str) -> Vec<DmMessage> {
    serde_json::from_value(
        node.ok(IpcRequest::GroupDmMessages {
            group_id: group_id.to_string(),
            limit: 100,
            before: None,
        })
        .await,
    )
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
    node.ok(IpcRequest::GroupDmSend {
        group_id: group_id.to_string(),
        body: body.to_string(),
    })
    .await;
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
    let gid: String = serde_json::from_value(
        nodes[0]
            .ok(IpcRequest::GroupDmCreate {
                name: "raid night".to_string(),
                members,
            })
            .await,
    )
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
        node.ok(IpcRequest::GroupDmAccept {
            group_id: gid.clone(),
        })
        .await;
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
    nodes[2]
        .ok(IpcRequest::GroupDmLeave {
            group_id: gid.clone(),
        })
        .await;
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
/// and publishes it. Returns the run's trace hash
/// ([`a_retired_group_stretch_has_one_trace_per_seed`]).
async fn retired_stretch() -> [u8; 32] {
    retired_stretch_at(93).await
}

async fn retired_stretch_at(seed: u64) -> [u8; 32] {
    let (sim, mut nodes) = start(3, seed).await;
    let gid = three_members(&sim, &mut nodes).await;
    let ann = key(&nodes[0]);
    let bea_events = nodes[1].events().await;
    // E2.5's design sends a message twice, by the direct frame and by the
    // chain, and either may reach Cid first; an entry his hint diff reads
    // before his watch on Ann's chain is held is reported by no change, so
    // reading a message says nothing of whether a watched change of her
    // chain reached him. As `pair::talk_until_watched` (db0c8cd1), Ann talks
    // until one has (at most four rounds), so the snapshot below names her
    // chain whichever path came first.
    for round in 0..4 {
        let body = format!("watched {round}");
        send(&nodes[0], &gid, &body).await;
        assert!(eventually(&sim, LIMIT, || reads(&nodes[2], &gid, &body)).await);
        if eventually(&sim, Duration::from_secs(15), || async {
            unretired(&sim, 2).iter().any(|k| sim.creator(k) == Some(0))
        })
        .await
        {
            break;
        }
    }
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
    // Finding F10: Cid reads the entry once its page and `HEAD` are on
    // replicas, while Ann's append is still in its set fanout (S11); Ann
    // notes the entry's age only when the append returns, so a skew before
    // that dates it 31 days on and her retire stops below it. Her row
    // settles sent once the entry is noted.
    assert!(
        eventually(&sim, LIMIT, || async {
            history(&nodes[0], &gid).await.iter().any(|m| {
                m.is_own && m.body == "unread on the chain" && m.send_state == SendState::Sent
            })
        })
        .await,
        "Ann's entry is noted on her chain"
    );
    let keys = unretired(&sim, 2);
    sim.fault(Fault::ClockSkew {
        node: 0,
        ms: 31 * 24 * 60 * 60 * 1000,
    });
    send(&nodes[0], &gid, "after the horizon").await;
    assert!(eventually(&sim, LIMIT, || reads(&nodes[2], &gid, "after the horizon")).await);
    // Cid reads the entry from its page once `HEAD` names it; the retire
    // mark is Ann's next write (a set of its own), and Bea must not be
    // back (her watch and catch-up read the chain at once) before it is on
    // the network, or she reads the stretch the mark is about to retire.
    assert!(
        eventually(&sim, LIMIT, || async { retired(&sim, &keys) }).await,
        "Ann's group chain is retired"
    );
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
    sim.trace().hash()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_retired_group_stretch_is_a_stored_history_gap() {
    retired_stretch().await;
}

/// Ann is detached while she writes a backlog; once she is back, the
/// outbox hands the stream's due run over at once and it lands on her
/// group chain in as few pages as hold it (plan E2.B F2: an append per
/// page, not per message), and every member reads each message in order.
/// The first message sent while detached is attempted alone; detached, its
/// page set fails at once (0.5.8 `set_value.rs:247-255`), so it is due again
/// with the backlog behind it and the whole run lands together.
/// (Premise rewritten in E2.5i: it read that the first attempt "waits for
/// readiness with only itself due" and so took a page of its own. That wait
/// was the append's network read of its own `HEAD`; a dm or group chain's
/// append now reads its local copy, `rekindle_records::chain`, and nothing of
/// it waits on readiness.)
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_group_backlog_lands_in_as_few_pages_as_hold_it() {
    let (sim, mut nodes) = start(3, 97).await;
    let gid = three_members(&sim, &mut nodes).await;
    sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    // Every chain page is a sealed single-subkey record.
    let pages_before = sim.single_subkey_values().len();
    send(&nodes[0], &gid, "in flight").await;
    // Every task runs until it waits, so the outbox takes the first row
    // alone before the backlog is written (F11: a database call no longer
    // lets it in between two sends).
    sim.quiesce().await;
    let backlog: Vec<String> = (0..8).map(|i| format!("backlog {i}")).collect();
    for body in &backlog {
        send(&nodes[0], &gid, body).await;
    }
    sim.fault(Fault::Detach {
        node: 0,
        detached: false,
    });
    let mut expected = vec!["in flight".to_string()];
    expected.extend(backlog.iter().cloned());
    for reader in &nodes[1..] {
        for body in &expected {
            assert!(
                eventually(&sim, LIMIT * 2, || reads(reader, &gid, body)).await,
                "node {} reads {body:?}",
                reader.index
            );
        }
        assert_eq!(
            bodies(reader, &gid).await[3..],
            expected[..],
            "node {} reads them in order",
            reader.index
        );
    }
    // The first message and the backlog: nine short entries fit one page,
    // ceil(9 / capacity) = 1.
    let appended = sim.single_subkey_values().len() - pages_before;
    assert_eq!(appended, 1, "{appended} page appends");
}

/// One seed, one trace (ADR 0021, C9.3) through the outbox's concurrent
/// stream attempts (E2.5j) and the transport's dispatch: the retired
/// stretch's trace split while both polled their `select!` branches in
/// tokio's drawn order (node 2's value-change handler and its outbox
/// `GroupDmAccept` send swapped at one sim instant). Each run is a runtime
/// of its own, so each draws another `select!` seed. Under seed 91 with the
/// selects unbiased, three runs gave three traces (7 of seeds 90-99 split).
#[test]
fn a_retired_group_stretch_has_one_trace_per_seed() {
    fn run() -> [u8; 32] {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("a runtime")
            .block_on(retired_stretch_at(91))
    }
    let first = run();
    // Six runtimes: under the unbiased selects one run in three still
    // matched the first, so four runtimes could pass a split.
    for _ in 0..5 {
        assert_eq!(run(), first, "one seed, one trace");
    }
}
