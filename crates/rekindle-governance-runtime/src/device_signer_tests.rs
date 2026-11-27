//! Our ops are signed by this device's op key under `p` (ADR 0022 rule 1):
//! every reader judges them effective with the real key, and a key the
//! vault lost is refused, never minted again (rule 4, threat model T7).

use rekindle_governance::device_judge::{judge_op_at, DeviceView, KeyJudgement};
use rekindle_governance::fixture::meta;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::id::PseudonymKey;

use super::{first_endorsement, held_key, OpSigner};
use crate::error::GovernanceRuntimeError;
use crate::op_log::{build_op, Deps, VerifiedSet};
use crate::test_world::ANCHOR;

fn cert(p: &SigningKey, d: &SigningKey) -> GenesisCert {
    let (record, owner) = ("VLD0:m", "o");
    GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(p.verifying_key().to_bytes()),
        creator_member_record: record.into(),
        creator_member_record_owner: owner.into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: Some(first_endorsement(p, ANCHOR, d, record, owner)),
    }
}

#[test]
fn ops_built_under_the_device_key_judge_effective_with_the_real_key() {
    let (p, d) = (
        SigningKey::from_bytes(&[7; 32]),
        SigningKey::from_bytes(&[8; 32]),
    );
    let author = PseudonymKey(p.verifying_key().to_bytes());
    let cert = cert(&p, &d);
    let state = GovernanceState::default();
    let mut set = VerifiedSet::default();
    for (seq, name) in (0u64..).zip(["a", "b", "c"]) {
        let signer = OpSigner {
            author: author.clone(),
            device: &d,
        };
        let (op, _) = build_op(
            ANCHOR,
            &cert,
            &state,
            &set,
            signer,
            meta(name),
            Deps::Frontier,
            seq,
        )
        .unwrap();
        assert_eq!(op.op().author, author, "p stays the author");
        assert_eq!(op.op().device_key, d.verifying_key().to_bytes());
        assert_eq!(op.op().seq, seq, "the device key's own chain");
        set.ops.push(op);
    }
    let mut view = DeviceView::default();
    view.seed_genesis(ANCHOR, &cert);
    for op in &set.ops {
        view.absorb(ANCHOR, op.op());
    }
    for op in &set.ops {
        // Judged for the author's latest join, as the merge judges it (Q4).
        let md = view.member(&op.op().author).cloned().unwrap_or_default();
        let record = md.latest_join.as_ref().map(|(_, r, _)| r.as_str());
        assert_eq!(
            judge_op_at(ANCHOR, &md, record, op.op()),
            KeyJudgement::Effective
        );
    }
}

#[test]
fn a_lost_device_key_is_refused_never_minted_again() {
    // A later op with no stored key: refused.
    assert!(matches!(
        held_key(None, ANCHOR, false),
        Err(GovernanceRuntimeError::DeviceOpKeyMissing(c)) if c == ANCHOR
    ));
    // Only this device's first op in the community may mint.
    assert!(held_key(None, ANCHOR, true).unwrap().is_none());
    // A stored key is used as it is, first op or not.
    let held = zeroize::Zeroizing::new([5; 32]);
    let key = held_key(Some(held), ANCHOR, false).unwrap().unwrap();
    assert_eq!(key.to_bytes(), [5; 32]);
}

/// A checkpoint endorsement is signed by its author's device key (ADR 0022
/// rule 1): the chain reads it as an endorsement only when it verifies
/// under that key, not under the author's pseudonym.
#[test]
fn a_device_signed_checkpoint_endorsement_verifies_under_the_device_key() {
    use rekindle_governance::checkpoint::SignedEndorsement;
    let creator = crate::test_member::Member::new(1);
    let mut world = crate::test_world::World::new(&creator);
    let body = world.checkpoint_body();
    let op = world.endorse(&creator, body);
    assert_ne!(op.op().device_key, creator.author().0);
    let unsigned = rekindle_codec::governance_op::op_unsigned_bytes(op.op()).unwrap();
    let read = SignedEndorsement::verify(crate::test_world::ANCHOR, op.op(), &unsigned);
    assert_eq!(read.map(|e| e.author), Some(creator.author()));
}
