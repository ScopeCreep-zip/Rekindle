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
        let gov_key = {
            let meta = self.session_meta.read();
            meta.resolve_community(governance_key)
                .map(|(_, m)| m.governance_key.clone())
                .unwrap_or_else(|| governance_key.to_string())
        };
        let governance_key = &gov_key;
        let metadata = self.community.read_metadata(governance_key).await?;
        let channels = self.community.read_channels(governance_key).await?;
        let roles = self.community.read_roles(governance_key).await?;
        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(governance_key).cloned()
        };
        let raw_members = self.community.list_members(governance_key).await.unwrap_or_default();
        let state_guard = self.pipeline.state().read();
        let presence_map: std::collections::HashMap<String, String> = state_guard
            .presence.community_members(governance_key)
            .into_iter()
            .map(|(pseudo, info)| (pseudo, info.effective_status().to_string()))
            .collect();
        let channel_unreads: std::collections::HashMap<String, u32> = channels.iter()
            .filter_map(|ch| {
                let count = state_guard.unread.channels
                    .get(&(governance_key.to_string(), ch.id.clone()))
                    .copied()
                    .unwrap_or(0);
                if count > 0 { Some((ch.id.clone(), count)) } else { None }
            })
            .collect();
        drop(state_guard);

        let role_names: std::collections::HashMap<u32, String> = roles.iter()
            .map(|r| (r.id, r.name.clone()))
            .collect();

        let members: Vec<rekindle_types::display::MemberWithPresence> = raw_members.into_iter()
            .map(|m| {
                let status = presence_map.get(&m.pseudonym_key)
                    .cloned()
                    .unwrap_or_else(|| "offline".to_string());
                let role_name = m.role_ids.first()
                    .and_then(|id| role_names.get(id))
                    .cloned();
                rekindle_types::display::MemberWithPresence { member: m, status, role_name }
            })
            .collect();

        let channel_count = channels.len();
        let role_count = roles.len();
        let member_count = members.len();
        Ok(CommunityInfoResult {
            governance_key: governance_key.to_string(),
            name: metadata.name,
            description: metadata.description.unwrap_or_default(),
            owner_pseudonym: metadata.owner_pseudonym,
            join_policy: format!("{:?}", metadata.join_policy),
            created_at: metadata.created_at,
            channels,
            channel_unreads,
            roles,
            members,
            channel_count,
            role_count,
            member_count,
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
    pub governance_key: String,
    pub name: String,
    pub description: String,
    pub owner_pseudonym: String,
    pub join_policy: String,
    pub created_at: u64,
    pub channels: Vec<rekindle_types::dht_types::ChannelEntry>,
    /// Channel UUID → unread message count from subscription state.
    #[serde(default)]
    pub channel_unreads: std::collections::HashMap<String, u32>,
    pub roles: Vec<rekindle_types::dht_types::RoleEntry>,
    pub members: Vec<rekindle_types::display::MemberWithPresence>,
    pub channel_count: usize,
    pub role_count: usize,
    pub member_count: usize,
    pub our_pseudonym: String,
    pub is_operator: bool,
    pub locked_down: bool,
}
