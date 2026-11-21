//! Plan E3.2 Verify: a join with nobody online. The join is self-sovereign
//! (ADR 0011; `evidence/slot-integrity.md` §4.4): the joiner reads the
//! anchor and its inviter's member record from the DHT, publishes its own
//! member record and its `MemberJoined`, and needs no member to answer. A
//! replica is a DHT store, not a member's storage, so the records stay
//! readable while every member is away (`rekindle-netsim` `records/replicas.rs`).
//! The community key needs a member (D11: the invite carries none), so it
//! arrives once one is back, as does the inviter's view of the joiner,
//! through the reconnect sweep (§4.3).

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community};

fn holds_member(community: &Community, i: usize, pseudonym: &str) -> bool {
    (community.nodes[i].host.governance())
        .governance_state(&community.anchor)
        .is_some_and(|state| (state.members).contains_key(&PseudonymKey::from_hex_lossy(pseudonym)))
}

/// The only member is away (detached for `away`) when the joiner joins.
async fn join_while_alone(seed: u64, away: Duration) {
    let mut community = Community::create(2, seed).await;
    let creator = community.info(0).await["creator_pseudonyms"].clone();
    let link = community.invite().await;
    community.sim.quiesce().await;
    community.sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    tokio::time::sleep(away).await;

    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    let joiner = community.pseudonym(1).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            holds_member(&community, 1, &joiner)
        })
        .await,
        "the joiner's merge admits it with nobody online"
    );
    assert_eq!(community.info(1).await["creator_pseudonyms"], creator);

    community.sim.fault(Fault::Detach {
        node: 0,
        detached: false,
    });
    assert!(
        eventually(&community.sim, Duration::from_secs(300), || async {
            holds_member(&community, 0, &joiner)
        })
        .await,
        "the returning member learns the join"
    );
    let keyed = eventually(&community.sim, Duration::from_secs(300), || async {
        let held = community.mek_list(1).await;
        held.as_array().is_some_and(|keys| !keys.is_empty())
            || held.as_object().is_some_and(|keys| !keys.is_empty())
    })
    .await;
    assert!(keyed, "the joiner holds the key once a member is back");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_with_nobody_online_completes() {
    join_while_alone(4302, Duration::from_secs(30)).await;
}

/// The same after a day with every member away: nothing a member keeps
/// alive is needed (0.5.8 records carry no expiry the app must refresh;
/// the netsim's stores evict only by count, `records/stores.rs`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_join_after_a_day_with_nobody_online_completes() {
    join_while_alone(4303, Duration::from_secs(24 * 3600)).await;
}
