//! Plan E3.2 S7 Verify (D15, register S7d): a restored backup equivocates.
//! The creator's data root is copied, it creates a channel (an op at its
//! next seq), the copy is put back, and it creates a different channel at
//! that same seq. No key is touched: the fork is what a real restore does.
//! The other members detect it within one anti-entropy sweep and drop
//! every op of that author from the fork on, the later ones too.

use std::time::Duration;

use rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD;
use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};

use crate::harness::{eventually, Community, PASSPHRASE};

const SEEN: Duration = Duration::from_secs(180);

async fn create_channel(community: &Community, i: usize, name: &str) {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.governance_key.clone(),
                name: name.into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
}

async fn shows(community: &Community, i: usize, name: &str) -> bool {
    community.info(i).await.to_string().contains(name)
}

async fn unlock(community: &Community, i: usize) {
    let node = &community.nodes[i];
    node.ok(IpcRequest::Unlock {
        identity: node.public_key.clone().expect("an identity"),
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_restored_backup_equivocates_and_its_later_ops_are_quarantined() {
    let community = Community::of(3, 1515).await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    community.nodes[0].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.nodes[0]);
    unlock(&community, 0).await;
    create_channel(&community, 0, "alpha").await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            shows(&community, 1, "alpha").await && shows(&community, 2, "alpha").await
        })
        .await,
        "the members see the first branch"
    );

    community.nodes[0].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.nodes[0], &backup);
    unlock(&community, 0).await;
    create_channel(&community, 0, "bravo").await;
    // One sweep period plus the time an op takes to land everywhere.
    let detected = eventually(&community.sim, SWEEP_PERIOD + SEEN, || async {
        let mut clean = true;
        for i in 1..3 {
            clean &= !shows(&community, i, "alpha").await && !shows(&community, i, "bravo").await;
        }
        clean
    })
    .await;
    assert!(
        detected,
        "both members drop the creator's ops from the fork on"
    );
    let recorded = crate::logs::warned("governance equivocation");
    for i in 1..3 {
        let node = format!("node Some({i})");
        assert!(
            recorded.iter().any(|line| line.starts_with(&node)),
            "member {i} records the equivocation: {recorded:#?}"
        );
    }

    create_channel(&community, 0, "charlie").await;
    tokio::time::sleep(SWEEP_PERIOD * 2).await;
    for i in 1..3 {
        assert!(
            !shows(&community, i, "charlie").await,
            "member {i} quarantines the equivocator's later op"
        );
    }
}
