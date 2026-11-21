//! Which successors a rebuild judges and announces (plan E3.2 S7s.3e part
//! 2, lead decision (31)).

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::SuccessorState;
use rekindle_types::governance::{
    AdmissionMode, CommunitySuccessor, GenesisCert, GovernanceEntry, OpHash, SuccessorReason,
};
use rekindle_types::id::PseudonymKey;

use super::{
    follow_creators, invites_record, judge, pending, successor_entry, FollowRefusal,
    SuccessorJudgements,
};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

const OLD_ANCHOR: &str = "VLD0:old";

fn successor(state: &mut GovernanceState, hash: u8, new_anchor: Option<&str>) {
    let row = SuccessorState {
        author: PseudonymKey([1; 32]),
        successor: CommunitySuccessor {
            new_anchor: new_anchor.map(str::to_owned),
            reason: if new_anchor.is_some() {
                SuccessorReason::Recreate
            } else {
                SuccessorReason::Transfer
            },
            transfer_to: None,
            creator_set_hash: None,
            accepts: None,
            links: Vec::new(),
            invites_record: new_anchor.map(|_| "VLD0:inv".to_owned()),
        },
        stamp: OpStamp {
            lamport: u64::from(hash),
            hash: OpHash([hash; 32]),
        },
    };
    state.successors.insert(OpHash([hash; 32]), row);
}

/// `new_anchor`'s cert naming this community as its predecessor.
fn linked_cert() -> GenesisCert {
    let mut cert = rekindle_governance::fixture::cert(&PseudonymKey([5; 32]), AdmissionMode::Open);
    cert.predecessor = Some(OLD_ANCHOR.into());
    cert
}

fn offered(events: &[GovernanceRuntimeEvent]) -> Vec<String> {
    (events.iter())
        .map(|e| match e {
            GovernanceRuntimeEvent::CommunitySuccessorOffered { new_anchor, .. } => {
                new_anchor.clone()
            }
            other => panic!("unexpected event {other:?}"),
        })
        .collect()
}

#[test]
fn two_valid_successors_are_both_shown() {
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:a"));
    successor(&mut state, 2, Some("VLD0:b"));
    // A withdrawal names no anchor; an already judged successor was shown.
    successor(&mut state, 3, None);
    successor(&mut state, 4, Some("VLD0:c"));
    let judgements = SuccessorJudgements::default();
    judgements.mark(OpHash([4; 32]));
    let anchors: Vec<_> = pending(&state, &judgements)
        .iter()
        .filter_map(|(_, row)| row.successor.new_anchor.clone())
        .collect();
    assert_eq!(anchors, ["VLD0:a", "VLD0:b"]);
}

/// Decision (31): a far cert that cannot be read leaves the offer pending;
/// the next rebuild reads it and offers it, once.
#[tokio::test]
async fn an_unreadable_far_cert_is_offered_once_it_reads() {
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:new"));
    let judgements = SuccessorJudgements::default();
    let missing = |anchor: String| async move {
        Err::<GenesisCert, _>(GovernanceRuntimeError::GenesisCertMissing(anchor))
    };
    let first = judge(OLD_ANCHOR, &state, &judgements, missing).await;
    assert!(first.is_empty(), "nothing to offer before the cert reads");
    let reads = |_: String| async { Ok(linked_cert()) };
    let second = judge(OLD_ANCHOR, &state, &judgements, reads).await;
    assert_eq!(offered(&second), ["VLD0:new"], "offered once it reads");
    let third = judge(OLD_ANCHOR, &state, &judgements, reads).await;
    assert!(third.is_empty(), "an offered successor is offered once");
}

