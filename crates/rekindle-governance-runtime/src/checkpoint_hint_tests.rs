use rekindle_governance::checkpoint::{eligible_set_hash, frontier_hash};
use rekindle_governance::fixture::{cert, meta, op};
use rekindle_governance::merge::merge_outcome;
use rekindle_types::governance::{AdmissionMode, Checkpoint, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use super::*;

/// A creator-only community: the creator's first op and its height-1
/// checkpoint over it, the creator its electorate and only endorser.
fn creator_checkpoint() -> (GenesisCert, Vec<VerifiedOp>) {
    let creator = PseudonymKey([1; 32]);
    let cert = cert(&creator, AdmissionMode::Open);
    let first = op(&creator, 0, &[], 1, meta("C"));
    let body = Checkpoint {
        height: 1,
        round: 0,
        prev: None,
        cut: vec![first.op_ref(&cert.creator_member_record)],
        frontier_hash: frontier_hash(&[first.hash()]),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: [0; 32],
        state_size: 0,
        mek_generation: 1,
        eligible_set_hash: eligible_set_hash(std::slice::from_ref(&creator)),
        pages: Vec::new(),
    };
    let cp = op(&creator, 1, &[&first], 2, GovernanceEntry::Checkpoint(body));
    (cert, vec![first, cp])
}

/// Plan E3.2 S7c.5: the inviter answers the hint selected at the greatest
/// height (height 0), and the hint a height and digest name, with its
/// lowest op and a strict majority of endorsements located through the
/// state's member records (the cert's for the creator); a digest no usable
/// hint carries is answered with no hint.
#[test]
fn the_inviter_answers_the_selected_hint_with_a_majority_of_endorsements() {
    let (cert, ops) = creator_checkpoint();
    let outcome = merge_outcome(&cert, &ops);
    let hint = outcome.hints.latest().expect("a usable height-1 hint");
    let at = |height, digest: &[u8; 32]| {
        hint_response(
            &cert,
            &outcome.state,
            &ops,
            &outcome.hints,
            (height, digest),
        )
    };
    let want = ops[1].op_ref(&cert.creator_member_record);
    for answer in [at(0, &[0; 32]), at(1, &hint.digest)] {
        assert_eq!(answer.checkpoint.as_ref(), Some(&want));
        assert_eq!(answer.endorsements, vec![want.clone()]);
    }
    assert_eq!(at(1, &[9; 32]), CheckpointHintResponse::default());
    assert_eq!(at(2, &hint.digest), CheckpointHintResponse::default());
}

/// S7c.4: a community holding a checkpoint equivocation uses no hint, so
/// its members answer none.
#[test]
fn an_equivocated_community_answers_no_hint() {
    let (cert, ops) = creator_checkpoint();
    let mut outcome = merge_outcome(&cert, &ops);
    let digest = outcome.hints.latest().expect("a usable hint").digest;
    outcome
        .hints
        .equivocations
        .push(rekindle_governance::checkpoint::Equivocation {
            digests: vec![digest, [0; 32]],
            flagged: Vec::new(),
        });
    for height in [0, 1] {
        let answer = hint_response(
            &cert,
            &outcome.state,
            &ops,
            &outcome.hints,
            (height, &digest),
        );
        assert_eq!(answer, CheckpointHintResponse::default());
    }
}
