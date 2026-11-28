//! Plan E3.4 Q1, the members on screen: the 15-minute rule covers the ring
//! plus the members a frontend shows (at most 50). A member outside the
//! reader's ring shows its last-known status; once the reader reports it on
//! screen (`MembersOnScreen`), its beat is inspected at once and once per
//! beat period, so it shows online while it beats, offline at expiry once
//! its beats stop, and online again on its next beat.

use std::time::Duration;

use rekindle_codec::member_record::BeatState;
use rekindle_governance_runtime::watch_ring::RING_K;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, UnreadRequest};
use rekindle_netsim::Fault;
use rekindle_types::dht_layout::member_record::BEAT;

use crate::harness::{eventually, Community};
use crate::ring_census::member_watchers;
use crate::watch_ring::creator_record;

/// The 15-minute rule, one inspection period and the expiry's 30 s check.
const STALE_BOUND: Duration = Duration::from_secs(15 * 60 + 5 * 60 + 60);

fn shown(community: &Community, reader: usize, member: &str) -> BeatState {
    community.nodes[reader]
        .host
        .governance()
        .member_beat_shown(&community.anchor, member)
}

async fn shows(
    community: &Community,
    reader: usize,
    member: &str,
    state: BeatState,
    bound: Duration,
) -> bool {
    eventually(&community.sim, bound, || async {
        shown(community, reader, member) == state
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_on_screen_outside_the_ring_stays_live_and_expires() {
    let community = Community::of(6, 4320).await;
    let creator = community.pseudonym(0).await;
    let record = creator_record(&community);
    // The creator's ring watchers settle at k = 4; the one other member
    // does not watch it.
    let watching = || {
        member_watchers(&community, BEAT)
            .remove(&record)
            .unwrap_or_default()
    };
    assert!(
        eventually(&community.sim, Duration::from_secs(600), || async {
            watching().len() == RING_K
        })
        .await,
        "the creator's record has its four ring watchers"
    );
    let watchers = watching();
    let reader = (1..6)
        .find(|i| !watchers.contains(i))
        .expect("a member outside the creator's ring");
    // Off screen: whatever a hint gave the reader goes stale.
    assert!(
        shows(
            &community,
            reader,
            &creator,
            BeatState::Offline,
            STALE_BOUND
        )
        .await,
        "off screen, the creator's last-known beat expires on node {reader}"
    );
    community.nodes[reader]
        .ok(IpcRequest::Community(CommunityRequest::Unread(
            UnreadRequest::MembersOnScreen {
                community: community.anchor.clone(),
                members: vec![creator.clone()],
            },
        )))
        .await;
    assert!(
        shows(
            &community,
            reader,
            &creator,
            BeatState::Online,
            Duration::from_secs(30)
        )
        .await,
        "on screen, node {reader} reads the creator's beat at once"
    );
    // Beating on: still online half an hour later.
    tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    assert_eq!(shown(&community, reader, &creator), BeatState::Online);
    // The creator's beats stop reaching anyone: offline at expiry.
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2, 3, 4, 5],
    });
    assert!(
        shows(
            &community,
            reader,
            &creator,
            BeatState::Offline,
            STALE_BOUND
        )
        .await,
        "node {reader} shows the silent creator offline within {STALE_BOUND:?}"
    );
    // Back: its next beat (at most 7.5 min) is inspected within a period.
    community.sim.fault(Fault::Heal);
    assert!(
        shows(
            &community,
            reader,
            &creator,
            BeatState::Online,
            Duration::from_secs(13 * 60 + 60)
        )
        .await,
        "node {reader} shows the creator online on its next beat"
    );
}
