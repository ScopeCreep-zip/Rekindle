//! Plan E3.2 S7 (`evidence/slot-integrity.md` §4.3 (ii), review S7m part
//! 3): the watch ring's rebuild trigger is a member record's frontier
//! (`M`/3), the writer's commit point (it appends the op-log head `M`/2
//! first and then writes the frontier that cites it), deduplicated by
//! (record, frontier seq). A change on the head, a beat or an outbox alone
//! rebuilds nothing; and an op whose gossip is lost reaches a ring watcher
//! through the frontier change, well before the 60 s anti-entropy sweep.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};
use rekindle_netsim::Fault;
use rekindle_types::dht_layout::member_record::{BEAT, FRONTIER, GOV_HEAD, OUTBOX, SUBKEY_COUNT};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, Events};
use crate::watch_ring::creator_record;

/// Clearly under one anti-entropy sweep period (60 s): the ring path takes
/// about 27 s of sim time here (the frontier write after the head, then the
/// reader's inspect, frontier read and op fetch, at SimDht latency); with
/// the head (`M`/2) as the trigger the op waited for the sweep (69 s).
const RING_BOUND: Duration = Duration::from_secs(40);

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
                community: community.governance_key.clone(),
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
    let key = community.governance_key.clone();
    let creator = creator_record(&community);
    // Node 1 is the one ring watcher of the creator's `M`, and every
    // join-time write has settled.
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            community
                .sim
                .watchers_of_dflt(SUBKEY_COUNT)
                .get(&creator)
                .is_some_and(|w| w.contains(&1))
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
    assert!(marked <= 1, "one frontier seq costs one rebuild at most");
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
        let appended = tokio::time::Instant::now();
        while !lists_channel(&community, 1, "announcements").await
            && appended.elapsed() < RING_BOUND
        {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        appended.elapsed()
    };
    let (_, took) = tokio::join!(create, read);
    assert!(
        took < RING_BOUND,
        "the op reached the ring watcher {took:?} after its author appended it, not within {RING_BOUND:?}"
    );
    assert!(
        rebuilds(&events, &key) > marked,
        "the frontier change rebuilt the community"
    );
}
