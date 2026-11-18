//! A/B finding R3-8, its open half: a member who left on its own keeps its
//! channel slot's keys in any backup of its profile, so a restored backup
//! can write the slot after the leave and back-date the post (its clock an
//! hour behind, so the post's `sent_at` precedes the leave). The leaver
//! signs its slot heads into its `MemberLeft`, and the history keeps a
//! departed author's post only at or below them, never by the time the
//! author wrote into it, so no reader shows or stores the back-dated post,
//! while the leaver's genuine post from before the leave still shows.
//!
//! The backup is taken while the member is locked, before it leaves, and
//! put back after; it is cut off from every peer when it posts, as a
//! restore made off-line (`equivocation.rs` stages a restored backup).
//!
//! The `MemberLeft` must name the leaver's own slot head, which every
//! reader holds (A/B 2026-11-17: a joiner's own slot is not on its roster,
//! so a leave signed no heads, and a reader backfilling its history kept
//! none of its genuine posts).
//!
//! Plan E3.2 S9: a head names the exact slot value, its seq and the digest
//! of its page, so a leaver that signs a head above its published seq and
//! fills the gap later (veilid-core 0.5.8 `set_value` `min_seqnum`) gets no
//! gap post kept, while an honest leaver's last write, withheld until after
//! the leave, is kept by every member.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_netsim::Fault;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_types::governance::{GovernanceEntry, SlotHead};
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community, PASSPHRASE};
use crate::moderation::listed;

const LIMIT: Duration = Duration::from_secs(120);

async fn unlock(community: &Community, i: usize) {
    let node = &community.nodes[i];
    node.ok(IpcRequest::Unlock {
        identity: node.public_key.clone().expect("an identity"),
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

/// Assert node `reader`'s merged state records a head for `departed`'s
/// departure in the general channel's record, the one record it posted in,
/// and in no other.
pub(crate) async fn assert_departure_heads(community: &Community, reader: usize, departed: &str) {
    let governance = community.nodes[reader].host.governance();
    let general = community.general_id(0).await;
    let record_key = (governance.community_membership(&community.anchor))
        .and_then(|m| m.channel_log_keys.get(&general).cloned())
        .expect("the general channel's record");
    let heads = (governance.governance_state(&community.anchor))
        .and_then(|state| {
            (state.departed_heads)
                .get(&PseudonymKey::from_hex_lossy(departed))
                .cloned()
        })
        .unwrap_or_default();
    assert_eq!(
        heads.keys().collect::<Vec<_>>(),
        vec![&record_key],
        "node {reader} holds the departure's head of the general channel's record"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leavers_back_dated_slot_write_is_kept_by_no_member() {
    let community = Community::of(3, 163).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let bea = community.pseudonym(1).await;
    let _ = community.post(1, "bea before leaving").await;
    for reader in [0, 2] {
        assert!(
            community.reads(reader, "bea before leaving", LIMIT).await,
            "node {reader} reads the member's post before the leave"
        );
    }
    community.nodes[1].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[1]);
    unlock(&community, 1).await;
    community.nodes[1]
        .call_held(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                anchor: community.anchor.clone(),
            },
        )))
        .await
        .expect("the leave commits");
    for reader in [0, 2] {
        assert!(
            eventually(&community.sim, LIMIT, || async {
                listed(&community, reader, &bea).await.is_none()
            })
            .await,
            "node {reader} merges the leave"
        );
        assert_departure_heads(&community, reader, &bea).await;
    }
    community.nodes[1].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[1], &backup);
    community.sim.fault(Fault::Partition {
        a: vec![0, 2],
        b: vec![1],
    });
    unlock(&community, 1).await;
    community.sim.fault(Fault::ClockSkew {
        node: 1,
        ms: -3_600_000,
    });
    community
        .post(1, "bea back-dated before leaving")
        .await
        .expect("the restored backup writes its slot");
    assert!(
        community
            .history_holds(1, "bea back-dated before leaving", LIMIT)
            .await,
        "the post is in the leaver's slot"
    );
    community.sim.fault(Fault::Heal);
    community.sim.quiesce().await;
    for reader in [0, 2] {
        assert!(
            !community
                .reads(reader, "bea back-dated before leaving", LIMIT)
                .await,
            "node {reader} never shows the back-dated post"
        );
        let page = community.history(reader, None, 100).await.expect("history");
        assert!(
            page.iter()
                .all(|m| m["body"].as_str() != Some("bea back-dated before leaving")),
            "node {reader} holds no back-dated post: {page:?}"
        );
        assert!(
            page.iter()
                .any(|m| m["body"].as_str() == Some("bea before leaving")),
            "node {reader} still shows the genuine pre-leave post: {page:?}"
        );
    }
}

/// The heads node `i` would sign for `departed` now (its local copy).
async fn heads_of(community: &Community, i: usize, departed: &str) -> Vec<SlotHead> {
    (community.nodes[i].host.governance())
        .departed_slot_heads(&community.anchor, departed)
        .await
        .expect("the slot heads")
}

/// Node `reader`'s merged heads of `departed`'s slots in `record_key`.
fn merged_head(
    community: &Community,
    reader: usize,
    departed: &str,
    record_key: &str,
) -> Option<std::collections::BTreeSet<(u32, [u8; 32])>> {
    (community.nodes[reader].host.governance())
        .governance_state(&community.anchor)
        .and_then(|state| {
            (state
                .departed_heads
                .get(&PseudonymKey::from_hex_lossy(departed)))
            .and_then(|heads| heads.get(record_key).cloned())
        })
}

