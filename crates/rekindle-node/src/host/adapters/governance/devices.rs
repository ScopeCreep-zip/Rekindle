//! Our device-key acts on the Host (ADR 0022, plan E3.2 S7d-keys K5):
//! retire one of our own device keys (rules 4 and 5: a lost or stolen
//! device), or co-sign the re-admission of a member's fork-answering key
//! (rules 7 and 8). Each is built under the rebuild lock and written by
//! `rekindle_governance_runtime::device_ops`; the governance snapshot is
//! then persisted, as every op this adapter writes is.

use rekindle_governance_runtime::device_signer::{self, OwnDevice};
use rekindle_governance_runtime::{device_ops, GovernanceRuntimeError, OpOutcome};
use rekindle_types::dto::community::DeviceStatus;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// This device's standing in `community` for its view (lead decision
    /// 63): `NeedsReenrol` when our key is not endorsed for the current
    /// member record. A community not yet merged or admitted is `Enrolled`
    /// (nothing to re-enrol against yet).
    pub(crate) fn device_status(&self, community: &str) -> DeviceStatus {
        let Some(state) = self.governance_state_impl(community) else {
            return DeviceStatus::Enrolled;
        };
        match device_signer::own_device(self, community, &state) {
            Ok(OwnDevice::NeedsEnrol) => DeviceStatus::NeedsReenrol,
            Ok(OwnDevice::Endorsed) | Err(_) => DeviceStatus::Enrolled,
        }
    }

    /// The unlock catch-up's device check (ADR 0022 rule 4, lead decision
    /// 63), after the community's rebuild: a key no longer endorsed for the
    /// current member record drops its stale `M(p,d)` binding and refuses
    /// its writes (`DeviceUnendorsed`); the view then reads `NeedsReenrol`.
    pub(crate) fn catch_up_own_device(&self, community: &str) {
        let Some(state) = self.governance_state_impl(community) else {
            return;
        };
        match device_signer::catch_up_own_device(self, community, &state) {
            Ok(OwnDevice::NeedsEnrol) => {
                tracing::info!(community, "this device needs re-enrolment here");
            }
            Ok(OwnDevice::Endorsed) => {}
            Err(error) => tracing::debug!(community, %error, "device check skipped"),
        }
    }

    /// Enrol this device in `anchor` with a fresh device key and its own
    /// `M(p,d)` (`DeviceKey`, signed by `p`); answers the new key.
    ///
    /// # Errors
    /// `DeviceActRefused` for a device already endorsed, or the write's.
    pub(crate) async fn enrol_device(
        &self,
        anchor: &str,
    ) -> Result<(OpOutcome, [u8; 32]), GovernanceRuntimeError> {
        let outcome = Box::pin(device_ops::enrol(self, anchor)).await?;
        self.persist_governance_snapshot(anchor).await;
        let key =
            rekindle_governance_runtime::deps::GovernanceRuntimeDeps::device_op_key(self, anchor)?
                .ok_or_else(|| GovernanceRuntimeError::DeviceOpKeyMissing(anchor.to_string()))?;
        let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&key);
        Ok((outcome, key.verifying_key().to_bytes()))
    }

    /// Retire our device key `device_key` in `anchor` above the highest op
    /// of it we hold (`DeviceRetired`, signed by `p`).
    ///
    /// # Errors
    /// `DeviceActRefused` for this device's own key or a key we never
    /// endorsed, or the write's.
    pub(crate) async fn retire_device(
        &self,
        anchor: &str,
        device_key: [u8; 32],
    ) -> Result<OpOutcome, GovernanceRuntimeError> {
        let outcome = Box::pin(device_ops::retire(self, anchor, device_key)).await?;
        self.persist_governance_snapshot(anchor).await;
        Ok(outcome)
    }

    /// Co-sign the re-admission of `member`'s fork-answering device key
    /// `device_key` in `anchor` (`DeviceReadmit`, carried by our op).
    ///
    /// # Errors
    /// `DeviceActRefused` when the member's set holds no fork-answering
    /// endorsement of the key, `PermissionDenied` without the authority
    /// rule 8 asks, or the write's.
    pub(crate) async fn readmit_device(
        &self,
        anchor: &str,
        member: PseudonymKey,
        device_key: [u8; 32],
    ) -> Result<OpOutcome, GovernanceRuntimeError> {
        let outcome = Box::pin(device_ops::readmit(self, anchor, member, device_key)).await?;
        self.persist_governance_snapshot(anchor).await;
        Ok(outcome)
    }
}
