//! Plan E3.2 S6 open item: `receive_bounded` (`op_log.rs`) refuses a
//! gossiped `MemberJoined` from an author the receiver does not hold as a
//! member unless its invite proof names one of the receiver's live
//! `InviteCreated` codes (plan E3.2: readers check `inviteProof` against
//! the community's `InviteCreated` set; every admission mode needs an
//! invite), so a key with no invite leaves no row behind.
//!
//! A kicked member's re-join, at a member that holds the kick but not the
//! fresh `InviteCreated` (the owner's gossip of it was lost and her DHT
//! calls failed), never reaches that bound: gossip authored by a departed
//! member is dropped at ingest (`DaemonHandler::takes_traffic`,
//! `CommunityStore::takes_traffic_from`: `GovernanceState::is_departed`),
//! and a removed author is departed, so nothing of the re-join is stored
//! there by gossip, whichever path brings it. The bound's invite check
//! bites only for an author the receiver never held. The member converges
//! through the consistent path (the three-path delivery of
//! `rekindle-communities-architecture.md`): once her DHT is back, her
//! anti-entropy sweep
//! (`anti_entropy::sweep`, every `SWEEP_PERIOD`, and in full on reconnect)
//! reads the owner's frontier, which the owner republished after merging
//! the re-join (`republish_own_frontier`), and fetches the re-join and the
//! invite by their `OpRef`s, within one sweep, as a Matrix server that
//! missed an event fetches it with its auth chain (`/get_missing_events`).

use std::time::Duration;

use rekindle_codec::governance_op::decode_op;
use rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD;
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community};
use crate::moderation::listed;

const LIMIT: Duration = Duration::from_secs(180);

/// How many `MemberJoined` ops by `who` node `i`'s op store holds.
async fn joins_held(community: &Community, i: usize, who: &PseudonymKey) -> usize {
    let held = (community.nodes[i].host.governance())
        .governance_ops(&community.anchor)
        .await
        .expect("the op store");
    held.iter()
        .filter(|op| op.author == *who)
        .filter_map(|op| decode_op(&op.encoded).ok())
        .filter(|d| matches!(d.op.body, GovernanceEntry::MemberJoined { .. }))
        .count()
}

/// Whether node `i`'s merged state holds `who` as removed and not a member.
fn holds_kick(community: &Community, i: usize, who: &PseudonymKey) -> bool {
    (community.nodes[i].host.governance())
        .governance_state(&community.anchor)
        .is_some_and(|s| s.removed.contains_key(who) && !s.members.contains_key(who))
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_rejoin_missed_by_gossip_converges_by_the_sweep() {
    let mut community = Community::create(3, 359).await;
    community.join(2).await;
    community.join(1).await;
    let bea = community.pseudonym(1).await;
    let bea_key = PseudonymKey::from_hex_lossy(&bea);
    assert!(
        eventually(&community.sim, LIMIT, || async {
            listed(&community, 2, &bea).await.is_some()
        })
        .await,
        "carol lists bea before the kick"
    );

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: bea.clone(),
                reason: None,
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            holds_kick(&community, 2, &bea_key) && community.info(1).await["removed"].is_object()
        })
        .await,
        "carol and bea merge the kick"
    );
    community.sim.quiesce().await;

    // The fresh invite does not reach carol: the owner's gossip of it is
    // lost, and carol's DHT calls fail (no watch report, no sweep, no
    // fetch), while app messages still reach her. Bea's join reads the
    // invite from the owner's member record.
    community.sim.fault(Fault::DhtOffline {
        node: 2,
        offline: true,
    });
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 1000,
    });
    let link = community.invite().await;
    community.sim.fault(Fault::DropFrom {
        node: 0,
        per_mille: 0,
    });
    community.join_with(1, link).await;
    assert!(
        community.info(1).await["removed"].is_null(),
        "bea re-joined: {}",
        community.info(1).await
    );
    assert!(
        eventually(&community.sim, LIMIT, || async {
            listed(&community, 0, &bea).await.is_some()
        })
        .await,
        "the owner lists bea again"
    );
    community.sim.quiesce().await;

    // Carol holds the kick and not the invite: bea's gossip is dropped at
    // her ingest, and with no DHT she cannot fetch the re-join.
    assert!(
        holds_kick(&community, 2, &bea_key),
        "carol still holds the kick"
    );
    assert_eq!(
        joins_held(&community, 2, &bea_key).await,
        1,
        "carol stored only bea's first join: the re-join is not taken from gossip"
    );
    assert!(listed(&community, 2, &bea).await.is_none());

    // Carol's DHT is back: within one sweep carol fetches the re-join from
    // the owner's frontier, with the invite it cites.
    community.sim.fault(Fault::Heal);
    let converged = eventually(&community.sim, SWEEP_PERIOD + SWEEP_PERIOD / 2, || async {
        listed(&community, 2, &bea).await.is_some()
            && joins_held(&community, 2, &bea_key).await == 2
    })
    .await;
    assert!(converged, "carol fetches the re-join by the sweep");
    let state = (community.nodes[2].host.governance())
        .governance_state(&community.anchor)
        .expect("a merged governance state");
    assert!(
        state.members.contains_key(&bea_key) && !state.is_departed(&bea_key),
        "carol: bea is a member, not departed"
    );

    community
        .post(1, "back, seen by carol")
        .await
        .expect("the re-joiner posts");
    assert!(
        community.reads(2, "back, seen by carol", LIMIT).await,
        "carol reads the re-joiner's post"
    );
}
