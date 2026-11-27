//! Audit log types for the audit log DHT record.
//!
//! The audit record key is stored as a pointer in manifest subkey 14.
//! The audit record itself is a DFLT 256-subkey record used as a ring buffer.

use serde::{Deserialize, Serialize};

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogEntry {
    /// Sequential entry ID within this community.
    pub entry_id: u64,
    /// The pseudonym key of the actor who performed the action.
    pub actor_pseudonym: String,
    /// What action was performed.
    pub action: AuditAction,
    /// What was the target of the action.
    pub target: AuditTarget,
    /// Detailed changes (for update operations).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<AuditChange>,
    /// Optional reason provided by the actor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Unix timestamp in seconds.
    pub timestamp: u64,
    /// Ed25519 signature of the entry by the acting admin.
    pub signature: Vec<u8>,
}

/// The type of moderation/administrative action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuditAction {
    // ── Channel operations ──
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,

    // ── Role operations ──
    RoleCreate,
    RoleUpdate,
    RoleDelete,

    // ── Member operations ──
    MemberRoleUpdate,
    MemberKick,
    MemberBan,
    MemberUnban,
    MemberTimeout,
    MemberTimeoutRemove,

    // ── Message operations ──
    MessageDelete,
    MessagePin,

    // ── Community operations ──
    CommunityUpdate,

    // ── AutoMod ──
    AutoModRuleCreate,
    AutoModRuleUpdate,
    AutoModActionExecuted,
}

/// The target of an audit action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "id")]
pub enum AuditTarget {
    /// A channel (by channel_id).
    Channel(String),
    /// A role (by role_id).
    Role(u32),
    /// A member (by pseudonym_key).
    Member(String),
    /// A message (by message_id).
    Message(String),
    /// The community itself.
    Community,
}

/// A field-level change for update operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditChange {
    /// The field that was changed.
    pub field: String,
    /// The previous value (serialized as string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_value: Option<String>,
    /// The new value (serialized as string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_value: Option<String>,
}

#[cfg(test)]
#[path = "audit_log_tests.rs"]
mod tests;
