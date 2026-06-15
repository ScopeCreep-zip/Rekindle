//! Moderation — ban, unban, kick, timeout.

use rekindle_types::dht_types::{BanEntry, MANIFEST_BANS};
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
        // Add to ban list (signed governance)
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
        self.write_signed_governance(gov_key, MANIFEST_BANS, &bytes).await?;

        // Clear the member's registry slot
        let members = self.read_members(&membership.registry_key).await?;
        if let Some(member) = members.iter().find(|m| m.pseudonym_key == target) {
            let slot_kp_bytes = self.derive_member_slot_keypair_bytes(&membership, member.subkey_index)?;
            self.io.open_and_write(
                &membership.registry_key, member.subkey_index, b"",
                Some(&slot_kp_bytes), Confirm::Accepted,
            ).await?;
        }

        // Rekey all channels for forward secrecy
        let remaining = self.read_members(&membership.registry_key).await?;
        self.rekey_all_channels(gov_key, &membership.registry_key, &remaining).await;

        self.notify_membership(gov_key, ControlPayload::Ban { target_pseudonym: target.into() }).await;
        self.append_audit_entry(gov_key, "ban", banned_by, Some(target), reason).await;
        tracing::info!(target = %&target[..12.min(target.len())], "banned + rekeyed + notified + audited");
        Ok(())
    }

    pub async fn unban_member(&self, gov_key: &str, target: &str) -> Result<(), ChatError> {
        let operator = self.io.pseudonym_hex(gov_key)?;
        let mut bans = self.read_bans(gov_key).await?;
        bans.retain(|b| b.pseudonym_key != target);
        let bytes = serde_json::to_vec(&bans).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_BANS, &bytes).await?;
        self.notify_membership(gov_key, ControlPayload::Unban { target_pseudonym: target.into() }).await;
        self.append_audit_entry(gov_key, "unban", &operator, Some(target), None).await;
        Ok(())
    }

    pub async fn kick_member(&self, gov_key: &str, target: &str) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;

        // Clear the member's registry slot
        let members = self.read_members(&membership.registry_key).await?;
        if let Some(member) = members.iter().find(|m| m.pseudonym_key == target) {
            let slot_kp_bytes = self.derive_member_slot_keypair_bytes(&membership, member.subkey_index)?;
            self.io.open_and_write(
                &membership.registry_key, member.subkey_index, b"",
                Some(&slot_kp_bytes), Confirm::Accepted,
            ).await?;
        }

        self.notify_membership(gov_key, ControlPayload::Kick { target_pseudonym: target.into() }).await;
        self.append_audit_entry(gov_key, "kick", &membership.pseudonym_key, Some(target), None).await;
        tracing::info!(target = %&target[..12.min(target.len())], "kicked + notified + audited");
        Ok(())
    }

    pub async fn timeout_member(
        &self, gov_key: &str, target: &str, duration_secs: u64,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;

        // Find member, update timeout_until, write back to their slot
        let members = self.read_members(&membership.registry_key).await?;
        if let Some(member) = members.iter().find(|m| m.pseudonym_key == target) {
            let mut updated = member.clone();
            updated.timeout_until = Some(timestamp_ms() + duration_secs * 1000);
            let slot_kp_bytes = self.derive_member_slot_keypair_bytes(&membership, member.subkey_index)?;
            let bytes = serde_json::to_vec(&updated).map_err(|e| ChatError::Serialization(format!("{e}")))?;
            self.io.open_and_write(
                &membership.registry_key, member.subkey_index, &bytes,
                Some(&slot_kp_bytes), Confirm::Accepted,
            ).await?;
        }

        self.notify_membership(gov_key, ControlPayload::TimeoutMember {
            target_pseudonym: target.into(), duration_seconds: duration_secs, reason: None,
        }).await;
        self.append_audit_entry(
            gov_key, "timeout", &membership.pseudonym_key, Some(target),
            Some(&format!("{}s", duration_secs)),
        ).await;
        Ok(())
    }

    /// Derive slot keypair bytes for a target member's slot.
    /// The operator derives any member's keypair from the shared slot_seed.
    pub(crate) fn derive_member_slot_keypair_bytes(
        &self,
        membership: &rekindle_types::session_types::CommunityMembership,
        slot_index: u32,
    ) -> Result<Vec<u8>, ChatError> {
        let slot_seed = membership.slot_seed.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("slot_seed not available".into()))?;
        let kp = rekindle_identity::derive_slot_keypair(&slot_seed, slot_index)
            .map_err(|e| ChatError::Internal(format!("slot keypair: {e}")))?;
        let mut buf = vec![0u8; 64];
        buf[..32].copy_from_slice(&kp.public_key_bytes());
        let mut ikm = Vec::with_capacity(36);
        ikm.extend_from_slice(&slot_seed);
        ikm.extend_from_slice(&slot_index.to_le_bytes());
        let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
        buf[32..].copy_from_slice(&seed);
        Ok(buf)
    }
}
