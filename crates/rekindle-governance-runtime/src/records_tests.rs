use super::*;
use crate::deps::MockGovernanceRuntimeDeps;
use rekindle_records::lease::LeaseId;

/// The publish write is an empty value in the last subkey, under the
/// writer derived for that slot of the record's range.
#[tokio::test]
async fn publish_writes_empty_last_slot_under_its_derived_writer() {
    let seed = [9u8; 32];
    let range_start = 255;
    let expected = rekindle_secrets::derive::derive_slot_keypair(&seed, range_start + 254)
        .expect("derive")
        .verifying_key()
        .to_bytes();
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_format_writer_keypair()
        .returning(|public, _| hex::encode(public));
    deps.expect_set_dht_value()
        .withf(move |lease, subkey, value, writer| {
            *lease == LeaseId(7)
                && *subkey == 254
                && value.is_empty()
                && writer.as_deref() == Some(hex::encode(expected).as_str())
        })
        .times(1)
        .returning(|_, _, _, _| Ok(None));
    publish_created(&deps, LeaseId(7), &seed, range_start)
        .await
        .expect("published");
}

/// A newer value already at that subkey is a conflict, not success.
#[tokio::test]
async fn publish_conflict_is_an_error() {
    let mut deps = MockGovernanceRuntimeDeps::new();
    deps.expect_format_writer_keypair()
        .returning(|_, _| "kp".to_string());
    deps.expect_set_dht_value()
        .returning(|_, _, _, _| Ok(Some(vec![1])));
    assert!(matches!(
        publish_created(&deps, LeaseId(1), &[0u8; 32], 0).await,
        Err(GovernanceRuntimeError::WriteConflict(1))
    ));
}
