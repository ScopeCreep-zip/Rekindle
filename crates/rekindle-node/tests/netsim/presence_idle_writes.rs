//! Plan E3.4 S5, IBS "Owed": an owner and a joiner that sit idle write
//! their presence rows about once a minute each (the poll's 60 s beat,
//! D4), never in a ping-pong. A registry change is answered by the Host's
//! registry arm (`registry_watch`) with a refresh that writes nothing; a
//! reaction that wrote our row made each member's write fire the other's
//! registry watch, whose write fired the first's again (finding IB1c, the
//! Linux round-7 storm, ~45 writes a minute on both sides).

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use crate::harness::Community;

/// The registry's subkey count (`REGISTRY_SLOTS`, one row per member).
const REGISTRY_SLOTS: u32 = 255;

/// How long the members sit idle, in simulated minutes.
const IDLE_MINUTES: u64 = 10;

/// The community's member registry key, from the creator's community view.
async fn registry_key(community: &Community) -> String {
    let views = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        )))
        .await;
    views
        .as_array()
        .and_then(|views| {
            views
                .iter()
                .find(|v| v["anchor"].as_str() == Some(community.anchor.as_str()))
        })
        .and_then(|v| v["memberRegistryKey"].as_str())
        .expect("the community's registry key")
        .to_owned()
}

/// Every write that landed on the registry so far: the sum of its rows'
/// seqs (each landed write moves its row's seq by one).
fn registry_writes(community: &Community, registry: &str) -> u64 {
    (0..REGISTRY_SLOTS)
        .filter_map(|subkey| community.sim.network_get(registry, subkey))
        .map(|value| u64::from(value.seq))
        .sum()
}

/// Rows that hold a value: the members whose presence landed.
fn rows_written(community: &Community, registry: &str) -> usize {
    (0..REGISTRY_SLOTS)
        .filter(|subkey| community.sim.network_get(registry, *subkey).is_some())
        .count()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn idle_members_write_their_presence_about_once_a_minute() {
    let community = Community::of(2, 5150).await;
    let registry = registry_key(&community).await;
    // Past the poll's start-up burst (one tick, then six at 5 s) and the
    // join's own writes.
    tokio::time::sleep(Duration::from_secs(3 * 60)).await;
    assert!(
        rows_written(&community, &registry) >= 2,
        "both members' rows landed in the registry"
    );
    let before = registry_writes(&community, &registry);
    tokio::time::sleep(Duration::from_secs(IDLE_MINUTES * 60)).await;
    let writes = registry_writes(&community, &registry) - before;
    // Each of the two writes once a minute; a beat that straddles the
    // window's edges adds at most one each. The storm wrote ~45 a minute.
    let most = 2 * (IDLE_MINUTES + 1);
    let least = 2 * (IDLE_MINUTES - 1);
    assert!(
        writes <= most,
        "{writes} registry writes in {IDLE_MINUTES} idle minutes (at most {most}): a watch reaction writes"
    );
    assert!(
        writes >= least,
        "{writes} registry writes in {IDLE_MINUTES} idle minutes (at least {least}): both members beat"
    );
}
