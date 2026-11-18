//! Plan E3.2 "Watching" (`evidence/slot-integrity.md` §4.5): with the
//! members joined and merged, every member record `M` is watched by its
//! `min(k, N − 1)` ring predecessors and nobody else, and no replica
//! refuses a watch. Six members here (each `M` has 4 watchers, the creator's
//! included); `watch_ring_budget` runs N = 40 in the scale target.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use rekindle_types::dht_layout::community_anchor;
use rekindle_types::dht_layout::member_record::SUBKEY_COUNT;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use crate::harness::{eventually, Community};
use crate::ring_census;

/// One line per node: its `CommunityDetail.member_count`, its roster's
/// size, the node indices missing from its roster, and how many records it
/// watches.
async fn member_sets(
    community: &Community,
    members: usize,
    watched: &BTreeMap<String, BTreeSet<usize>>,
) -> Vec<String> {
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut rosters: Vec<BTreeSet<String>> = Vec::with_capacity(members);
    for i in 0..members {
        index.insert(community.pseudonym(i).await, i);
        let roster = community.nodes[i]
            .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityMembers {
                    anchor: community.anchor.clone(),
                },
            )))
            .await;
        rosters.push(
            roster
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["pseudonymKey"].as_str().map(str::to_owned))
                .collect(),
        );
    }
    let union: BTreeSet<String> = rosters.iter().flatten().cloned().collect();
    let mut lines = Vec::with_capacity(members);
    for (i, roster) in rosters.iter().enumerate() {
        let count = community.info(i).await["member_count"].clone();
        let missing: Vec<String> = union
            .difference(roster)
            .map(|p| {
                index
                    .get(p)
                    .map_or_else(|| p[..8].to_owned(), usize::to_string)
            })
            .collect();
        let watches = watched.values().filter(|n| n.contains(&i)).count();
        lines.push(format!(
            "node {i}: member_count {count}, roster {}, missing {missing:?}, watches {watches}",
            roster.len()
        ));
    }
    lines
}

fn census(community: &Community) -> BTreeMap<String, BTreeSet<usize>> {
    community.sim.watchers_of_dflt(SUBKEY_COUNT)
}

/// `members` join through one invite; then every `M` converges to
/// `min(4, members − 1)` ring watchers, and no replica refuses a watch.
pub async fn ring_converges(members: usize, seed: u64) {
    // One invite for everyone: an inviter holds at most 8 live invites
    // (`invite_quota::MAX_ACTIVE_INVITES_PER_INVITER`).
    let mut community = Community::create(members, seed).await;
    let uses = u32::try_from(members - 1).expect("a small count");
    let link = community.invite_for(uses).await;
    let want = 4.min(members - 1);
    let last = Arc::new(Mutex::new(String::from("census: none yet")));
    let done = ring_census::guard(Arc::clone(&last));
    let run = async {
        for i in 1..members {
            community.join_with(i, link.clone()).await;
        }
        eventually(&community.sim, Duration::from_secs(600), || async {
            let watched = census(&community);
            watched.len() == members && watched.values().all(|nodes| nodes.len() == want)
        })
        .await
    };
    let converged = tokio::select! {
        converged = run => converged,
        () = ring_census::every_minute(&community, Arc::clone(&last)) => unreachable!("the census never ends"),
    };
    drop(done);
    let watched = census(&community);
    let counts: Vec<usize> = watched.values().map(BTreeSet::len).collect();
    // Each node's member count, the nodes its roster lacks against the
    // union of every roster, and the records it watches, so a run that
    // does not converge says whether the member sets or the ring differ.
    let nodes = member_sets(&community, members, &watched).await;
    assert!(
        converged,
        "every M has {want} ring watchers: {counts:?}\n{}",
        nodes.join("\n")
    );
    assert!(counts.iter().all(|n| *n <= want), "{counts:?}");
    assert_eq!(
        community.sim.watch_refusals(),
        0,
        "no replica refused a watch"
    );
    // The creator's own `M`, the GenesisCert's `creatorMemberRecord`, is
    // one of the watched records and has its full ring of watchers.
    let creator = creator_record(&community);
    assert_eq!(
        watched.get(&creator).map(BTreeSet::len),
        Some(want),
        "the creator's M {creator} has {want} ring watchers: {watched:?}"
    );
}

/// The creator's member record as the anchor's GenesisCert names it.
pub fn creator_record(community: &Community) -> String {
    let cert = community
        .sim
        .network_get(&community.anchor, community_anchor::GENESIS_CERT)
        .expect("the anchor's GenesisCert");
    rekindle_codec::governance_op::genesis::decode_genesis(&cert.data)
        .expect("a GenesisCert")
        .signed
        .cert
        .creator_member_record
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn six_members_agree_on_the_ring_and_the_creator_is_in_it() {
    ring_converges(6, 4006).await;
}
