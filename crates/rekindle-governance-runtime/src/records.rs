//! One-shot inspects on a record borrowed for the call (plan C7.5); a
//! one-shot read has the same shape in [`crate::invite_secrets`].
//! While the community holds a record for its session, the borrow is a
//! table hit with no Veilid call.

use rekindle_records::port::OpenIntent;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// [`GovernanceRuntimeDeps::inspect_dht_record_present_subkeys`] on a borrow.
pub(crate) async fn inspect_present_subkeys<D: GovernanceRuntimeDeps>(
    deps: &D,
    record_key: &str,
) -> Result<Vec<u32>, GovernanceRuntimeError> {
    let lease = deps
        .acquire_record(record_key, None, OpenIntent::Check)
        .await?;
    let present = deps.inspect_dht_record_present_subkeys(lease).await;
    deps.release_record(lease).await;
    present
}

/// Publish a record just created (plan C7.6d). A create is local only: the
/// network learns the record from its first set (`create_record.rs`), so
/// until then every other node gets `Key not found`. Write an empty value to
/// the last subkey under that slot's writer, derived from the community's
/// shared slot seed. Readers treat an empty value as absent, and a
/// joiner claims that slot last. The write must be stored.
pub(crate) async fn publish_created<D: GovernanceRuntimeDeps>(
    deps: &D,
    lease: rekindle_records::lease::LeaseId,
    slot_seed: &[u8; 32],
) -> Result<(), GovernanceRuntimeError> {
    let last = rekindle_types::dht_layout::REGISTRY_SLOTS - 1;
    let slot = rekindle_secrets::derive::derive_slot_keypair(slot_seed, last)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("derive publish slot: {e}")))?;
    let writer = deps.format_writer_keypair(slot.verifying_key().to_bytes(), slot.to_bytes());
    match deps
        .set_dht_value(lease, last, Vec::new(), Some(writer))
        .await?
    {
        None => Ok(()),
        Some(newer) => Err(GovernanceRuntimeError::WriteConflict(newer.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::MockGovernanceRuntimeDeps;
    use rekindle_records::lease::LeaseId;

    /// The publish write is an empty value in the last subkey, under the
    /// writer derived for that slot.
    #[tokio::test]
    async fn publish_writes_empty_last_slot_under_its_derived_writer() {
        let seed = [9u8; 32];
        let expected = rekindle_secrets::derive::derive_slot_keypair(&seed, 254)
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
        publish_created(&deps, LeaseId(7), &seed)
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
            publish_created(&deps, LeaseId(1), &[0u8; 32]).await,
            Err(GovernanceRuntimeError::WriteConflict(1))
        ));
    }
}
