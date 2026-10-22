//! No app polling of a watched record (D4, plan E3.1 C1): over a two-minute
//! idle nobody inspects a community's governance record; a change reaches
//! its members through their watches alone.

use std::time::Duration;

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_idle_community_is_never_inspected() {
    let community = Community::of(2, 139).await;
    // The join's own reads settle first.
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.as_str();
    let before: Vec<u64> = (0..2).map(|i| community.sim.app_inspects(i, key)).collect();
    tokio::time::sleep(Duration::from_secs(120)).await;
    for (i, was) in before.into_iter().enumerate() {
        assert_eq!(
            community.sim.app_inspects(i, key),
            was,
            "node {i} inspected the governance record while idle"
        );
    }
}
