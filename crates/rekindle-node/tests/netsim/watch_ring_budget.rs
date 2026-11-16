//! Plan E3.2 S7 Verify: with N = 40 members every member record `M` is
//! watched by its k = 4 ring predecessors and nobody else, under Veilid's
//! `public_watch_limit` (32), and no replica refuses a watch.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rekindle_types::dht_layout::member_record::SUBKEY_COUNT;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use crate::harness::{eventually, Community};

const MEMBERS: usize = 40;

/// One line per node: its `CommunityDetail.member_count`, its roster's
/// size, the node indices missing from its roster, and how many records it
/// watches.
async fn member_sets(
    community: &Community,
    watched: &BTreeMap<String, BTreeSet<usize>>,
) -> Vec<String> {
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut rosters: Vec<BTreeSet<String>> = Vec::with_capacity(MEMBERS);
    for i in 0..MEMBERS {
        index.insert(community.pseudonym(i).await, i);
        let roster = community.nodes[i]
            .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityMembers {
                    governance_key: community.governance_key.clone(),
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
    let mut lines = Vec::with_capacity(MEMBERS);
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

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn forty_members_watch_each_member_record_four_times_and_none_is_refused() {
    // One invite for everyone: an inviter holds at most 8 live invites
    // (`invite_quota::MAX_ACTIVE_INVITES_PER_INVITER`).
    let mut community = Community::create(MEMBERS, 4040).await;
    let uses = u32::try_from(MEMBERS - 1).expect("a small count");
    let link = community.invite_for(uses).await;
    for i in 1..MEMBERS {
        community.join_with(i, link.clone()).await;
    }
    let converged = eventually(&community.sim, Duration::from_secs(600), || async {
        let watched = census(&community);
        watched.len() == MEMBERS && watched.values().all(|nodes| nodes.len() == 4)
    })
    .await;
    let watched = census(&community);
    let counts: Vec<usize> = watched.values().map(BTreeSet::len).collect();
    // Each node's member count, the nodes its roster lacks against the
    // union of every roster, and the records it watches, so a run that
    // does not converge says whether the member sets or the ring differ.
    let nodes = member_sets(&community, &watched).await;
    assert!(
        converged,
        "every M has 4 ring watchers: {counts:?}\n{}",
        nodes.join("\n")
    );
    assert!(counts.iter().all(|n| *n <= 4), "{counts:?}");
    assert_eq!(
        community.sim.watch_refusals(),
        0,
        "no replica refused a watch"
    );
}
