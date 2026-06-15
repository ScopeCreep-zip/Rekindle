//! Invite management — create, revoke.

use rekindle_types::dht_types::{InviteEntry, MANIFEST_INVITES};

use aws_lc_rs::rand::SecureRandom;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::super::CommunityService;

impl CommunityService {
    pub async fn create_invite(
        &self, gov_key: &str, created_by: &str, max_uses: u32, expires_secs: Option<u64>,
    ) -> Result<String, ChatError> {
        self.require_operator(gov_key)?;
        let mut code_bytes = [0u8; 16];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut code_bytes)
            .map_err(|e| ChatError::Internal(format!("rng: {e}")))?;
        let invite_code = hex::encode(code_bytes);
        let code_hash = hex::encode(blake3::hash(invite_code.as_bytes()).as_bytes());
        let now = timestamp_ms();
        let expires_at = expires_secs.map(|dur| now + dur * 1000);
        let entry = InviteEntry {
            code_hash, created_by: created_by.to_string(), created_at: now,
            expires_at, max_uses, use_count: 0, encrypted_secrets: None,
        };
        let mut invites = self.read_invites(gov_key).await?;
        invites.push(entry);
        let bytes = serde_json::to_vec(&invites).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_INVITES, &bytes).await?;
        self.notify_governance_updated(gov_key, MANIFEST_INVITES).await;
        Ok(invite_code)
    }

    pub async fn revoke_invite(&self, gov_key: &str, invite_code: &str) -> Result<(), ChatError> {
        self.require_operator(gov_key)?;
        let code_hash = hex::encode(blake3::hash(invite_code.as_bytes()).as_bytes());
        let mut invites = self.read_invites(gov_key).await?;
        invites.retain(|inv| inv.code_hash != code_hash);
        let bytes = serde_json::to_vec(&invites).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.write_signed_governance(gov_key, MANIFEST_INVITES, &bytes).await?;
        self.notify_governance_updated(gov_key, MANIFEST_INVITES).await;
        Ok(())
    }
}
