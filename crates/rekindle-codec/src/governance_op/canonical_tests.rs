//! The canonical unsigned form is the bytes an op's author signs and its
//! reader compares: one that cannot be computed is an error, never empty
//! bytes a signature would cover in its place (only `GOV_OP ‖ anchor`).

use rekindle_types::governance::{GovernanceEntry, GovernanceOp, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::{build, canonical, governance_op, op_unsigned_bytes, received_unsigned, unsigned_of};
use crate::CodecError;

fn op_with_deps() -> GovernanceOp {
    let dep = |b: u8| OpRef {
        op_hash: OpHash([b; 32]),
        device_key: [b; 32],
        author: PseudonymKey([b; 32]),
        author_record: format!("VLD0:{}", "A".repeat(43)),
        author_seq: u64::from(b),
    };
    GovernanceOp {
        device_key: [7; 32],
        author: PseudonymKey([7; 32]),
        seq: 3,
        prev: Some(OpHash([8; 32])),
        deps: vec![dep(1), dep(2)],
        issued_at: 1_700_000_000,
        body: GovernanceEntry::MemberLeft { heads: Vec::new() },
        sig: [9; 64],
    }
}

/// `each` applied to the op's root read back under every traversal limit
/// from zero words to past the whole op: read, or refused by its limit.
fn under_each_limit<T>(
    op: &GovernanceOp,
    mut each: impl FnMut(capnp::Result<governance_op::Reader<'_>>) -> T,
) -> Vec<T> {
    let builder = build(op);
    let segments = builder.get_segments_for_output();
    let words: usize = segments.iter().map(|s| s.len() / 8).sum();
    (0..=words + 8)
        .map(|limit| {
            let options =
                *capnp::message::ReaderOptions::new().traversal_limit_in_words(Some(limit));
            let message =
                capnp::message::Reader::new(capnp::message::SegmentArray::new(&segments), options);
            each(message.get_root::<governance_op::Reader<'_>>())
        })
        .collect()
}

#[test]
fn canonical_refuses_a_reader_past_its_traversal_limit() {
    let op = op_with_deps();
    let reference = op_unsigned_bytes(&op).unwrap();
    assert!(!reference.is_empty());
    let results = under_each_limit(&op, |root| root.ok().map(canonical));
    let refused = results.iter().flatten().filter(|r| r.is_err()).count();
    assert!(refused > 0, "some limit reads the root but not the op");
    for result in results.into_iter().flatten() {
        match result {
            Ok(bytes) => assert_eq!(bytes, reference, "a canonical form is the whole op"),
            Err(e) => assert!(matches!(e, CodecError::Deserialization(_)), "{e:?}"),
        }
    }
}

/// The signer's bytes (`op_unsigned_bytes`) and the reader's (`read_op`'s
/// received form and its recomputation) all come from `unsigned_of`: a
/// root it cannot read or canonicalize is refused, never `Ok` with empty
/// bytes for a signature to cover or a comparison to read.
#[test]
fn the_unsigned_form_is_refused_never_empty() {
    let op = op_with_deps();
    let reference = op_unsigned_bytes(&op).unwrap();
    let results = under_each_limit(&op, |root| {
        let received = root.as_ref().ok().map(|r| received_unsigned(*r));
        (unsigned_of(root), received)
    });
    let mut refused = 0;
    for (unsigned, received) in results {
        for result in std::iter::once(unsigned).chain(received) {
            match result {
                Ok(bytes) => assert_eq!(bytes, reference, "never empty, never partial"),
                Err(_) => refused += 1,
            }
        }
    }
    assert!(refused > 0, "a limit under the whole op is refused");
}
