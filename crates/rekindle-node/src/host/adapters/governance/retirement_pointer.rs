//! S7d-keys.9p2. The retired record's owner survives the vault cutover
//! until the governance outbox has published the locator on that record.

use rekindle_governance_runtime::{device_signer, GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_records::chain::{publish_device_retirement, DeviceRetirementPointer};
use rekindle_types::governance::GovernanceEntry;
use rekindle_vault::VaultKey;
use zeroize::Zeroizing;

use super::HostGovernance;

fn owner_key(community: &str, device: &[u8; 32]) -> VaultKey {
    VaultKey::PendingRetirementOwner {
        community: community.to_owned(),
        device: hex::encode(device),
    }
}

impl HostGovernance {
    pub(super) fn stage_retirement_owner(
        &self,
        community: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let device = device_signer::device_op_key(self, community, false)?;
        let (record, writer) = self
            .own_op_record_writer(community)
            .map_err(GovernanceRuntimeError::Adapter)?;
        let bytes = Zeroizing::new(format!("{record}\n{}", writer.as_str()));
        let guard = self.ctx.device_vault.lock();
        let vault = guard
            .as_ref()
            .ok_or_else(|| GovernanceRuntimeError::Adapter("device vault is locked".into()))?;
        vault
            .put(
                &owner_key(community, &device.verifying_key().to_bytes()),
                bytes.as_bytes(),
            )
            .map_err(|error| {
                GovernanceRuntimeError::Adapter(format!("stage retirement owner: {error}"))
            })
    }

    /// Complete an outbox attempt after the successor op and its frontier
    /// land. Retry both writes through the same row across unlocks.
    pub(crate) async fn publish_retirement_pointer(
        &self,
        community: &str,
        op: &[u8],
    ) -> Result<(), String> {
        let decoded = rekindle_codec::governance_op::decode_op(op)
            .map_err(|error| format!("read retirement carrier: {error}"))?;
        let GovernanceEntry::DeviceReEnrol(re_enrol) = &decoded.op.body else {
            return Ok(());
        };
        let key = owner_key(community, &re_enrol.retired.device_key);
        let bytes = {
            let guard = self.ctx.device_vault.lock();
            let vault = guard.as_ref().ok_or("device vault is locked")?;
            vault
                .get(&key)
                .map_err(|error| format!("read retirement owner: {error}"))?
        };
        // Older queued rows never staged an owner. A retried row can also
        // reach here after its pointer landed and its staged owner was deleted.
        let Some(bytes) = bytes else {
            return Ok(());
        };
        let (record, writer) = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|value| value.split_once('\n'))
            .ok_or("malformed staged retirement record and owner")?;
        let pointer = DeviceRetirementPointer {
            op_hash: rekindle_governance::op::op_hash(&decoded.unsigned, &decoded.op.sig).0,
            successor_key: decoded.op.device_key,
            successor_record: re_enrol.endorsement.device_record.clone(),
        };
        let port = GovernanceRuntimeDeps::record_port(self).map_err(|error| error.to_string())?;
        publish_device_retirement(&*port, record, writer, &pointer)
            .await
            .map_err(|error| format!("publish retirement pointer: {error}"))?;
        let guard = self.ctx.device_vault.lock();
        let vault = guard.as_ref().ok_or("device vault is locked")?;
        vault
            .delete(&key)
            .map_err(|error| format!("clear retirement owner: {error}"))
    }
}
