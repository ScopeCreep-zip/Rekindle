//! Plan E3.4 S2, the beat engine: a member's scheduled beats (every 2.5 to
//! 7.5 minutes) reach another member's read of its record, a picked status
//! is written at once, and Invisible writes one `Offline` beat and nothing
//! after it. The reader is node 1, the creator's ring watcher, reading the
//! creator's `M`/1 through its own record port, pinned to the owner its
//! `MemberJoined` names (`member_card::read_beat`).

use std::time::Duration;

use rekindle_codec::member_record::{BeatState, PresenceBeat};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::member_card::{read_beat, MemberRecordRef};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::presence::SessionStatus;

use crate::harness::{eventually, Community};
use crate::watch_ring::creator_record;

/// The creator's beat as node `reader` reads it now.
async fn creator_beat(community: &Community, reader: usize) -> Option<PresenceBeat> {
    let governance = community.nodes[reader].host.governance();
    let state = governance.governance_state(&community.anchor)?;
    let creator = creator_record(community);
    let (pseudonym, member) = (state.members.iter()).find(|(_, m)| m.member_record == creator)?;
    let port = governance.record_port().ok()?;
    let member = MemberRecordRef {
        community_id: &community.anchor,
        pseudonym,
        record: &member.member_record,
        owner: &member.member_record_owner,
    };
    read_beat(&*port, &member).await.ok().flatten()
}

/// The creator's beat once node 1 reads it in `state`, within `bound`.
pub(crate) async fn reads_state(
    community: &Community,
    state: BeatState,
    bound: Duration,
) -> PresenceBeat {
    let beat = std::cell::RefCell::new(None);
    let read = eventually(&community.sim, bound, || async {
        let read = creator_beat(community, 1)
            .await
            .filter(|b| b.state == state);
        let found = read.is_some();
        *beat.borrow_mut() = read;
        found
    })
    .await;
    assert!(
        read,
        "node 1 reads the creator's {state:?} beat within {bound:?}"
    );
    beat.into_inner().expect("read")
}

/// Well under the shortest scheduled delay (2.5 min): a picked status's
/// write and node 1's forced read, at SimDht latency.
pub(crate) const AT_ONCE: Duration = Duration::from_secs(60);

pub(crate) async fn set_status(community: &Community, node: usize, status: SessionStatus) {
    community.nodes[node]
        .ok(IpcRequest::PresenceSet { status })
        .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn beats_follow_the_schedule_a_picked_status_and_invisible() {
    let community = Community::of(2, 4208).await;
    let first = reads_state(&community, BeatState::Online, Duration::from_secs(300)).await;

    // The engine schedules a community within 2.5 minutes of its join and
    // draws each delay from 2.5 to 7.5 minutes: 11 minutes on, node 1
    // reads a newer beat than the join's.
    tokio::time::sleep(Duration::from_secs(11 * 60)).await;
    let scheduled = creator_beat(&community, 1).await.expect("a beat");
    assert!(
        scheduled.seq > first.seq,
        "{} > {}",
        scheduled.seq,
        first.seq
    );
    assert!(scheduled.beat_number >= first.beat_number);
    assert_eq!(scheduled.state, BeatState::Online);

    // A picked status is written at once, not at the next scheduled beat.
    set_status(&community, 0, SessionStatus::Busy).await;
    reads_state(&community, BeatState::Busy, AT_ONCE).await;

    // Invisible: one Offline beat, then nothing for half an hour.
    set_status(&community, 0, SessionStatus::Invisible).await;
    let hidden = reads_state(&community, BeatState::Offline, AT_ONCE).await;
    tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    let later = creator_beat(&community, 1).await.expect("a beat");
    assert_eq!(later.seq, hidden.seq, "an Invisible member wrote a beat");
}
