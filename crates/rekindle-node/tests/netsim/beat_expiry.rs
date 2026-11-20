//! Plan E3.4 S3, the beat view and its local expiry: a member whose beats
//! stop (cut off from the network, so nothing new is written) shows
//! offline on every other member within 15 minutes of its last beat, and a
//! beat number an hour ahead of the reader's clock counts from its arrival
//! (Q4 as amended 2026-11-19), so it does not keep its member live for an
//! hour and a quarter. What a member shows is read from each reader's Host
//! (`HostGovernance::member_beat_shown`), the source of the
//! `CommunityMemberChanged` events the member list renders.

use std::time::Duration;

use rekindle_codec::member_record::BeatState;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;

use crate::harness::{eventually, Community};

/// The 15-minute rule plus the expiry's 30 s check and SimDht latency.
const STALE_BOUND: Duration = Duration::from_secs(15 * 60 + 60);

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
async fn a_member_whose_beats_stop_shows_offline_on_every_member() {
    let community = Community::of(3, 4310).await;
    let creator = community.pseudonym(0).await;
    for reader in [1, 2] {
        assert!(
            shows(
                &community,
                reader,
                &creator,
                BeatState::Online,
                Duration::from_secs(600)
            )
            .await,
            "node {reader} shows the creator online from its beats"
        );
    }
    // The creator's beats stop reaching anyone: no new write lands.
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    // Still online well inside the 15 minutes.
    tokio::time::sleep(Duration::from_secs(60)).await;
    for reader in [1, 2] {
        assert_eq!(shown(&community, reader, &creator), BeatState::Online);
    }
    for reader in [1, 2] {
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
    }
    community.sim.fault(Fault::Heal);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_beat_number_an_hour_ahead_counts_from_its_arrival() {
    let community = Community::of(2, 4311).await;
    let creator = community.pseudonym(0).await;
    assert!(
        shows(
            &community,
            1,
            &creator,
            BeatState::Online,
            Duration::from_secs(600)
        )
        .await,
        "node 1 shows the creator online"
    );
    // The creator's clock runs an hour ahead: its next beat's number is an
    // hour past node 1's clock. A picked status writes it at once.
    community.sim.fault(Fault::ClockSkew {
        node: 0,
        ms: 3_600_000,
    });
    community.nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Busy,
        })
        .await;
    assert!(
        shows(
            &community,
            1,
            &creator,
            BeatState::Busy,
            Duration::from_secs(60)
        )
        .await,
        "node 1 takes the creator's hour-ahead beat through its ring watch"
    );
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    assert!(
        shows(&community, 1, &creator, BeatState::Offline, STALE_BOUND).await,
        "the hour-ahead beat counts from its arrival: offline within {STALE_BOUND:?}, \
         not an hour and a quarter later"
    );
    community.sim.fault(Fault::Heal);
}

/// Plan E3.4 "Constant rate": a quit writes an explicit `Offline` beat and
/// gossips its `HeadAdvanced` hint, so every member shows the quitter
/// offline within one gossip round (a hop and the beat's read), not 15
/// minutes later.
const GOSSIP_ROUND: Duration = Duration::from_secs(3);

/// How the creator ends its session in a quit scenario.
#[derive(Clone, Copy)]
enum Quit {
    /// A lock or logout over the bus.
    Lock,
    /// The Host's exit, as a stop signal (SIGTERM, SIGINT,
    /// `systemctl stop`) or the shell's quit runs it.
    Exit,
}

async fn quit_is_seen_within_one_gossip_round(seed: u64, quit: Quit) {
    let community = Community::of(3, seed).await;
    let creator = community.pseudonym(0).await;
    for reader in [1, 2] {
        assert!(
            shows(
                &community,
                reader,
                &creator,
                BeatState::Online,
                Duration::from_secs(600)
            )
            .await,
            "node {reader} shows the creator online"
        );
    }
    match quit {
        Quit::Lock => {
            community.nodes[0].ok(IpcRequest::Lock).await;
        }
        Quit::Exit => {
            community.nodes[0]
                .host
                .shutdown()
                .await
                .expect("the host stops");
        }
    }
    for reader in [1, 2] {
        assert!(
            shows(
                &community,
                reader,
                &creator,
                BeatState::Offline,
                GOSSIP_ROUND
            )
            .await,
            "node {reader} shows the quit creator offline within {GOSSIP_ROUND:?}"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_quit_is_seen_within_one_gossip_round() {
    quit_is_seen_within_one_gossip_round(4312, Quit::Lock).await;
}

/// The same quit through the Host's exit: the gossip worker carries the
/// teardown's hint, so it stops after the teardown, not at the stop
/// request (Linux W5b: every signal stop left its peers showing it online).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_exit_is_seen_within_one_gossip_round() {
    quit_is_seen_within_one_gossip_round(4313, Quit::Exit).await;
}
