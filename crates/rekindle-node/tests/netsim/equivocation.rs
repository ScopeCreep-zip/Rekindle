//! Plan E3.2 S7 Verify (D15, register S7d): a device key that signs two
//! different ops at one seq equivocates. The other members detect it
//! within one anti-entropy sweep and drop every op of that author from the
//! fork on, the later ones too.
//!
//! The fork's source moved with S7d-keys.8: a restored backup cut off from
//! every peer no longer signs (the unlock barrier holds its write, and
//! after the heal it detects the restore and signs nothing), so that
//! scenario now asserts the hold and the detection; the readers'
//! equivocation coverage takes a fork built directly, two ops at one seq
//! under the creator's device key, injected through its gossip.

use std::time::Duration;

use rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD;
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::device_signer::OpSigner;
use rekindle_governance_runtime::op_log::{self, HeldOp, OpFate};
use rekindle_governance_runtime::unlock_barrier::BarrierPhase;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, MemberRequest};
use rekindle_netsim::Fault;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::display_catch_up::{PendingGovKind, PendingGovOp, PendingGovState};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::governance::OpHash;
use rekindle_types::id::RoleId;

use crate::harness::{eventually, Community, PASSPHRASE};

const SEEN: Duration = Duration::from_secs(180);

/// A role: a governance op and nothing else (a channel create still mints
/// its SMPL record, which a node cut off from its peers cannot store).
pub(crate) async fn create_role(community: &Community, i: usize, name: &str) {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: name.into(),
                permissions: 0,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
}

pub(crate) async fn shows(community: &Community, i: usize, name: &str) -> bool {
    community.nodes[i]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleList {
                community: community.anchor.clone(),
            },
        )))
        .await
        .to_string()
        .contains(name)
}

