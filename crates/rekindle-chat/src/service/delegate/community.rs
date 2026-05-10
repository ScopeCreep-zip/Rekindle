//! Community delegation — create, join, leave, list, info, approve, reject.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn create_community(
        &self, name: &str, description: &str,
    ) -> Result<crate::community::create::CommunityCreated, ChatError> {
        self.community.create_community(name, description).await
    }

    pub async fn join_community(
        &self, governance_key: &str,
    ) -> Result<crate::community::join::JoinCompleted, ChatError> {
        self.community.join_community(governance_key).await
    }

    pub async fn leave_community(&self, governance_key: &str) -> Result<(), ChatError> {
        self.community.leave_community(governance_key).await
    }

    pub fn list_communities(
        &self,
    ) -> Vec<crate::community::membership::CommunitySummary> {
        self.community.list_communities()
    }

    pub async fn community_info(
        &self, governance_key: &str,
    ) -> Result<CommunityInfoResult, ChatError> {
        let metadata = self.community.read_metadata(governance_key).await?;
        let channels = self.community.read_channels(governance_key).await?;
        let roles = self.community.read_roles(governance_key).await?;
        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(governance_key).cloned()
        };
        Ok(CommunityInfoResult {
            name: metadata.name,
            description: metadata.description.unwrap_or_default(),
            owner_pseudonym: metadata.owner_pseudonym,
            join_policy: format!("{:?}", metadata.join_policy),
            channel_count: channels.len(),
            role_count: roles.len(),
            our_pseudonym: membership.as_ref().map(|m| m.pseudonym_key.clone()).unwrap_or_default(),
            is_operator: membership.as_ref().is_some_and(|m| m.is_operator),
            locked_down: membership.as_ref().is_some_and(|m| m.locked_down),
        })
    }

    pub async fn approve_member(
        &self, governance_key: &str, member_pseudonym: &str,
    ) -> Result<(), ChatError> {
        self.community.approve_member(governance_key, member_pseudonym).await.map(|_| ())
    }

    pub async fn reject_member(
        &self, governance_key: &str, member_pseudonym: &str, reason: &str,
    ) -> Result<(), ChatError> {
        self.community.reject_member(governance_key, member_pseudonym, reason).await
    }

    pub async fn pending_members(
        &self, governance_key: &str,
    ) -> Result<Vec<rekindle_types::dht_types::PendingJoinEntry>, ChatError> {
        self.community.pending_members(governance_key).await
    }

    pub async fn transfer_ownership(
        &self, governance_key: &str, new_owner: &str,
    ) -> Result<(), ChatError> {
        self.community.transfer_ownership(governance_key, new_owner).await
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunityInfoResult {
    pub name: String,
    pub description: String,
    pub owner_pseudonym: String,
    pub join_policy: String,
    pub channel_count: usize,
    pub role_count: usize,
    pub our_pseudonym: String,
    pub is_operator: bool,
    pub locked_down: bool,
}
