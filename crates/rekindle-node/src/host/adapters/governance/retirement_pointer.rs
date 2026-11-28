//! S7d-keys.9p2. The retired record's owner survives the vault cutover
//! until the governance outbox has published the locator on that record.

use rekindle_governance_runtime::deps::GovernanceOpStore;
use rekindle_governance_runtime::op_log::HeldOp;
use rekindle_governance_runtime::{GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_records::chain::{publish_device_retirement, DeviceRetirementPointer};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_vault::typed::device_keys::{self, DeviceRecordKeys};
use rekindle_vault::VaultKey;
use zeroize::Zeroizing;

use super::HostGovernance;

fn owner_key(community: &str, device: &[u8; 32]) -> VaultKey {
    VaultKey::PendingRetirementOwner {
        community: community.to_owned(),
        device: hex::encode(device),
    }
}

/// The staged handoff: the retired record, its writer and the signed carrier
/// (`commit_retirement_carrier`, its only writer, always stores all three).
fn split_handoff(bytes: &[u8]) -> Result<(&str, &str, &[u8]), String> {
    let mut fields = bytes.splitn(3, |byte| *byte == b'\n');
    let record = fields
        .next()
        .and_then(|field| std::str::from_utf8(field).ok());
    let writer = fields
        .next()
        .and_then(|field| std::str::from_utf8(field).ok());
    match (record, writer, fields.next()) {
        (Some(record), Some(writer), Some(carrier)) if !carrier.is_empty() => {
            Ok((record, writer, carrier))
        }
        _ => Err("malformed staged retirement handoff".into()),
    }
}

fn adapter(error: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("retirement cutover: {error}"))
}

impl HostGovernance {
    pub(super) fn commit_retirement_carrier(
        &self,
        community: &str,
        retired: [u8; 32],
        secret: &[u8; 32],
        owner: &str,
        carrier: &HeldOp,
    ) -> Result<(), GovernanceRuntimeError> {
        let (record, writer) = self.own_op_record_writer(community).map_err(adapter)?;
        let mut handoff = Zeroizing::new(format!("{record}\n{}\n", writer.as_str()).into_bytes());
        handoff.extend_from_slice(&carrier.encoded);
        let guard = self.ctx.device_vault.lock();
        let vault = guard
            .as_ref()
            .ok_or_else(|| adapter("device vault is locked"))?;
        let current = device_keys::load_device_op_key(vault, community)
            .map_err(adapter)?
            .ok_or_else(|| adapter("device key is missing"))?;
        if SigningKey::from_bytes(&current).verifying_key().to_bytes() != retired {
            return Err(adapter("device key changed during re-enrolment"));
        }
        let binding = DeviceRecordKeys {
            record: carrier.author_record.clone(),
            owner: Zeroizing::new(owner.to_owned()),
        };
        device_keys::commit_re_enrolment(vault, community, retired, secret, &binding, &handoff)
            .map_err(adapter)
    }

    /// Restore every signed cutover before the network unlock starts.
    /// The existing governance stream deduplicates by the op hash.
    pub(crate) async fn resume_retirement_carriers(&self) -> Result<(), String> {
        let staged = {
            let guard = self.ctx.device_vault.lock();
            let vault = guard.as_ref().ok_or("device vault is locked")?;
            vault
                .list("device")
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter_map(|key| match key {
                    VaultKey::PendingRetirementOwner { ref community, .. } => {
                        Some(vault.get(&key).map(|bytes| (community.clone(), bytes)))
                    }
                    _ => None,
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        let queue = self
            .ctx
            .outbox
            .governance_ops()
            .ok_or("the outbox is not running")?;
        for (community, bytes) in staged {
            if self.governance_state_impl(&community).is_none() {
                continue;
            }
            let Some(bytes) = bytes else { continue };
            let (_, _, encoded) = split_handoff(&bytes)?;
            let decoded = rekindle_codec::governance_op::decode_op(encoded)
                .map_err(|error| format!("read retirement carrier: {error}"))?;
            let GovernanceEntry::DeviceReEnrol(re_enrol) = &decoded.op.body else {
                return Err("staged retirement carrier is not a re-enrolment".into());
            };
            let record = &re_enrol.endorsement.device_record;
            let hash = rekindle_governance::op::op_hash(&decoded.unsigned, &decoded.op.sig);
            let carrier =
                rekindle_governance_runtime::op_receive::held_at(hash, record, encoded.to_vec())
                    .ok_or("staged retirement carrier is invalid")?;
            GovernanceOpStore::store_signed_governance_op(self, &community, &carrier)
                .await
                .map_err(|error| error.to_string())?;
            let barrier = self.ctx.communities.unlock_barriers().of(&community);
            let id = hex::encode(hash.0);
            barrier.signed(&id);
            if let Err(error) = queue
                .resume_carrier(&community, record, carrier.encoded)
                .await
            {
                barrier.settled(&id);
                return Err(error);
            }
            self.install_resumed_carrier(&community).await?;
        }
        Ok(())
    }

    async fn install_resumed_carrier(&self, community: &str) -> Result<(), String> {
        // The pool may be unavailable during an offline unlock. All carrier
        // dependencies were held when it was signed, so project from disk.
        let held = GovernanceOpStore::governance_ops(self, community)
            .await
            .map_err(|error| error.to_string())?;
        let cert = self
            .genesis_cert(community)
            .ok_or("no genesis cert is pinned")?;
        let set = rekindle_governance_runtime::op_log::verify_held_with(self, community, &held);
        self.set_governance_state(
            community,
            rekindle_governance::merge::merge(&cert, &set.ops),
        );
        Ok(())
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
        // A retried row reaches here after its pointer landed and its staged
        // handoff was deleted.
        let Some(bytes) = bytes else {
            return Ok(());
        };
        let (record, writer, _) = split_handoff(&bytes)?;
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
