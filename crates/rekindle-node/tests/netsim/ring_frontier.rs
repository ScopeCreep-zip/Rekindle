//! Plan E3.2 S7 (`evidence/slot-integrity.md` §4.3 (ii), review S7m part
//! 3): the watch ring's rebuild trigger is a member record's frontier
//! (`M`/3), the writer's commit point (it appends the op-log head `M`/2
//! first and then writes the frontier that cites it), deduplicated by
//! (record, frontier seq). A change on the head, a beat or an outbox alone
//! rebuilds nothing; and an op whose gossip is lost reaches a ring watcher
//! through the frontier change, well before the 60 s anti-entropy sweep.

use std::num::NonZeroUsize;
use std::time::Duration;

use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{ActivityRequest, ChannelRequest, CommunityRequest, IpcRequest};
use rekindle_netsim::Fault;
use rekindle_types::dht_layout::member_record::{BEAT, FRONTIER, GOV_HEAD, OUTBOX, SUBKEY_COUNT};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};
use tracing::Instrument as _;

use crate::events;
use crate::harness::{eventually, Community, Events};
use crate::ring_census::member_watchers;
use crate::watch_ring::creator_record;

/// The ring arm's rebuild line (`watch.rs` `refresh_member_record`): a
/// frontier read off a ring change or a hint stored a head we lacked. The
/// anti-entropy sweep rebuilds without it.
const RING_REBUILD: &str = "ring member's frontier moved — rebuilding that community";
/// A frontier catch-up's DHT read (`refresh_member_record`).
const READING: &str = "reading a member's frontier";
/// A ring watcher's frontier hint (`head_advanced.rs` `relay_head_advanced`).
const RELAYED: &str = "ring relayed a frontier hint";

fn rebuilds(events: &Events, community: &str) -> usize {
    events.matching(|e| {
        matches!(
            e,
            SubscriptionEvent::Governance(GovernanceEvent::GovernanceRebuilt { community: c })
                if c == community
        )
    })
}

