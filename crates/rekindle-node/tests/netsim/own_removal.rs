//! A/B finding R1-1 (plan E3.2 S9 own removal): a kicked member reacts to
//! its own removal once it merges the `MemberRemoved` naming it. It
//! announces `MembershipEvent::Kicked` naming the remover, marks the
//! community removed in `community info` (by whom), forgets the
//! community's keys, refuses its own sends, and keeps its history; the
//! other members see nothing new from it. A/B finding R3-2: the history it
//! keeps stays readable as it was (no row it opened turns undecryptable,
//! after the kick or after a relogin), and nothing posted after the kick
//! is opened.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, PASSPHRASE};
use crate::moderation::listed;

/// Why the creator kicks the member (A/B R5-2).
const REASON: &str = "ab5 test";

/// The posts the member read before its removal.
const KEPT: [&str; 2] = ["bea before the kick", "ann before the kick"];

/// Whether node `i`'s history holds every body of `bodies` opened
/// (`decryptionFailed=false`) within `limit`.
async fn opened(community: &Community, i: usize, bodies: &[&str], limit: Duration) -> bool {
    eventually(&community.sim, limit, || async {
        community.history(i, None, 100).await.is_ok_and(|page| {
            bodies.iter().all(|body| {
                page.iter()
                    .any(|m| m["body"] == *body && m["decryptionFailed"] == false)
            })
        })
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kicked_member_leaves_locally_and_keeps_its_history() {
    let community = Community::of(3, 163).await;
    let ann = community.pseudonym(0).await;
    let bea = community.pseudonym(1).await;
    let bea_events = community.nodes[1].events().await;
    community
        .post(1, "bea before the kick")
        .await
        .expect("a member posts");
    assert!(
        community
            .reads(0, "bea before the kick", Duration::from_secs(120))
            .await,
        "the creator reads the member's post"
    );
    community
        .post(0, "ann before the kick")
        .await
        .expect("the creator posts");
    assert!(
        opened(&community, 1, &KEPT, Duration::from_secs(120)).await,
        "the member opens both posts before the kick: {:?}",
        community.history(1, None, 100).await
    );
    assert!(
        !community
            .mek_list(1)
            .await
            .as_array()
            .is_none_or(Vec::is_empty),
        "the member holds the community's keys before the kick"
    );

    // The name the member knows the creator by before the kick.
    let ann_name = listed(&community, 1, &ann)
        .await
        .and_then(|m| m["displayName"].as_str().map(str::to_owned))
        .filter(|name| !name.is_empty());
    assert!(ann_name.is_some(), "the member lists the creator by name");

    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                reason: Some(REASON.into()),
            },
        )))
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(180), || async {
            community.info(1).await["removed"]["by"].as_str() == Some(ann.as_str())
        })
        .await,
        "the kicked member's info shows it removed by the creator: {}",
        community.info(1).await
    );
    community.sim.quiesce().await;
    assert!(
        bea_events.any(|e| matches!(
            e,
            SubscriptionEvent::Membership(MembershipEvent::Kicked {
                target_pseudonym,
                removed_by: Some(by),
                reason: Some(reason),
                ..
            }) if *target_pseudonym == bea && *by == ann && reason == REASON
        )),
        "the kicked member announces its own removal, naming the remover and the reason"
    );
    assert_eq!(
        community.info(1).await["removed"]["banned"].as_bool(),
        Some(false),
        "a kick, not a ban"
    );
    // A/B R5-2 (R3-5): the banner names who removed us and why, from the
    // name the member knew the creator by when the kick merged.
    let removed = &community.info(1).await["removed"];
    assert!(
        removed["byName"].is_string() && removed["byName"].as_str() == ann_name.as_deref(),
        "the kicked member's info names the remover ({ann_name:?}): {removed}"
    );
    assert_eq!(removed["reason"].as_str(), Some(REASON), "{removed}");
    assert!(
        eventually(&community.sim, Duration::from_secs(60), || async {
            community
                .mek_list(1)
                .await
                .as_array()
                .is_none_or(Vec::is_empty)
        })
        .await,
        "the kicked member forgot the community's keys: {}",
        community.mek_list(1).await
    );
    let refused = community
        .post(1, "bea after the kick")
        .await
        .expect_err("a removed member's send is refused");
    assert_eq!(refused.code, 403, "{}", refused.message);
    assert!(
        community
            .history_holds(1, "bea before the kick", Duration::from_secs(30))
            .await,
        "the kicked member keeps its history"
    );
    assert!(
        opened(&community, 1, &KEPT, Duration::from_secs(1)).await,
        "the kicked member's kept history stays readable (R3-2): {:?}",
        community.history(1, None, 100).await
    );
    community
        .post(0, "ann after the kick")
        .await
        .expect("the creator posts after the kick");

    // Long past a presence beat and a keepalive round: the others see
    // nothing new of it.
    tokio::time::sleep(Duration::from_secs(120)).await;
    community.sim.quiesce().await;
    for reader in [0, 2] {
        assert!(
            listed(&community, reader, &bea).await.is_none(),
            "node {reader} does not list the kicked member again"
        );
        assert!(
            !community
                .reads(reader, "bea after the kick", Duration::from_secs(10))
                .await,
            "node {reader} shows nothing new of the kicked member"
        );
    }

    // A relogin reloads the vault's keys: the kept history reads as it was,
    // and the post made after the kick is still not opened.
    let bea_key = community.nodes[1].public_key.clone().expect("an identity");
    community.nodes[1].ok(IpcRequest::Lock).await;
    community.nodes[1]
        .ok(IpcRequest::Unlock {
            identity: bea_key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert!(
        opened(&community, 1, &KEPT, Duration::from_secs(120)).await,
        "the kept history stays readable after a relogin (R3-2): {:?}",
        community.history(1, None, 100).await
    );
    let removed = &community.info(1).await["removed"];
    assert!(
        removed["byName"].as_str() == ann_name.as_deref()
            && removed["reason"].as_str() == Some(REASON),
        "the remover's name and the reason survive a relogin (R5-2): {removed}"
    );
    assert!(
        community
            .reads(2, "ann after the kick", Duration::from_secs(60))
            .await,
        "a remaining member reads the post made after the kick"
    );
    community.sim.quiesce().await;
    assert!(
        !community
            .history_holds(1, "ann after the kick", Duration::from_secs(10))
            .await,
        "nothing posted after the kick is opened by the kicked member"
    );
}
