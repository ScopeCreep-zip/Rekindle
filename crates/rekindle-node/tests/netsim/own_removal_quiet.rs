//! A/B findings R3-1 and R3-3 (plan E3.2 S9 own removal, amended
//! 2026-11-15 from research: Signal's inactive group sends nothing). A
//! kicked member's node sends nothing for the community it was removed
//! from: no gossip of its own and no relay of the remover's, so no send
//! finds the overlay its teardown removed ("community or gossip overlay
//! missing"), and a relogin brings none of it back. It is not an eligible
//! rotator (ADR 0018), so it never evaluates the departure rotation.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};

use crate::harness::{eventually, Community, PASSPHRASE};

/// The kicked node's gossip sends for the community so far, and the sends
/// the gossip worker's backstop refused for it (a sender that did not
/// check the removal itself).
fn sends(community: &Community) -> Vec<String> {
    let key = community.anchor.as_str();
    let mut sends = crate::logs::logged(1, &["send_to_mesh", key]);
    sends.extend(crate::logs::logged(1, &["nothing sent for a removed", key]));
    sends
}

/// The kicked node's departure-rotation evaluations so far.
fn rotations() -> Vec<String> {
    crate::logs::logged(1, &["departure rotation"])
}

/// The creator posts four times, 30 s apart: the remaining members keep
/// gossiping (posts, beats) for two minutes of sim time.
async fn two_minutes_of_gossip(community: &Community, label: &str) {
    for i in 0..4 {
        community
            .post(0, &format!("ann {label} {i}"))
            .await
            .expect("the creator posts");
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
    community.sim.quiesce().await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kicked_member_sends_nothing_for_the_community() {
    let community = Community::of(3, 167).await;
    let ann = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    community
        .post(0, "ann before the kick")
        .await
        .expect("the creator posts");
    assert!(
        community
            .reads(1, "ann before the kick", Duration::from_secs(120))
            .await,
        "the member reads the creator's post"
    );
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: bea,
                reason: None,
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community.info(1).await["removed"]["by"].as_str() == Some(ann.as_str())
        })
        .await,
        "the member learns its removal: {}",
        community.info(1).await
    );
    community.sim.quiesce().await;
    let at_teardown = sends(&community).len();

    two_minutes_of_gossip(&community, "after the kick").await;
    let after_kick = sends(&community);
    assert_eq!(
        after_kick.len(),
        at_teardown,
        "the kicked node sends nothing for the community after its teardown: {:#?}",
        &after_kick[at_teardown..]
    );

    let key = community.nodes[1].public_key.clone().expect("an identity");
    community.nodes[1].ok(IpcRequest::Lock).await;
    community.nodes[1]
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    two_minutes_of_gossip(&community, "after the relogin").await;
    let after_relogin = sends(&community);
    assert_eq!(
        after_relogin.len(),
        at_teardown,
        "a relogin restores no send for the removed community: {:#?}",
        &after_relogin[at_teardown..]
    );
    let missing = crate::logs::warned("overlay missing");
    assert!(
        missing.is_empty(),
        "no send looks for the removed community's overlay: {missing:#?}"
    );
    assert!(
        rotations().is_empty(),
        "a removed member never evaluates the departure rotation: {:#?}",
        rotations()
    );
}
