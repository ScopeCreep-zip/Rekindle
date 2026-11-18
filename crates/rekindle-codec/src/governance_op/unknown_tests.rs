//! D15: an op this reader cannot interpret is read with the canonical
//! unsigned bytes its author signed, so its signature can be checked and
//! the community quarantined, never dropped silently.

use rekindle_types::governance::{GovernanceEntry, GovernanceOp};
use rekindle_types::id::PseudonymKey;

use super::{decode_op, op_unsigned_bytes, read_op, seal_op, ReadOp, Uninterpretable};
use crate::CodecError;

fn left_op() -> GovernanceOp {
    GovernanceOp {
        author: PseudonymKey([7; 32]),
        seq: 3,
        prev: None,
        deps: Vec::new(),
        issued_at: 1_700_000_000,
        body: GovernanceEntry::MemberLeft { heads: Vec::new() },
        sig: [9; 64],
    }
}

/// `unsigned` (a canonical unsigned op) with its body's union
/// discriminant, the first 16 bits of the body's data section, set to
/// `arm`. Root pointer at word 0; the body is pointer 3 of the root.
fn with_arm(unsigned: &[u8], arm: u16) -> Vec<u8> {
    let target = |at: usize| {
        let ptr = &unsigned[at * 8..at * 8 + 8];
        let offset = i32::from_le_bytes(ptr[0..4].try_into().unwrap()) >> 2;
        let data_words = u16::from_le_bytes(ptr[4..6].try_into().unwrap());
        (
            at + 1 + usize::try_from(offset).unwrap(),
            usize::from(data_words),
        )
    };
    let (root, root_data) = target(0);
    let (body, _) = target(root + root_data + 3);
    let mut out = unsigned.to_vec();
    out[body * 8..body * 8 + 2].copy_from_slice(&arm.to_le_bytes());
    out
}

#[test]
fn a_sealed_known_op_reads_back_with_its_signed_bytes() {
    let op = left_op();
    let unsigned = op_unsigned_bytes(&op);
    let bytes = seal_op(&unsigned, &op.sig).unwrap();
    let ReadOp::Known(decoded) = read_op(&bytes).unwrap() else {
        panic!("a known op reads as known");
    };
    assert_eq!(decoded.op, op);
    assert_eq!(decoded.unsigned, unsigned);
}

#[test]
fn an_op_of_an_arm_beyond_the_schema_reads_with_the_bytes_its_author_signed() {
    let op = left_op();
    let unsigned = with_arm(&op_unsigned_bytes(&op), 0x7fff);
    let bytes = seal_op(&unsigned, &op.sig).unwrap();
    let ReadOp::Unknown(unknown) = read_op(&bytes).unwrap() else {
        panic!("an arm beyond the schema is not interpretable");
    };
    assert_eq!(unknown.what, Uninterpretable::Kind(0x7fff));
    assert_eq!(unknown.unsigned, unsigned, "the author's signed bytes");
    assert_eq!(unknown.author, op.author);
    assert_eq!((unknown.seq, unknown.sig), (op.seq, op.sig));
    assert!(matches!(
        decode_op(&bytes),
        Err(CodecError::UnknownGovernanceArm(_))
    ));
}
