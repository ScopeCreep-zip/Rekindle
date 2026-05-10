//! Key management delegation — MEK operations, prekey replenishment.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub fn mek_list(&self, community: &str) -> Vec<crate::crypto::mek::MekSnapshot> {
        self.mek_cache.snapshot(community)
    }

    pub async fn mek_rotate(
        &self, community: &str, channel: &str,
    ) -> Result<u64, ChatError> {
        self.community.rotate_mek(community, channel).await
    }

    pub async fn mek_request(
        &self, community: &str, channel: &str, generation: u64,
    ) -> Result<(), ChatError> {
        self.community.request_mek_from_operator(community, channel, generation).await
    }

    pub async fn prekey_replenish(&self) -> Result<u32, ChatError> {
        self.identity.replenish_prekeys().await
    }
}
