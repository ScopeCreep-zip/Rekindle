//! Plan GI3: a relayed watch change is applied only when the value read
//! hashes to the observer's content hash.

use crate::host::runtime::tests::offline_host;

/// A value whose hash is not the relay's changes nothing; the value the
/// observer saw is applied.
#[tokio::test]
async fn a_relay_whose_hash_does_not_match_the_value_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let governance = host.governance();
    let seen = b"what the observer saw".to_vec();
    let seen_hash = blake3::hash(&seen).to_hex().to_string();
    assert!(
        !governance
            .apply_relayed_value("record", 1, &seen_hash, "observer", b"other".to_vec())
            .await,
        "a mismatching value is dropped"
    );
    assert!(
        governance
            .apply_relayed_value("record", 1, &seen_hash, "observer", seen)
            .await,
        "the observed value is applied"
    );
}
