//! Audit log types for the community audit record.
//!
//! The audit record key is stored as a pointer in manifest subkey 14.
//! The audit record itself is a DFLT ring buffer.

use serde::{Deserialize, Serialize};

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntry {
    pub entry_id: u64,
    pub actor_pseudonym: String,
    pub action: AuditAction,
    pub target: AuditTarget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<AuditChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub timestamp: u64,
    pub signature: Vec<u8>,
}

/// Moderation/administrative action type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuditAction {
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,
    RoleCreate,
    RoleUpdate,
    RoleDelete,
    MemberRoleUpdate,
    MemberKick,
    MemberBan,
    MemberUnban,
    MemberTimeout,
    MemberTimeoutRemove,
    MessageDelete,
    MessagePin,
    CommunityUpdate,
    AutoModRuleCreate,
    AutoModRuleUpdate,
    AutoModActionExecuted,
}

/// Target of an audit action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "id")]
pub enum AuditTarget {
    Channel(String),
    Role(u32),
    Member(String),
    Message(String),
    Community,
}

/// A field-level change for update operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditChange {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_value: Option<String>,
}
