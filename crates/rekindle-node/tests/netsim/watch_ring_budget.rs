//! Plan E3.2 S7 Verify: with N = 40 members every member record `M` is
//! watched by its k = 4 ring predecessors and nobody else, under Veilid's
//! `public_watch_limit` (32), and no replica refuses a watch.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use rekindle_types::dht_layout::member_record::SUBKEY_COUNT;

use crate::harness::{eventually, Community};

const MEMBERS: usize = 40;

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
    // Each node's merged member count and the records it watches, so a run
    // that does not converge says whether the member sets or the ring
    // differ.
    let mut nodes = Vec::with_capacity(MEMBERS);
    for i in 0..MEMBERS {
        let members = community.info(i).await["memberCount"].clone();
        let mine: Vec<&String> = watched
            .iter()
            .filter(|(_, nodes)| nodes.contains(&i))
            .map(|(record, _)| record)
            .collect();
        nodes.push(format!(
            "node {i}: members {members}, watches {}",
            mine.len()
        ));
    }
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
