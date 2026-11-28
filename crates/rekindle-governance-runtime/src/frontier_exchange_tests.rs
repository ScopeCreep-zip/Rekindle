use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::fixture::{cert, meta};
use rekindle_governance::op::op_signing_bytes;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{AdmissionMode, GovernanceOp, OpHash};
use rekindle_types::id::PseudonymKey;

use super::*;

const ANCHOR: &str = "VLD0:anchor";

/// The creator's signed seq-0 op, encoded, and the head naming it on the
/// cert's member record.
fn signed_head() -> (GenesisCert, OpRef, Vec<u8>) {
    let key = SigningKey::from_bytes(&[5; 32]);
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let cert = cert(&author, AdmissionMode::Open);
    let mut op = GovernanceOp {
        device_key: author.0,
        author: author.clone(),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 1,
        body: meta("C"),
        sig: [0; 64],
    };
    let unsigned = op_unsigned_bytes(&op).unwrap();
    op.sig = key.sign(&op_signing_bytes(ANCHOR, &unsigned)).to_bytes();
    let head = OpRef {
        op_hash: op_hash(&unsigned, &op.sig),
        device_key: op.device_key,
        author,
        author_record: cert.creator_member_record.clone(),
        author_seq: 0,
    };
    (cert, head, encode_op(&op).unwrap())
}

/// Plan E3.2 S7d-keys.8: a head holds when the op at its locator is the op
/// it names; a forged head (another hash, a raised seq, another device key,
/// another record than the one the state names, a broken signature, another
/// community) is refused, so a peer cannot raise the barrier's result.
#[test]
fn a_forged_frontier_head_is_refused() {
    let (cert, head, encoded) = signed_head();
    let state = GovernanceState::default();
    let known = Some((&cert, &state));
    assert!(head_holds(ANCHOR, &head, &encoded, known));
    assert!(head_holds(ANCHOR, &head, &encoded, None));

    let mut forged = head.clone();
    forged.op_hash = OpHash([7; 32]);
    assert!(
        !head_holds(ANCHOR, &forged, &encoded, known),
        "another hash"
    );
    let mut forged = head.clone();
    forged.author_seq = 9;
    assert!(
        !head_holds(ANCHOR, &forged, &encoded, known),
        "a raised seq"
    );
    let mut forged = head.clone();
    forged.device_key = [8; 32];
    assert!(
        !head_holds(ANCHOR, &forged, &encoded, known),
        "another device key"
    );
    let mut forged = head.clone();
    forged.author_record = "VLD0:elsewhere".into();
    assert!(
        !head_holds(ANCHOR, &forged, &encoded, known),
        "another record"
    );
    assert!(
        !head_holds("VLD0:other", &head, &encoded, None),
        "another community"
    );
    let mut broken = encoded.clone();
    let last = broken.len() - 1;
    broken[last] ^= 1;
    assert!(!head_holds(ANCHOR, &head, &broken, known), "a broken op");
}
