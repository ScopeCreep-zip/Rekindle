use base64::Engine as _;

/// Guards the choice of alphabet/padding, not base64 itself: the
/// invite blob is written with STANDARD, and a mismatch here would
/// surface as a confusing decryption failure rather than a decode
/// error.
#[test]
fn invite_blob_uses_standard_base64() {
    let enc = base64::engine::general_purpose::STANDARD;
    assert_eq!(enc.decode("Zm9vYmFy").unwrap(), b"foobar");
    assert_eq!(enc.decode("Zg==").unwrap(), b"f");
    assert!(enc.decode("Zm9v!!").is_err());
}

/// A walk missing a cited dep fails the join with the missing op's
/// author record and seq; a complete walk passes.
#[test]
fn a_walk_missing_a_dep_fails_the_join_naming_it() {
    use crate::error::GovernanceRuntimeError;
    use crate::op_log::VerifiedSet;
    use rekindle_types::governance::{OpHash, OpRef};
    use rekindle_types::id::PseudonymKey;

    assert!(super::refuse_incomplete(&VerifiedSet::default()).is_ok());
    let set = VerifiedSet {
        missing: vec![OpRef {
            op_hash: OpHash([7; 32]),
            author: PseudonymKey([1; 32]),
            author_record: "VLD0:leaver".to_owned(),
            author_seq: 1,
            device_key: [2; 32],
        }],
        ..VerifiedSet::default()
    };
    let refused = super::refuse_incomplete(&set).expect_err("a partial walk is refused");
    assert!(
        matches!(
            &refused,
            GovernanceRuntimeError::HistoryIncomplete { author_record, author_seq: 1 }
                if author_record == "VLD0:leaver"
        ),
        "{refused:?}"
    );
    assert!(refused
        .to_string()
        .starts_with("This community's history is incomplete on the network"));
}