async fn lists_channel(community: &Community, i: usize, name: &str) -> bool {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelList {
                community: community.anchor.clone(),
            },
        )))
        .await
        .is_ok_and(|channels| {
            channels
                .as_array()
                .into_iter()
                .flatten()
                .any(|c| c["name"] == name)
        })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lost_op_gossip_reaches_the_ring_watcher_through_the_frontier() {
    let community = Community::of(2, 4107).await;
    let key = community.anchor.clone();
    let creator = creator_record(&community);
    // Node 1 is the one ring watcher of the creator's `M`, and every
    // join-time write has settled.
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            community
                .sim
                .watched_subkeys(1, &creator)
                .contains(&FRONTIER)
        })
        .await,
        "node 1 watches the creator's member record"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.sim.quiesce().await;

    // `on_watched_change` dispatches by subkey: a beat, an op-log head or
    // an outbox alone rebuilds nothing; a frontier at a seq already read
    // rebuilds once at most (the first call marks it read); a record off
    // the ring is not ours.
    let governance = community.nodes[1].host.governance();
    let events = community.nodes[1].events().await;
    assert!(governance.on_watched_change(&creator, &[FRONTIER]).await);
    tokio::time::sleep(Duration::from_secs(15)).await;
    community.sim.quiesce().await;
    let marked = rebuilds(&events, &key);
    assert_eq!(
        marked, 0,
        "node 1 holds every head the settled frontier names: its forced read stores nothing, so nothing rebuilds"
    );
    for subkeys in [
        &[BEAT][..],
        &[GOV_HEAD],
        &[OUTBOX],
        &[BEAT, GOV_HEAD, OUTBOX],
        &[FRONTIER],
        &[GOV_HEAD, FRONTIER],
    ] {
        assert!(governance.on_watched_change(&creator, subkeys).await);
        community.sim.quiesce().await;
        assert_eq!(
            rebuilds(&events, &key),
            marked,
            "a ring change on {subkeys:?} rebuilt the community"
        );
    }
    assert!(
        !governance
            .on_watched_change("VLD0:not-a-ring-record", &[FRONTIER])
            .await
    );

    // Everything node 0 sends is lost, so its op's gossip never arrives;
    // only the DHT carries it.
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    // The create waits on sends that are lost, so both nodes are polled
    // (every 250 ms of sim time, without waiting for the sim to go quiet)
    // while it runs: the clock starts once the op is on the creator's
    // op log (`M`/2 in the network) and stops when node 1 holds it.
    let ring_rebuilds = || crate::logs::logged(1, &[RING_REBUILD, creator.as_str()]).len();
    let before_ring = ring_rebuilds();
    let head = |sim: &rekindle_netsim::Netsim| sim.network_get(&creator, GOV_HEAD).map(|v| v.seq);
    let before = head(&community.sim);
    let create = community.nodes[0].ok(IpcRequest::Community(CommunityRequest::Channels(
        ChannelRequest::ChannelCreate {
            community: key.clone(),
            name: "announcements".into(),
            kind: "text".into(),
            category: None,
            topic: None,
            slowmode_seconds: 0,
            parent_voice_channel: None,
        },
    )));
    let read = async {
        let issued = tokio::time::Instant::now();
        while head(&community.sim) == before {
            assert!(
                issued.elapsed() < Duration::from_secs(120),
                "node 0 never appended its op"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        while !lists_channel(&community, 1, "announcements").await {
            assert!(
                issued.elapsed() < Duration::from_secs(600),
                "node 1 never got the op"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    tokio::join!(create, read);
    // The op reached node 1 on the ring path: its ring arm read the
    // creator's frontier and stored the op (the sweep, had it come first,
    // would have left that read nothing to store, and no hint reaches node
    // 1 from node 0), whatever the sweep's phase.
    assert_eq!(
        ring_rebuilds(),
        before_ring + 1,
        "node 1's ring read of the creator's frontier delivered the op"
    );
    assert!(
        rebuilds(&events, &key) > marked,
        "the frontier change rebuilt the community"
    );
}

/// R5-3 (A/B round 5): a ring member's frontier that moved but names only
/// heads we hold stores nothing, and a rebuild merges stored heads
/// (`anti_entropy`), so it does not rebuild. Here node 1 authors an op; the
/// creator merges it and republishes its frontier citing it (a new seq);
/// node 1, the creator's ring watcher, reads that frontier, stores nothing
/// (the op is its own) and must not rebuild for it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_moved_frontier_naming_only_held_heads_rebuilds_nothing() {
    let community = Community::of(2, 4108).await;
    let key = community.anchor.clone();
    let creator = creator_record(&community);
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            community
                .sim
                .watched_subkeys(1, &creator)
                .contains(&FRONTIER)
        })
        .await,
        "node 1 watches the creator's member record"
    );
    // The creator schedules an event; node 1's RSVP to it is an op any
    // member may author (`validate`: "Answering an event is not a
    // permission").
    let event = events::create(
        &community.nodes[0],
        &key,
        events::new_event("Raid night", 4_000_000_000),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.sim.quiesce().await;

    let frontier =
        |sim: &rekindle_netsim::Netsim| sim.network_get(&creator, FRONTIER).map(|v| v.seq);
    let before = frontier(&community.sim);
    let seen = community.nodes[1].events().await;
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::EventRsvp {
                community: key.clone(),
                event_id: event,
                status: "going".into(),
            },
        )))
        .await;
    assert_eq!(
        frontier(&community.sim),
        before,
        "the creator has not republished its frontier yet"
    );
    let own = rebuilds(&seen, &key);
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            frontier(&community.sim) != before
        })
        .await,
        "the creator merged node 1's op and republished its frontier"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.sim.quiesce().await;
    assert_eq!(
        rebuilds(&seen, &key),
        own,
        "node 1 rebuilt for a frontier naming only an op it holds"
    );
}

/// `community` once every member record has a ring watcher and every
/// join-time write has settled.
async fn ring_settled(community: &Community) {
    let n = community.nodes.len();
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            let watched = member_watchers(community, FRONTIER);
            watched.len() >= n && watched.values().all(|w| !w.is_empty())
        })
        .await,
        "every member record has a ring watcher"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.sim.quiesce().await;
}

