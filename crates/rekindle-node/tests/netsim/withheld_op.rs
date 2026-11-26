//! A governance op is merged only once it is readable at its locator (lead
//! ruling 2026-11-26, items 1 and 5; ADR 0011's `OpRef` locator, plan D8 /
//! E3.2 S6 (4)): a member that gossips an op and never lands it on its op
//! log leaves no reader holding it, so no later joiner is asked to walk an
//! op the DHT never had.
//!
//! The withholding author signs `W` at its next seq `n`, gossips it and never
//! lands it, and keeps acting: its later ops take `n + 1`, `n + 2` with
//! `prev = W` (the chain rule, `op_log::build_op`), so nothing collides at a
//! seq (two signed ops at one seq is equivocation, a different property:
//! `equivocation::a_forked_device_key_equivocates_and_its_later_ops_are_quarantined`).
//! Those later ops depend on `W`, which no reader ever merges, so the merge
//! holds them for that dep (D15's fates, `op_log::fates`: "one the merge
//! holds for a dep is `PendingDeps`, and only an applied op is `Applied`"):
//! none is ever merged or cited as a dep, and a joiner through another
//! member still joins.

use std::time::Duration;

use rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD;
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::op_log;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_types::governance::OpHash;
use rekindle_types::id::PseudonymKey;

use crate::equivocation::{create_role, forged_fork, shows};
use crate::harness::Community;

/// The time a gossiped op takes to reach every member and be read.
const SEEN: Duration = Duration::from_secs(60);

/// The withholding author's names: `W`, then its two later ops.
const NAMES: [&str; 3] = ["withheld", "later-one", "later-two"];

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_withheld_gossiped_op_is_never_merged_and_never_blocks_a_joiner() {
    let mut community = Community::create(4, 1611).await;
    community.join(1).await;
    community.join(2).await;
    let anchor = community.anchor.clone();
    let gov = community.nodes[0].host.governance();
    let held = gov.governance_ops(&anchor).await.unwrap();
    let state = gov.governance_state(&anchor).expect("the creator's state");
    // `W`, signed at the creator's next seq `n` and gossiped; its append is
    // withheld for good. The author holds what it signed, so its later
    // ops chain after it.
    let withheld = forged_fork(&community, &held, &state, NAMES[0]);
    let decoded = rekindle_codec::governance_op::decode_op(&withheld).expect("the op decodes");
    let w_hash = rekindle_governance::op::op_hash(&decoded.unsigned, &decoded.op.sig);
    let n = decoded.op.seq;
    let cert = gov.genesis_cert(&anchor).expect("the pinned cert");
    let record = op_log::author_record(&cert, &state, &decoded.op.author).expect("our record");
    let own = rekindle_governance_runtime::op_receive::held_at(w_hash, &record, withheld.clone())
        .expect("W reads");
    gov.store_governance_op(&anchor, &own).await.unwrap();
    rekindle_governance_runtime::dht_hydration::rebuild_governance(&gov, &anchor).await;
    gov.gossip_governance_op(&anchor, &withheld);
    // The author's later ops: real ops, landed by its outbox, at `n + 1`
    // and `n + 2`, each citing the one before (`prev`), the first `W`.
    create_role(&community, 0, NAMES[1]).await;
    create_role(&community, 0, NAMES[2]).await;
    let later = later_ops(&community, &decoded.op.author, n).await;
    assert_eq!(
        later.iter().map(|(seq, _, _)| *seq).collect::<Vec<_>>(),
        vec![n + 1, n + 2],
        "the author's later ops chain after W, nothing at W's seq"
    );
    assert_eq!(later[0].2, Some(w_hash), "the first later op's prev is W");
    assert_eq!(later[1].2, Some(later[0].1), "the second's is the first");
    let dependent: Vec<OpHash> = [w_hash, later[0].1, later[1].1].into();
    tokio::time::sleep(SWEEP_PERIOD + SEEN).await;
    for i in 1..3 {
        never_merged(&community, i, &dependent).await;
    }
    // A member's own op after them cites none of them (ruling item 1: only
    // landed ops are cited as deps), and a joiner through that member walks
    // only what is on the DHT and joins: no `HistoryIncomplete` for it.
    let link = invite_from(&community, 1).await;
    let own_deps = newest_own_deps(&community, 1).await;
    assert!(
        own_deps.iter().all(|d| !dependent.contains(d)),
        "member 1's own op never cites W or the ops after it: {own_deps:?}"
    );
    community.join_with(3, link).await;
    tokio::time::sleep(SEEN).await;
    for i in 1..4 {
        never_merged(&community, i, &dependent).await;
    }
}

/// Member `i` neither shows nor stores `W` and shows none of the author's
/// later ops (held for their missing dep, never applied).
async fn never_merged(community: &Community, i: usize, dependent: &[OpHash]) {
    for name in NAMES {
        assert!(
            !shows(community, i, name).await,
            "member {i} never merges {name}: it never landed, or its dep did not"
        );
    }
    let stored = community.nodes[i]
        .host
        .governance()
        .governance_ops(&community.anchor)
        .await
        .unwrap();
    assert!(
        stored.iter().all(|op| op.hash != dependent[0]),
        "member {i} never stores the withheld op"
    );
}

/// The creator's held ops of `author` above seq `n`: `(seq, hash, prev)`,
/// by seq.
async fn later_ops(
    community: &Community,
    author: &PseudonymKey,
    n: u64,
) -> Vec<(u64, OpHash, Option<OpHash>)> {
    let gov = community.nodes[0].host.governance();
    let held = gov.governance_ops(&community.anchor).await.unwrap();
    let mut later: Vec<_> = (held.iter())
        .filter(|op| op.author == *author && op.seq > n)
        .map(|op| {
            let d = rekindle_codec::governance_op::decode_op(&op.encoded).expect("decodes");
            (op.seq, op.hash, d.op.prev)
        })
        .collect();
    later.sort_by_key(|(seq, _, _)| *seq);
    later
}

/// An invite from member `i`.
async fn invite_from(community: &Community, i: usize) -> String {
    let invite = community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::InviteCreate {
                community: community.anchor.clone(),
                max_uses: 1,
                expires_seconds: None,
            },
        )))
        .await;
    invite["url"].as_str().expect("an invite link").to_string()
}

/// The deps of member `i`'s newest own op.
async fn newest_own_deps(community: &Community, i: usize) -> Vec<OpHash> {
    let gov = community.nodes[i].host.governance();
    let (me, _) = op_log::own_pseudonym(&gov, &community.anchor).expect("a pseudonym");
    let held = gov.governance_ops(&community.anchor).await.unwrap();
    let newest = (held.iter())
        .filter(|op| op.author == me)
        .max_by_key(|op| op.seq)
        .expect("an own op");
    let d = rekindle_codec::governance_op::decode_op(&newest.encoded).expect("decodes");
    d.op.deps.iter().map(|r| r.op_hash).collect()
}
