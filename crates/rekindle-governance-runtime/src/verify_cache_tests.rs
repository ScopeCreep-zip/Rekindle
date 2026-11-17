//! An op verifies once per cache: a warm read verifies no signature, a
//! tampered held op is still quarantined, a refusal is kept, a missing dep
//! is not, and the cache holds the last held set's ops only.

use super::*;
use crate::op_log::{build_op, held, verify_held, Deps, OpFate};
use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::fixture::meta;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::id::PseudonymKey;

fn chain(len: usize) -> Vec<HeldOp> {
    let key = SigningKey::from_bytes(&[7; 32]);
    let cert = GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(key.verifying_key().to_bytes()),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
    };
    let state = GovernanceState::default();
    let mut out = Vec::new();
    for i in 0..len {
        let set = verify_held("A", &out);
        let (op, bytes) = build_op(
            "A",
            &cert,
            &state,
            &set,
            &key,
            meta(&format!("m{i}")),
            Deps::Frontier,
            5,
        )
        .unwrap();
        out.push(held(&op, "VLD0:m".into(), bytes, OpFate::Applied));
    }
    out
}

/// `op` with its signature's first byte flipped, re-hashed: a refused op.
fn bad_signature(op: &HeldOp) -> HeldOp {
    let mut decoded = decode_op(&op.encoded).unwrap().op;
    decoded.sig[0] ^= 1;
    let hash = op_hash(&op_unsigned_bytes(&decoded), &decoded.sig);
    HeldOp {
        hash,
        encoded: encode_op(&decoded).unwrap(),
        ..op.clone()
    }
}

#[test]
fn a_warm_cache_verifies_no_held_op_again() {
    let ops = chain(3);
    let cache = OpVerifyCache::default();
    let cold = verify_held_cached("A", &ops, &cache);
    // Op 0 verifies in the first pass; ops 1 and 2 wait on their deps
    // there, and each is verified once more when its dep is (not on every
    // pass: 6).
    assert_eq!(cache.verifications(), 5);
    let warm = verify_held_cached("A", &ops, &cache);
    assert_eq!(cache.verifications(), 5, "no signature is verified again");
    assert_eq!(warm.ops, cold.ops);
    assert_eq!(warm.ops, verify_held("A", &ops).ops);
    assert!(warm.pending.is_empty() && warm.quarantined.is_empty());
    // A new op costs at most two: it waits one pass for its kept dep to
    // join the pass's verified set, then verifies.
    let more = chain(4);
    let set = verify_held_cached("A", &more, &cache);
    assert_eq!((set.ops.len(), cache.verifications()), (4, 7));
}

#[test]
fn a_tampered_op_under_a_kept_hash_is_still_quarantined() {
    let ops = chain(2);
    let cache = OpVerifyCache::default();
    let _ = verify_held_cached("A", &ops, &cache);
    // The first op's hash slot holding the second op's bytes.
    let tampered = HeldOp {
        encoded: ops[1].encoded.clone(),
        ..ops[0].clone()
    };
    let set = verify_held_cached("A", &[tampered], &cache);
    assert!(set.ops.is_empty());
    assert_eq!(set.quarantined, vec![ops[0].hash]);
}

#[test]
fn a_refusal_is_kept_and_a_missing_dep_is_not() {
    let ops = chain(2);
    let cache = OpVerifyCache::default();
    let set = verify_held_cached("A", &ops[1..], &cache);
    assert_eq!(set.pending, vec![ops[1].hash]);
    assert!(cache.is_empty(), "a missing dep is not a verdict");
    let bad = bad_signature(&ops[0]);
    let held = [bad.clone(), ops[0].clone(), ops[1].clone()];
    let set = verify_held_cached("A", &held, &cache);
    assert_eq!(
        (set.ops.len(), set.quarantined.clone()),
        (2, vec![bad.hash])
    );
    let spent = cache.verifications();
    let set = verify_held_cached("A", &held, &cache);
    assert_eq!(set.quarantined, vec![bad.hash]);
    assert_eq!(cache.verifications(), spent, "the refusal is kept");
}

#[test]
fn the_cache_holds_the_last_held_set_only() {
    let ops = chain(2);
    let cache = OpVerifyCache::default();
    let _ = verify_held_cached("A", &ops, &cache);
    assert_eq!(cache.len(), 2);
    let _ = verify_held_cached("A", &ops[..1], &cache);
    assert_eq!(cache.len(), 1);
}
