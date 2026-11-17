//! Live A/B finding F7-4 (2026-11-17): a member that left sends nothing for
//! the community it left. Its teardown removes the community and its gossip
//! overlay, so the remaining members' gossip that still reaches it (their
//! peer lists hold it until their next refresh) is neither relayed nor sent
//! anywhere: no send looks for the overlay ("community or gossip overlay
//! missing"), as Signal's left group sends nothing.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use crate::harness::Community;

/// The leaver's gossip sends for the community so far.
fn sends(community: &Community) -> Vec<String> {
    crate::logs::logged(1, &["send_to_mesh", community.governance_key.as_str()])
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_that_left_sends_nothing_for_the_community() {
    let community = Community::of(3, 331).await;
    community
        .post(0, "ann before the leave")
        .await
        .expect("the creator posts");
    assert!(
        community
            .reads(1, "ann before the leave", Duration::from_secs(120))
            .await,
        "the member reads the creator's post"
    );
    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    community.sim.quiesce().await;
    let at_teardown = sends(&community).len();

    // The remaining members keep gossiping (posts, beats, presence) for two
    // minutes of sim time.
    for i in 0..4 {
        community
            .post(0, &format!("ann after the leave {i}"))
            .await
            .expect("the creator posts");
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
    community.sim.quiesce().await;

    let after = sends(&community);
    assert_eq!(
        after.len(),
        at_teardown,
        "the leaver sends nothing for the community after its teardown: {:#?}",
        &after[at_teardown..]
    );
    let missing = crate::logs::warned("overlay missing");
    assert!(
        missing.is_empty(),
        "no send looks for the left community's overlay: {missing:#?}"
    );
}