pub(crate) async fn unlock(community: &Community, i: usize) {
    let node = &community.nodes[i];
    node.ok(IpcRequest::Unlock {
        identity: node.public_key.clone().expect("an identity"),
        passphrase: PASSPHRASE.into(),
    })
    .await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_restored_backup_cut_off_from_every_peer_is_held_then_restore_detected() {
    let community = Community::of(3, 1515).await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    community.nodes[0].ok(IpcRequest::Lock).await;
    let backup = crate::data_root::snapshot(&community.sim, &community.nodes[0]);
    unlock(&community, 0).await;
    create_role(&community, 0, "alpha").await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            shows(&community, 1, "alpha").await && shows(&community, 2, "alpha").await
        })
        .await,
        "the members see the first branch"
    );

    community.nodes[0].ok(IpcRequest::Lock).await;
    crate::data_root::restore(&community.sim, &community.nodes[0], &backup);
    community.sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    unlock(&community, 0).await;
    // Answered at once, held: no ring peer answers the barrier.
    create_role(&community, 0, "bravo").await;
    let gov = community.nodes[0].host.governance();
    let barrier = gov.unlock_barrier(&community.anchor);
    tokio::time::sleep(SEEN).await;
    assert_eq!(
        barrier.phase(),
        BarrierPhase::CatchingUp,
        "no peer answered"
    );
    // An automatic `Checkpoint` (the anti-entropy tick's) may be held
    // beside bravo; the claim is bravo's.
    let held = barrier.view().expect("the held write is listed").pending;
    let roles = |held: &[PendingGovOp]| {
        (held.iter())
            .filter(|op| op.kind == PendingGovKind::RoleEdited)
            .count()
    };
    assert_eq!(roles(held.as_slice()), 1, "bravo is held: {held:?}");
    community.sim.fault(Fault::Heal);
    let detected = eventually(&community.sim, SWEEP_PERIOD + SEEN, || async {
        matches!(barrier.phase(), BarrierPhase::Restored(_))
    })
    .await;
    assert!(detected, "the barrier finds the key signed past this copy");
    tokio::time::sleep(SWEEP_PERIOD + SEEN).await;
    for i in 1..3 {
        assert!(
            !shows(&community, i, "bravo").await,
            "member {i} never sees an op the restored copy did not sign"
        );
        assert!(
            shows(&community, i, "alpha").await,
            "member {i} keeps alpha"
        );
    }
    let still = barrier.view().expect("the write stays held").pending;
    assert_eq!(
        roles(still.as_slice()),
        1,
        "bravo stays held, unsigned: {still:?}"
    );
    assert!(
        (still.iter())
            .filter(|op| op.kind == PendingGovKind::RoleEdited)
            .all(|op| op.state == PendingGovState::CatchingUp),
        "bravo is never signed: {still:?}"
    );
    assert!(
        crate::logs::warned("governance equivocation").is_empty(),
        "no fork reached any member"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_forked_device_key_equivocates_and_its_later_ops_are_quarantined() {
    let community = Community::of(3, 1515).await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    // What a backup taken now holds: the op set and state before alpha.
    let gov = community.nodes[0].host.governance();
    let held_before = gov.governance_ops(&community.anchor).await.unwrap();
    let state_before = gov
        .governance_state(&community.anchor)
        .expect("the creator's state");
    create_role(&community, 0, "alpha").await;
    assert!(
        eventually(&community.sim, SEEN, || async {
            shows(&community, 1, "alpha").await && shows(&community, 2, "alpha").await
        })
        .await,
        "the members see the first branch"
    );
    let bravo = forged_fork(&community, &held_before, &state_before, "bravo");
    gov.gossip_governance_op(&community.anchor, &bravo);
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

    // The fork as each member stored it: the creator's two ops at one seq,
    // both held as `Equivocation` (D15), and its earlier ops not.
    let creator = community.pseudonym(0).await;
    let before: Vec<Vec<Held>> = creator_ops(&community, &creator).await;
    let fork: Vec<u64> = before
        .iter()
        .enumerate()
        .map(|(k, ops)| fork_seq(ops).unwrap_or_else(|| panic!("member {} stores the fork", k + 1)))
        .collect();
    for (k, ops) in before.iter().enumerate() {
        for op in ops {
            assert_eq!(
                op.fate == OpFate::Equivocation,
                op.seq >= fork[k],
                "member {}: only the creator's ops from the fork on are equivocation: {op:?}",
                k + 1
            );
        }
    }
    // Charlie is the creator's next op after the fork (nothing intervenes),
    // so its seq is the fork's plus one; it must reach each member and be
    // stored there as `Equivocation`, not merely be missing.
    create_role(&community, 0, "charlie").await;
    let quarantined = eventually(&community.sim, SWEEP_PERIOD * 2 + SEEN, || async {
        let now = creator_ops(&community, &creator).await;
        now.iter().enumerate().all(|(k, ops)| {
            ops.iter().any(|op| {
                op.seq == fork[k] + 1
                    && op.fate == OpFate::Equivocation
                    && !before[k].iter().any(|held| held.hash == op.hash)
            })
        })
    })
    .await;
    assert!(
        quarantined,
        "each member holds charlie, the equivocator's later op, as equivocation: {:#?}",
        creator_ops(&community, &creator).await
    );
    for i in 1..3 {
        assert!(
            !shows(&community, i, "charlie").await,
            "member {i} quarantines the equivocator's later op"
        );
    }
}

/// One of the creator's ops as a member's op store holds it.
#[derive(Debug)]
struct Held {
    hash: OpHash,
    seq: u64,
    fate: OpFate,
}

/// The creator's (`author`, hex) ops each member (1 and 2) stores.
async fn creator_ops(community: &Community, author: &str) -> Vec<Vec<Held>> {
    let mut out = Vec::new();
    for i in 1..3 {
        let held = (community.nodes[i].host.governance())
            .governance_ops(&community.anchor)
            .await
            .unwrap_or_default();
        out.push(
            held.into_iter()
                .filter(|op| hex::encode(op.author.0) == author)
                .map(|op| Held {
                    hash: op.hash,
                    seq: op.seq,
                    fate: op.fate,
                })
                .collect(),
        );
    }
    out
}

/// The fork seq in a member's record: the lowest seq at which it holds two
/// different ops of the author.
fn fork_seq(ops: &[Held]) -> Option<u64> {
    ops.iter()
        .filter(|a| ops.iter().any(|b| b.seq == a.seq && b.hash != a.hash))
        .map(|op| op.seq)
        .min()
}

/// A fork built directly: `name`'s role op signed under the creator's
/// device key on the op set and state it held before its last op
/// (`held`, `state`), so at that op's seq: the op a restored copy would
/// sign, injected without one.
pub(crate) fn forged_fork(
    community: &Community,
    held: &[HeldOp],
    state: &rekindle_governance::state::GovernanceState,
    name: &str,
) -> Vec<u8> {
    let gov = community.nodes[0].host.governance();
    let anchor = &community.anchor;
    let set = op_log::verify_held(anchor, held);
    let cert = gov.genesis_cert(anchor).expect("the pinned cert");
    let secret = gov
        .device_op_key(anchor)
        .expect("the vault reads")
        .expect("the creator's device key");
    let device = SigningKey::from_bytes(&secret);
    let (author, _) = op_log::own_pseudonym(&gov, anchor).expect("our pseudonym");
    let body = GovernanceEntry::RoleDefinition {
        role_id: RoleId([0xbb; 16]),
        name: name.into(),
        permissions: 0,
        position: 1,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    };
    let signer = OpSigner {
        author,
        device: &device,
    };
    let now = gov.clock().now_ms();
    let (_, encoded) = op_log::build_op(
        anchor,
        &cert,
        state,
        &set,
        signer,
        body,
        op_log::Deps::Frontier,
        now,
    )
    .expect("the forged op builds");
    encoded
}
