//! Plan E3.4 S7, "History ad moves to `M`/5" (ADR 0011): a member's history
//! ad is written to its own member record, sealed under the community key,
//! and another member reads it from there, owner pinned to the member's
//! `MemberJoined` (`HostGovernance::member_history_ads`). Node 1 holds the
//! creator's post; the creator reads node 1's ad naming the channel.

use std::time::Duration;

use crate::harness::{eventually, Community};

/// Two ad checks (`AD_CHECK`, 150 s: one may pass before node 1 holds the
/// community key or the post) and the forced read at SimDht latency.
const BOUND: Duration = Duration::from_secs(6 * 60);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_members_history_ad_is_read_from_its_member_record() {
    let community = Community::of(2, 5207).await;
    community
        .post(0, "a post node 1 advertises")
        .await
        .expect("the creator posts");
    assert!(
        community
            .reads(1, "a post node 1 advertises", Duration::from_secs(120))
            .await,
        "node 1 holds the post"
    );
    let channel = community.general_id(0).await;
    let governance = community.nodes[0].host.governance();
    let read = eventually(&community.sim, BOUND, || async {
        governance
            .member_history_ads(&community.anchor)
            .await
            .iter()
            .flat_map(|(_, ads)| ads)
            .any(|ad| ad.channel_id == channel && ad.oldest_lamport <= ad.newest_lamport)
    })
    .await;
    assert!(
        read,
        "the creator reads node 1's history ad naming {channel} from its M/5 within {BOUND:?}: {:?}",
        governance.member_history_ads(&community.anchor).await
    );
}
