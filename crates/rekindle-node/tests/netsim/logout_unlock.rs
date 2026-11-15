//! Logout and the unlocks around it are ordered (plan VU4, A/B round 1
//! R1-2): logout stops the previous unlock's stages still running (own
//! records, prekeys, presence, the community stage, roster and op-log
//! reads) before it closes the ended session's records, and the next
//! unlock starts only after that closer ends. A member locks, unlocks,
//! logs out at once and unlocks again (twice), with Veilid's DHT latency;
//! then its own records are held writable and no stage logged an error.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::harness::{eventually, Community, PASSPHRASE};

/// What a stage that outlived its unlock logs (live R1-2).
const STRAYS: [&str; 3] = [
    "record pool is shut down",
    "transport node not started",
    "not held writable",
];

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn logout_right_after_unlock_leaves_the_next_unlock_whole() {
    let community = Community::of(2, 171).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let member = &community.nodes[1];
    let key = member.public_key.clone().expect("an identity");
    let unlock = || IpcRequest::Unlock {
        identity: key.clone(),
        passphrase: PASSPHRASE.into(),
    };
    member.ok(IpcRequest::Lock).await;
    // Logout at once (a watch re-arm of the ended pool was due), then half
    // a second in (the community stage's presence write in flight).
    for pause in [0, 500] {
        member.ok(unlock()).await;
        tokio::time::sleep(Duration::from_millis(pause)).await;
        member.ok(IpcRequest::Logout).await;
    }
    member.ok(unlock()).await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            member
                .call(IpcRequest::ProfileSetNickname {
                    display_name: "renamed".into(),
                })
                .await
                .is_ok()
        })
        .await,
        "the member's profile is writable after the second unlock"
    );
    tokio::time::sleep(Duration::from_secs(60)).await;
    let seen: Vec<String> = STRAYS.iter().flat_map(|s| crate::logs::warned(s)).collect();
    assert!(seen.is_empty(), "a stage outlived its unlock: {seen:#?}");
}