/// The op-log head (`M`/2) seq of every member record.
fn heads(community: &Community) -> std::collections::BTreeMap<String, u64> {
    community
        .sim
        .watchers_of_dflt(SUBKEY_COUNT)
        .keys()
        .filter_map(|r| {
            let seq = community.sim.network_get(r, GOV_HEAD)?.seq;
            Some((r.clone(), u64::from(seq)))
        })
        .collect()
}

fn relayed(community: &Community) -> usize {
    (0..community.nodes.len())
        .map(|i| crate::logs::logged(i, &[RELAYED]).len())
        .sum()
}

/// Review S7m 83f7fc6b P2 (item 6 (a)): a frontier change is relayed only
/// when it follows its author's head advance. One op moves its author's
/// head; every other member merges it and republishes its own frontier
/// with no head change, which stays ring-local. So the frontier hints
/// relayed for one op are at most each moved head's seq increase times its
/// record's ring watchers, never one per republish (the O(k·N²) flood).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn one_op_relays_frontier_hints_only_for_the_heads_it_moved() {
    let community = Community::of(4, 4109).await;
    ring_settled(&community).await;
    let before = heads(&community);
    let relayed_before = relayed(&community);
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.anchor.clone(),
                name: "announcements".into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            let mut listed = true;
            for i in 0..community.nodes.len() {
                listed &= lists_channel(&community, i, "announcements").await;
            }
            listed
        })
        .await,
        "every member merged the op"
    );
    tokio::time::sleep(Duration::from_secs(30)).await;
    community.sim.quiesce().await;
    let watchers = member_watchers(&community, FRONTIER);
    let bound: u64 = heads(&community)
        .iter()
        .map(|(record, seq)| {
            let moved = seq - before.get(record).copied().unwrap_or(*seq);
            moved * watchers.get(record).map_or(0, |w| w.len() as u64)
        })
        .sum();
    let sent = (relayed(&community) - relayed_before) as u64;
    assert!(bound > 0, "the op moved its author's watched head");
    assert!(
        sent <= bound,
        "{sent} frontier hints relayed for one op, more than the {bound} its head advances allow"
    );
}

/// Review S7m 83f7fc6b P1 (item 6 (b)): concurrent triggers for one
/// frontier cost one catch-up: whichever claims the record first reads
/// it, under the communities lock, and the other is refused.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_frontier_triggers_read_the_record_once() {
    let community = Community::of(2, 4110).await;
    let creator = creator_record(&community);
    ring_settled(&community).await;
    let governance = community.nodes[1].host.governance();
    let reads = || crate::logs::logged(1, &[READING, creator.as_str()]).len();
    let before = reads();
    // In node 1's span, as its watch dispatch runs, so the reads count
    // under node 1.
    let (first, second) = async {
        tokio::join!(
            governance.on_watched_change(&creator, &[FRONTIER]),
            governance.on_watched_change(&creator, &[FRONTIER]),
        )
    }
    .instrument(crate::harness::node_span(1))
    .await;
    assert!(first && second, "both are ring changes");
    community.sim.quiesce().await;
    assert_eq!(reads(), before + 1, "one frontier read for both triggers");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ring_frontier_retries_when_the_claiming_read_lags() {
    let community = Community::of(2, 4117).await;
    let creator = creator_record(&community);
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            community
                .sim
                .watched_subkeys(1, &creator)
                .contains(&FRONTIER)
        })
        .await
    );
    community.sim.quiesce().await;
    let previous = community.sim.network_get(&creator, FRONTIER).unwrap().seq;

    // Deliver one watch trigger explicitly, so another replica's push
    // or an app hint cannot rescue the first lagging fanout.
    community.sim.fault(Fault::DropValueChanges {
        node: 1,
        key: creator.clone(),
        drop: true,
    });
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    community.sim.fault(Fault::GetsFromLaggingReplica {
        node: 1,
        key: creator.clone(),
        subkey: FRONTIER,
        count: NonZeroUsize::MIN,
    });
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.anchor.clone(),
                name: "lagging-frontier".into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    let current = community.sim.network_get(&creator, FRONTIER).unwrap().seq;
    assert_eq!(current, previous + 1);
    assert!(!lists_channel(&community, 1, "lagging-frontier").await);

    let before = community.sim.app_gets_at(1, &creator, FRONTIER);
    assert!(
        community.nodes[1]
            .host
            .governance()
            .on_watched_change(&creator, &[FRONTIER])
            .await
    );
    assert!(lists_channel(&community, 1, "lagging-frontier").await);
    let port = community.nodes[1].host.governance().record_port().unwrap();
    let caught_up = port
        .read_once(&creator, FRONTIER, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(caught_up.seq, current, "node 1 holds the written frontier");
    assert_eq!(community.sim.app_gets_at(1, &creator, FRONTIER) - before, 2);
}