/// Decision (31): a cert that reads and fails a rule is refused and never
/// read again.
#[tokio::test]
async fn a_far_cert_that_fails_a_rule_is_not_read_again() {
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:new"));
    let judgements = SuccessorJudgements::default();
    let broken = |anchor: String| async move {
        Err::<GenesisCert, _>(GovernanceRuntimeError::Crypto(format!("{anchor}: bad sig")))
    };
    assert!(judge(OLD_ANCHOR, &state, &judgements, broken)
        .await
        .is_empty());
    let unlinked = |_: String| async {
        Ok(rekindle_governance::fixture::cert(
            &PseudonymKey([5; 32]),
            AdmissionMode::Open,
        ))
    };
    let mut state2 = GovernanceState::default();
    successor(&mut state2, 2, Some("VLD0:other"));
    assert!(judge(OLD_ANCHOR, &state2, &judgements, unlinked)
        .await
        .is_empty());
    let reads = |_: String| async { Ok(linked_cert()) };
    assert!(
        judge(OLD_ANCHOR, &state, &judgements, reads)
            .await
            .is_empty(),
        "a cert that failed a rule is not read again"
    );
    assert!(
        judge(OLD_ANCHOR, &state2, &judgements, reads)
            .await
            .is_empty(),
        "nor one that did not link back"
    );
}

/// A follow re-checks both links from the new community's side: the op
/// here naming the anchor, and the new cert naming this community.
#[test]
fn follow_checks_both_links_from_the_new_communitys_side() {
    use rekindle_governance::successor::SuccessorRefusal;
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:new"));
    let linked = linked_cert();
    let (signer, creators) =
        follow_creators(OLD_ANCHOR, &state, "VLD0:new", &linked).expect("both links hold");
    assert_eq!(signer, PseudonymKey([1; 32]));
    assert_eq!(creators, [PseudonymKey([5; 32])]);
    // The far link missing: the op alone is not enough.
    let mut unlinked = linked.clone();
    unlinked.predecessor = None;
    assert_eq!(
        follow_creators(OLD_ANCHOR, &state, "VLD0:new", &unlinked),
        Err(FollowRefusal::Links(SuccessorRefusal::NoPredecessorLink))
    );
    // The near link missing: a cert naming us is not enough.
    assert_eq!(
        follow_creators(OLD_ANCHOR, &state, "VLD0:other", &linked),
        Err(FollowRefusal::NoSuccessorOp)
    );
}

/// The offers a rebuild made are kept for the snapshot a window reads.
#[tokio::test]
async fn an_offer_is_kept_for_the_snapshot() {
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:a"));
    let judgements = SuccessorJudgements::default();
    assert!(judgements.offers().is_empty());
    let read = |_: String| async { Ok::<_, GovernanceRuntimeError>(linked_cert()) };
    let events = judge(OLD_ANCHOR, &state, &judgements, read).await;
    assert_eq!(offered(&events), ["VLD0:a"]);
    assert_eq!(offered(&judgements.offers()), ["VLD0:a"]);
}

/// Decision (34): the published op carries the first invites page's key.
#[test]
fn the_published_op_carries_its_invites_record() {
    let GovernanceEntry::CommunitySuccessor(op) = successor_entry("VLD0:new", "VLD0:inv") else {
        panic!("not a successor op");
    };
    assert_eq!(op.new_anchor.as_deref(), Some("VLD0:new"));
    assert_eq!(op.invites_record.as_deref(), Some("VLD0:inv"));
}

/// A follow reads the invites of the successor naming the anchor it
/// follows, by the signer it checked; a link naming another anchor, or
/// another author's successor, gives none and the follow is refused.
#[test]
fn a_follow_reads_only_the_linked_successors_invites() {
    let mut state = GovernanceState::default();
    successor(&mut state, 1, Some("VLD0:new"));
    let signer = PseudonymKey([1; 32]);
    assert_eq!(
        invites_record(&state, "VLD0:new", &signer).as_deref(),
        Some("VLD0:inv")
    );
    assert_eq!(invites_record(&state, "VLD0:other", &signer), None);
    assert_eq!(
        invites_record(&state, "VLD0:new", &PseudonymKey([9; 32])),
        None
    );
}
