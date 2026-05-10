//! System delegation — announcements, moderation alerts, bootstrap, sync.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn system_message(&self, gov: &str, body: &str) -> Result<(), ChatError> {
        self.community.system_message(gov, body).await
    }

    pub async fn raid_alert(&self, gov: &str, active: bool) -> Result<(), ChatError> {
        self.community.raid_alert(gov, active).await
    }

    pub async fn channel_lockdown(&self, gov: &str, locked: bool) -> Result<(), ChatError> {
        self.community.channel_lockdown(gov, locked).await
    }

    pub async fn kicked_notification(&self, gov: &str, target: &str) -> Result<(), ChatError> {
        self.community.kicked_notification(gov, target).await
    }

    pub async fn bootstrap_request(&self, gov: &str) -> Result<(), ChatError> {
        self.community.bootstrap_request(gov).await
    }

    pub async fn bootstrap_response(
        &self, gov: &str, target: &str, governance_entries: Vec<Vec<u8>>,
        member_list: Vec<Vec<u8>>, channel_meks: Vec<Vec<u8>>,
        recent_messages: Vec<Vec<u8>>, wrapped_owner_keypair: Vec<u8>,
    ) -> Result<(), ChatError> {
        self.community.bootstrap_response(gov, target, governance_entries, member_list, channel_meks, recent_messages, wrapped_owner_keypair).await
    }

    pub async fn sync_request(&self, gov: &str, channel_id: &str, since: u64) -> Result<(), ChatError> {
        self.community.sync_request(gov, channel_id, since).await
    }

    pub async fn sync_response(
        &self, gov: &str, target: &str, channel_id: &str, messages: Vec<Vec<u8>>,
    ) -> Result<(), ChatError> {
        self.community.sync_response(gov, target, channel_id, messages).await
    }
}
