//! Our own op record's watch (ADR 0022 rule 4, plan E3.2 S7d-keys.9): each
//! device watches its own `M(p,d)`/2 in every community, since the ring
//! watches only successors and a copy of this data root writes there under
//! our key before any of its ops reach a peer. On each `ValueChange` the
//! entries above our signed head are read from the log, received as any
//! read op is (`op_receive::store_received_op`), merged, and the merged
//! device set is checked for a `DeviceRetired` of our own key
//! (`OpSigner::checked`, the check made before each sign), so signing stops
//! at the change, not at the next write.

use crate::deps::GovernanceRuntimeDeps;
use crate::device_signer::{device_op_key, OpSigner};
use crate::error::GovernanceRuntimeError;

/// Our op record `record` of `community_id` changed: receive the entries
/// above this device's signed head, rebuild when one was stored, and check
/// the merged state for a retirement of our key.
///
/// # Errors
/// [`GovernanceRuntimeError::OwnKeyRetired`] when the merged device set
/// retires our key (its text is the user's notice), or the read, the store
/// or the key lookup failed.
pub async fn own_record_changed<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    record: &str,
) -> Result<(), GovernanceRuntimeError> {
    let device = device_op_key(deps, community_id, false)?;
    let key = device.verifying_key().to_bytes();
    let head = deps.op_store().signed_here_head(community_id, &key).await?;
    let port = deps.record_port()?;
    let (read, pointer) =
        rekindle_records::chain::read_log_with_retirement(&*port, record, head.unwrap_or(0))
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("own op log: {e}")))?;
    let mut stored = false;
    for entry in read.entries {
        let received =
            crate::op_receive::store_received_op(deps, community_id, entry.ciphertext).await?;
        stored |= received.is_some();
    }
    if let Some(pointer) = pointer {
        stored |=
            crate::retirement_pointer::receive(deps, &*port, community_id, record, &key, pointer)
                .await?;
    }
    if stored {
        crate::dht_hydration::rebuild_governance(deps, community_id).await;
    }
    let state = deps.governance_state(community_id).unwrap_or_default();
    let me = crate::op_log::own_member(deps, community_id, &state)?;
    OpSigner::checked(community_id, &state, me.pseudonym, &device).map(|_| ())
}