/// Node `i` shows none of `body` and holds no row of it.
async fn holds_none(community: &Community, i: usize, body: &str) {
    assert!(
        !community.reads(i, body, LIMIT).await,
        "node {i} never shows {body:?}"
    );
    let page = community.history(i, None, 100).await.expect("history");
    assert!(
        page.iter().all(|m| m["body"].as_str() != Some(body)),
        "node {i} holds no {body:?}: {page:?}"
    );
}

/// Plan E3.2 S9: a modified leaver signs a `MemberLeft` whose head
/// reserves seqs above the one it published (its seq + 5, a digest of a
/// page it never wrote), then, from a backup restored after the leave,
/// writes into the gap with `min_seqnum` (a public 0.5.8 `set_value`
/// option) at exactly the head's seq. No member keeps the gap post: not
/// the readers who held the leaver's genuine post live, who still show
/// that (a post-departure write lifts no held row out), and not a member
/// joining after the leave, which reads only the gap value.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leavers_reserved_seq_gap_is_kept_by_no_member() {
    let mut community = Community::create(4, 167).await;
    community.join(1).await;
    community.join(2).await;
    let bea = community.pseudonym(1).await;
    let _ = community.post(1, "bea before leaving").await;
    for reader in [0, 2] {
        assert!(
            community.reads(reader, "bea before leaving", LIMIT).await,
            "node {reader} reads the member's post before the leave"
        );
    }
    let honest = heads_of(&community, 1, &bea).await;
    let [honest] = honest.as_slice() else {
        panic!("one head, the general channel's: {honest:?}");
    };
    let reserved = SlotHead {
        record_key: honest.record_key.clone(),
        seq: honest.seq + 5,
        digest: [0xA5; 32],
    };
    community.nodes[1].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[1]);
    unlock(&community, 1).await;
    let forged = GovernanceEntry::MemberLeft {
        heads: vec![reserved.clone()],
    };
    rekindle_governance_runtime::apply::write_entry(
        &community.nodes[1].host.governance(),
        &community.anchor,
        forged,
    )
    .await
    .expect("the forged leave commits");
    for reader in [0, 2] {
        assert!(
            eventually(&community.sim, LIMIT, || async {
                merged_head(&community, reader, &bea, &reserved.record_key)
                    == Some([(reserved.seq, reserved.digest)].into_iter().collect())
            })
            .await,
            "node {reader} merges the leave with its reserved head"
        );
    }
    community.nodes[1].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[1], &backup);
    community.sim.fault(Fault::Partition {
        a: vec![0, 2, 3],
        b: vec![1],
    });
    unlock(&community, 1).await;
    community.sim.fault(Fault::MinSeqnum {
        node: 1,
        key: reserved.record_key.clone(),
        min_seqnum: Some(reserved.seq),
    });
    community
        .post(1, "bea in the reserved gap")
        .await
        .expect("the restored backup writes its slot");
    assert!(
        community
            .history_holds(1, "bea in the reserved gap", LIMIT)
            .await,
        "the post is in the leaver's slot"
    );
    assert_eq!(
        heads_of(&community, 1, &bea).await.first().map(|h| h.seq),
        Some(reserved.seq),
        "the gap write landed at the reserved head's seq"
    );
    community.sim.fault(Fault::Heal);
    community.sim.quiesce().await;
    community.join(3).await;
    for reader in [0, 2, 3] {
        holds_none(&community, reader, "bea in the reserved gap").await;
    }
    for reader in [0, 2] {
        let page = community.history(reader, None, 100).await.expect("history");
        assert!(
            page.iter()
                .any(|m| m["body"].as_str() == Some("bea before leaving")),
            "node {reader} still shows the genuine pre-leave post: {page:?}"
        );
    }
}

/// The honest control: a leaver's last post, written while it is cut off
/// (held, never delivered live), is the value its leave's head names
/// (`moderation::leave`, the production leave op). After the heal every
/// member keeps it and the earlier post from the leaver's slot, the last one
/// from history alone. The exact-value rule keeps a cumulative page's whole
/// history.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_leavers_withheld_last_write_is_kept_by_every_member() {
    let community = Community::of(4, 173).await;
    let bea = community.pseudonym(1).await;
    let _ = community.post(1, "bea before the cut").await;
    for reader in [0, 2, 3] {
        assert!(
            community.reads(reader, "bea before the cut", LIMIT).await,
            "node {reader} reads the member's first post"
        );
    }
    community.sim.fault(Fault::Partition {
        a: vec![0, 2, 3],
        b: vec![1],
    });
    let _ = community.post(1, "bea last, held").await;
    assert!(
        community.history_holds(1, "bea last, held", LIMIT).await,
        "the held post is in the leaver's slot"
    );
    rekindle_governance_runtime::moderation::leave(
        &community.nodes[1].host.governance(),
        &community.anchor,
    )
    .await
    .expect("the leave op commits");
    community.sim.fault(Fault::Heal);
    community.sim.quiesce().await;
    for reader in [0, 2, 3] {
        assert!(
            eventually(&community.sim, LIMIT, || async {
                listed(&community, reader, &bea).await.is_none()
            })
            .await,
            "node {reader} merges the leave"
        );
    }
    for reader in [0, 2, 3] {
        for body in ["bea before the cut", "bea last, held"] {
            assert!(
                community.reads(reader, body, LIMIT).await,
                "node {reader} keeps the leaver's {body:?}"
            );
        }
    }
}
