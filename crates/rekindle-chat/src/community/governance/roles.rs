//! Role management — create, update, delete, assign, unassign.
//!
//! Roles define permission sets for community members. Each role has a unique ID,
//! a name, color, permissions bitmask, and position (for hierarchy ordering).
//! Role assignment/unassignment modifies the member registry and broadcasts
//! MemberRolesChanged gossip for immediate peer visibility.

use rekindle_types::dht_types::{
    RoleEntry, MANIFEST_ROLES, REGISTRY_MEMBER_INDEX,
};
use rekindle_types::gossip_payload::ControlPayload;

use crate::io::Confirm;
use crate::ChatError;
use super::super::CommunityService;

impl CommunityService {
    pub async fn create_role(
        &self, gov_key: &str, name: &str, permissions: u64, color: u32, position: i32,
    ) -> Result<RoleEntry, ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
        let mut roles = self.read_roles(gov_key).await?;
        let next_id = roles.iter().map(|r| r.id).max().map_or(1, |m| m + 1);
        let role = RoleEntry {
            id: next_id, name: name.to_string(), color, permissions, position,
            hoist: false, mentionable: true, self_assignable: false,
        };
        roles.push(role.clone());
        let bytes = serde_json::to_vec(&roles).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_ROLES, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_governance_updated(gov_key, MANIFEST_ROLES).await;
        Ok(role)
    }

    pub async fn update_role(
        &self, gov_key: &str, role_id: u32, name: Option<&str>, permissions: Option<u64>, color: Option<u32>,
    ) -> Result<RoleEntry, ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
        let mut roles = self.read_roles(gov_key).await?;
        let role = roles.iter_mut().find(|r| r.id == role_id)
            .ok_or_else(|| ChatError::Internal(format!("role {role_id} not found")))?;
        if let Some(n) = name { role.name = n.to_string(); }
        if let Some(p) = permissions { role.permissions = p; }
        if let Some(c) = color { role.color = c; }
        let updated = role.clone();
        let bytes = serde_json::to_vec(&roles).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_ROLES, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_governance_updated(gov_key, MANIFEST_ROLES).await;
        Ok(updated)
    }

    pub async fn delete_role(&self, gov_key: &str, role_id: u32) -> Result<(), ChatError> {
        let keypair = self.require_governance_keypair(gov_key)?;
        let mut roles = self.read_roles(gov_key).await?;
        roles.retain(|r| r.id != role_id);
        let bytes = serde_json::to_vec(&roles).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(gov_key, MANIFEST_ROLES, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_governance_updated(gov_key, MANIFEST_ROLES).await;
        Ok(())
    }

    pub async fn assign_role(
        &self, gov_key: &str, member_pseudonym: &str, role_id: u32,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut members = self.read_members(&membership.registry_key).await?;
        if let Some(m) = members.iter_mut().find(|m| m.pseudonym_key == member_pseudonym) {
            if !m.role_ids.contains(&role_id) { m.role_ids.push(role_id); }
        }
        let bytes = serde_json::to_vec(&members).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(&membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_membership(gov_key, ControlPayload::MemberRolesChanged {
            pseudonym_key: member_pseudonym.into(), role_ids: vec![role_id],
        }).await;
        Ok(())
    }

    pub async fn unassign_role(
        &self, gov_key: &str, member_pseudonym: &str, role_id: u32,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(gov_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut members = self.read_members(&membership.registry_key).await?;
        if let Some(m) = members.iter_mut().find(|m| m.pseudonym_key == member_pseudonym) {
            m.role_ids.retain(|&id| id != role_id);
        }
        let bytes = serde_json::to_vec(&members).map_err(|e| ChatError::Serialization(format!("{e}")))?;
        self.io.write_record(&membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), Confirm::Accepted).await?;
        self.notify_membership(gov_key, ControlPayload::MemberRolesChanged {
            pseudonym_key: member_pseudonym.into(), role_ids: vec![],
        }).await;
        Ok(())
    }
}
