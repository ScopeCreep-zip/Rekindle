//! Live A/B finding (2026-11-17): a member that left re-joins with a fresh
//! invite under the same community pseudonym (its `MemberJoined` is a later
//! op of its own chain, following its `MemberLeft`). The inviter merges the
//! re-join through the bootstrap exchange, every member lists it again, it
//! receives the community key (D16) and every member reads its next post,
//! a member joining later too.

use std::time::Duration;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community};

const LIMIT: Duration = Duration::from_secs(180);

/// The pseudonyms node `i` lists as the community's members.
async fn members(community: &Community, i: usize) -> Vec<String> {
    community.nodes[i]
        .call(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityMembers {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .ok()
        .and_then(|v| v.as_array().cloned())
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r["pseudonymKey"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

async fn all_list(community: &Community, who: &str, listed: bool, nodes: &[usize]) -> bool {
    eventually(&community.sim, LIMIT, || async {
        let mut every = true;
        for &i in nodes {
            every &= members(community, i).await.iter().any(|p| p == who) == listed;
        }
        every
    })
    .await
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_that_left_rejoins_and_posts() {
    let mut community = Community::create(4, 317).await;
    community.join(1).await;
    community.join(2).await;
    let bea = community.pseudonym(1).await;
    assert!(
        all_list(&community, &bea, true, &[0, 2]).await,
        "every member lists bea"
    );
    community
        .post(1, "before leaving")
        .await
        .expect("bea posts");
    assert!(community.reads(2, "before leaving", LIMIT).await);

    leave(&community, 1, &bea).await;

    // A fresh invite; `join_with` asserts the re-joiner holds the key.
    community.join(1).await;
    assert_eq!(
        community.pseudonym(1).await,
        bea,
        "the same community pseudonym"
    );
    assert!(
        all_list(&community, &bea, true, &[0, 2]).await,
        "every member lists bea again"
    );
    community
        .post(1, "back again")
        .await
        .expect("the re-joiner posts");
    for i in [0, 2] {
        assert!(
            community.reads(i, "back again", LIMIT).await,
            "node {i} reads the re-joiner's post"
        );
    }
    // A member joining after the re-join walks the op DAG by `OpRef`
    // through the re-joiner's chain, whose `MemberJoined` lives on its
    // fresh member record.
    community.join(3).await;
    assert!(
        all_list(&community, &bea, true, &[3]).await,
        "a later joiner lists the re-joiner"
    );
    assert!(
        community.reads(3, "back again", LIMIT).await,
        "a later joiner reads the re-joiner's post"
    );
}

/// Node `i` leaves; every other member of nodes 0 and 2 stops listing it.
async fn leave(community: &Community, i: usize, who: &str) {
    community.nodes[i]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                governance_key: community.governance_key.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    assert!(
        all_list(community, who, false, &[0, 2]).await,
        "the leaver is gone from every list"
    );
}

/// Node `reader`'s merged heads of `departed`'s slots in the general
/// channel's record.
async fn general_heads(community: &Community, reader: usize, departed: &str) -> usize {
    let governance = community.nodes[reader].host.governance();
    let general = community.general_id(0).await;
    let Some(record_key) = (governance.community_membership(&community.governance_key))
        .and_then(|m| m.channel_log_keys.get(&general).cloned())
    else {
        return 0;
    };
    (governance.governance_state(&community.governance_key))
        .and_then(|state| {
            (state
                .departed_heads
                .get(&PseudonymKey::from_hex_lossy(departed)))
            .and_then(|heads| heads.get(&record_key).map(std::collections::BTreeSet::len))
        })
        .unwrap_or(0)
}

/// A/B finding F8-1 (2026-11-17): the owner kicks a member right after it
/// left, re-joined into a new slot and posted, before the registry poll
/// puts it on the roster. The removal heads the member's slot from the
/// pages it signed, not from the roster, and keeps the leave's head of the
/// old slot beside it, so every member records both heads and a member
/// joining after the kick keeps the member's post from the current slot.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_kick_right_after_a_rejoin_heads_the_current_slot_and_keeps_the_old() {
    let mut community = Community::create(4, 331).await;
    community.join(1).await;
    community.join(2).await;
    let bea = community.pseudonym(1).await;
    assert!(
        all_list(&community, &bea, true, &[0, 2]).await,
        "every member lists bea"
    );
    community
        .post(1, "before leaving")
        .await
        .expect("bea posts");
    assert!(community.reads(0, "before leaving", LIMIT).await);
    leave(&community, 1, &bea).await;
    community.join(1).await;
    community
        .post(1, "back again")
        .await
        .expect("the re-joiner posts");
    assert!(
        community.reads(0, "back again", LIMIT).await,
        "the owner reads the re-joiner's post"
    );
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.governance_key.clone(),
                target_pseudonym: bea.clone(),
                reason: None,
            },
        )))
        .await;
    for reader in [0, 2] {
        assert!(
            eventually(&community.sim, LIMIT, || async {
                general_heads(&community, reader, &bea).await == 2
            })
            .await,
            "node {reader} holds the leave's head and the kick's head of the current slot \
             (holds {})",
            general_heads(&community, reader, &bea).await
        );
    }
    // A member joining after the kick reads the roster's slots only, so
    // the old slot's page reaches it only once history reads cover
    // departed members (plan S9 open item (i)), and it holds no key of the
    // pre-rotation text (item (iii)); it keeps the current slot's entry
    // from exactly the kick's head.
    community.join(3).await;
    assert!(
        eventually(&community.sim, LIMIT, || async {
            (community.history(3, None, 100).await).is_ok_and(|page| {
                page.iter()
                    .any(|m| m["senderId"].as_str() == Some(bea.as_str()))
            })
        })
        .await,
        "a member joining after the kick keeps the current slot's entry"
    );
}
