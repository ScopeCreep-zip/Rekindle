//! Moderation — ban, unban, kick, timeout.
//!
//! Ban includes forward-secrecy rekey of all channel MEKs to ensure the banned
//! member cannot decrypt any messages sent after their ban. Kick removes from
//! the member index without banning (they can rejoin). Timeout temporarily
//! restricts the member's ability to send messages.

use rekindle_types::dht_types::{
    BanEntry, MANIFEST_BANS, REGISTRY_MEMBER_INDEX,
};
use rekindle_types::gossip_payload::ControlPayload;

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::super::CommunityService;

impl CommunityService {
    pub async fn ban_member(
        &self, gov_key: &str, target: &str, reason: Option<&str>, banned_by: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;
        let gov_keypair = self.require_governance_keypair(gov_key)?;
        let reg_keypair = self.require_registry_keypair(&membership.registry_key)?;

        // Add to ban list
        let mut bans = self.read_bans(gov_key).await?;
        if bans.iter().any(|b| b.pseudonym_key == target) {
            return Err(ChatError::Internal(format!("{target} already banned")));
        }
        bans.push(BanEntry {
            pseudonym_key: target.to_string(),
            reason: reason.map(String::from),
            banned_by: banned_by.to_string(),
            banned_at: timestamp_ms(),
        });
        let bytes = serde_json::to_vec(&bans).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_BANS, &bytes, Some(&gov_keypair), Confirm::Accepted).await?;

        // Remove from member index
        let mut members = self.read_members(&membership.registry_key).await?;
        members.retain(|m| m.pseudonym_key != target);
        let bytes = serde_json::to_vec(&members).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(&membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&reg_keypair), Confirm::Accepted).await?;

        // Rekey all channels for forward secrecy
        self.rekey_all_channels(gov_key, &membership.registry_key, &members).await;

        self.notify_membership(gov_key, ControlPayload::Ban { target_pseudonym: target.into() }).await;
        tracing::info!(target = %&target[..12.min(target.len())], "banned + rekeyed + notified");
        Ok(())
    }

    pub async fn unban_member(&self, gov_key: &str, target: &str) -> Result<(), ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
        let mut bans = self.read_bans(gov_key).await?;
        bans.retain(|b| b.pseudonym_key != target);
        let bytes = serde_json::to_vec(&bans).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_BANS, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_membership(gov_key, ControlPayload::Unban { target_pseudonym: target.into() }).await;
        Ok(())
    }

    pub async fn kick_member(&self, gov_key: &str, target: &str) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut members = self.read_members(&membership.registry_key).await?;
        members.retain(|m| m.pseudonym_key != target);
        let bytes = serde_json::to_vec(&members).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(&membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_membership(gov_key, ControlPayload::Kick { target_pseudonym: target.into() }).await;
        tracing::info!(target = %&target[..12.min(target.len())], "kicked + notified");
        Ok(())
    }

    pub async fn timeout_member(
        &self, gov_key: &str, target: &str, duration_secs: u64,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut members = self.read_members(&membership.registry_key).await?;
        if let Some(m) = members.iter_mut().find(|m| m.pseudonym_key == target) {
            m.timeout_until = Some(timestamp_ms() + duration_secs * 1000);
        }
        let bytes = serde_json::to_vec(&members).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(&membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_membership(gov_key, ControlPayload::TimeoutMember {
            target_pseudonym: target.into(), duration_seconds: duration_secs, reason: None,
        }).await;
        Ok(())
    }
}
