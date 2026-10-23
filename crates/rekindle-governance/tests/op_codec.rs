//! A signed op off the wire: `VerifiedOp::verify` accepts the canonical
//! unsigned bytes `rekindle_codec::governance_op::decode_op` returns, and
//! no single tampered byte yields a different op that still verifies.

use rekindle_codec::governance_op::{decode_op, encode_op, op_unsigned_bytes};
use rekindle_governance::op::{op_signing_bytes, OpRejection, VerifiedOp};
use rekindle_secrets::derive::sign_with_pseudonym;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp};
use rekindle_types::id::PseudonymKey;

const ANCHOR: &str = "VLD0:anchor";

fn signed_op() -> GovernanceOp {
    let key = SigningKey::from_bytes(&[7; 32]);
    let mut op = GovernanceOp {
        author: PseudonymKey(key.verifying_key().to_bytes()),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 1_700_000_000,
        body: GovernanceEntry::CommunityPolicy {
            policy_text: Some("be kind".into()),
            max_joins_per_interval: 10,
            join_interval_seconds: 60,
        },
        sig: [0; 64],
    };
    op.sig = sign_with_pseudonym(&key, &op_signing_bytes(ANCHOR, &op_unsigned_bytes(&op)));
    op
}

fn verify(op: GovernanceOp, unsigned: &[u8]) -> Result<VerifiedOp, OpRejection> {
    VerifiedOp::verify(ANCHOR, op, unsigned, |_| None)
}

#[test]
fn decoded_op_verifies_over_its_canonical_bytes() {
    let op = signed_op();
    let decoded = decode_op(&encode_op(&op).unwrap()).unwrap();
    let verified = verify(decoded.op, &decoded.unsigned).unwrap();
    assert_eq!(verified.op(), &op);
    assert_eq!(verified.lamport(), 1);
    // Another community's anchor does not verify the same bytes.
    let decoded = decode_op(&encode_op(&op).unwrap()).unwrap();
    assert_eq!(
        VerifiedOp::verify("VLD0:other", decoded.op, &decoded.unsigned, |_| None).unwrap_err(),
        OpRejection::BadSignature
    );
}

#[test]
fn a_tampered_byte_fails_verification() {
    let op = signed_op();
    let wire = encode_op(&op).unwrap();
    let mut changed = 0;
    for i in 0..wire.len() {
        for flip in [0x01, 0x80] {
            let mut bytes = wire.clone();
            bytes[i] ^= flip;
            let Ok(decoded) = decode_op(&bytes) else {
                continue;
            };
            if decoded.op == op {
                // The same op (an unused bit): its canonical bytes are the
                // signed ones, so it verifies as the op it is.
                assert!(verify(decoded.op, &decoded.unsigned).is_ok());
                continue;
            }
            changed += 1;
            assert_eq!(
                verify(decoded.op, &decoded.unsigned).unwrap_err(),
                OpRejection::BadSignature,
                "byte {i} ^ {flip:#x}"
            );
        }
    }
    assert!(changed > 0, "some tampered byte must decode to another op");
}
