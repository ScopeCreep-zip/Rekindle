//! Our ops are signed by this device's op key under `p` (ADR 0022 rule 1):
//! every reader judges them effective with the real key, and a key the
//! vault lost is refused, never minted again (rule 4, threat model T7).

use rekindle_governance::device_judge::{judge_op, DeviceView, KeyJudgement};
use rekindle_governance::fixture::meta;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::id::PseudonymKey;

use super::{first_endorsement, held_key, OpSigner};
use crate::error::GovernanceRuntimeError;
use crate::op_log::{build_op, Deps, VerifiedSet};

const ANCHOR: &str = "A";

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
        creator_device: Some(first_endorsement(p, ANCHOR, d, record, owner).unwrap()),
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
        assert_eq!(judge_op(ANCHOR, &view, op.op()), KeyJudgement::Effective);
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
