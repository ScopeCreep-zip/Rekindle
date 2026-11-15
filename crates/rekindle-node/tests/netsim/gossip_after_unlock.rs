//! Gossip waits for its overlay after an unlock (plan VU4, A/B round 1
//! R1-13): the unlock's stages that broadcast to a community's gossip mesh
//! (the community sync, presence) run only once that community's overlay
//! is registered, so no send finds it missing ("community or gossip
//! overlay missing", every ~10 s live), and the member's broadcasts reach
//! its peers.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{Community, PASSPHRASE};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_gossip_send_runs_before_the_overlay_after_an_unlock() {
    let community = Community::of(2, 181).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let member = &community.nodes[1];
    let key = member.public_key.clone().expect("an identity");
    member.ok(IpcRequest::Lock).await;
    member
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let missing = crate::logs::warned("overlay missing");
    assert!(
        missing.is_empty(),
        "gossip ran before its overlay: {missing:#?}"
    );
    assert!(
        community.post(1, "after the unlock").await.is_ok(),
        "the member posts after the unlock"
    );
    assert!(
        community
            .reads(0, "after the unlock", Duration::from_secs(60))
            .await,
        "the creator reads it"
    );
}