/// An inspected seq 4 arrives while the seq-3 read is finishing, then
/// another watch report names 4 after that catch-up. The inspect sees
/// the write ahead of the replicas reached by the forced gets.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ring_frontier_catches_up_an_overlapping_and_late_trigger() {
    let community = Community::of(2, 4117).await;
    let creator = creator_record(&community);
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            community
                .sim
                .watched_subkeys(1, &creator)
                .contains(&FRONTIER)
        })
        .await
    );
    community.sim.quiesce().await;
    community.sim.fault(Fault::DropValueChanges {
        node: 1,
        key: creator.clone(),
        drop: true,
    });
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    let writer = &community.nodes[0];
    let watcher = community.nodes[1].host.governance();
    let seq = || community.sim.network_get(&creator, FRONTIER).unwrap().seq;
    let gets = || community.sim.app_gets_at(1, &creator, FRONTIER);
    let anchor = community.anchor.as_str();
    let write_channel = |name: &'static str| async move {
        writer
            .ok(IpcRequest::Community(CommunityRequest::Channels(
                ChannelRequest::ChannelCreate {
                    community: anchor.to_owned(),
                    name: name.into(),
                    kind: "text".into(),
                    category: None,
                    topic: None,
                    slowmode_seconds: 0,
                    parent_voice_channel: None,
                },
            )))
            .await;
    };
    // The join writes seq 2. Deliver it before the two later writes.
    assert_eq!(seq(), 2);
    assert!(watcher.on_watched_change(&creator, &[FRONTIER]).await);
    write_channel("copy").await;
    assert_eq!(seq(), 3);
    assert!(watcher.on_watched_change(&creator, &[FRONTIER]).await);
    assert!(lists_channel(&community, 1, "copy").await);

    // Pause completion of a seq-3 report in the equal-heads convergence
    // check. It holds a frontier flight while seq 4 is written and reported.
    let lock = watcher.rebuild_lock(&community.anchor);
    let installing = lock.lock().await;
    let before = gets();
    // The claiming get and the next two fanouts reach replicas at 3.
    community.sim.fault(Fault::GetsFromLaggingReplica {
        node: 1,
        key: creator.clone(),
        subkey: FRONTIER,
        count: NonZeroUsize::new(3).unwrap(),
    });
    let in_flight = watcher.on_watched_change(&creator, &[FRONTIER]);
    let newer = async {
        assert!(
            eventually(&community.sim, Duration::from_secs(5), || async {
                gets() > before
            })
            .await,
            "the claiming get started"
        );
        write_channel("after-the-copy").await;
        assert_eq!(seq(), 4);
        assert!(watcher.on_watched_change(&creator, &[FRONTIER]).await);
        assert_eq!(gets(), before + 1, "seq 4 joined the existing flight");
        drop(installing);
    };
    let (handled, ()) = tokio::join!(in_flight, newer);
    assert!(handled);
    let overlapping_gets = gets() - before;
    assert!(watcher.on_watched_change(&creator, &[FRONTIER]).await);
    assert!(
        lists_channel(&community, 1, "after-the-copy").await,
        "the watcher has the op named by frontier seq 4 after both reports"
    );
    assert_eq!(
        overlapping_gets, 3,
        "seq 4 had a forced get and a capped refetch"
    );
    assert_eq!(
        gets() - before,
        4,
        "the late trigger clears the remaining lag"
    );
}
