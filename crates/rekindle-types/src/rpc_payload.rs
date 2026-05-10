//! RPC (request/response) payload types for app_call operations.
//!
//! Every RPC payload is wrapped in a signed envelope for authentication.
//! RPC is used ONLY for real-time operations where both parties must be online:
//!
//! - **Governance operations** — admin commands from non-operator nodes
//! - **Leave notification** — best-effort cleanup + rekey trigger
//! - **Sync requests** — history sync to archiver nodes
//!
//! Community join, friend requests, DMs, and all other critical lifecycle
//! state changes use DHT (fully async, offline-safe).

use serde::{Deserialize, Serialize};

/// A single MEK wrapped for a specific member via ECDH.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekTransferPayload {
    pub channel_id: String,
    pub generation: u64,
    pub rotator_pseudonym_hex: String,
    pub wrapped_mek: Vec<u8>,
}

/// Compact channel entry for join responses (no log_key or per-member data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelEntrySummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mek_generation: u64,
}

/// Notification sent when a member leaves a community.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityLeaveNotification {
    pub governance_key: String,
    pub leaving_pseudonym_hex: String,
}

/// Governance operation request scoped to a specific community.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceRequest {
    pub governance_key: String,
    pub operation: GovernanceOp,
}

/// Governance operations submitted by admins/moderators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GovernanceOp {
    Kick { target_pseudonym: String },
    Ban { target_pseudonym: String, reason: Option<String> },
    Unban { target_pseudonym: String },
    Timeout { target_pseudonym: String, duration_seconds: u64, reason: Option<String> },
    ApproveJoin { target_pseudonym: String },
    RejectJoin { target_pseudonym: String, reason: String },
    CreateChannel { name: String, kind: String, topic: Option<String> },
    DeleteChannel { channel_id: String },
    UpdateChannel { channel_id: String, name: Option<String>, topic: Option<String> },
    CreateRole { name: String, permissions: u64, color: u32, position: i32 },
    UpdateRole { role_id: u32, name: Option<String>, permissions: Option<u64>, color: Option<u32> },
    DeleteRole { role_id: u32 },
    AssignRole { member_pseudonym: String, role_id: u32 },
    UnassignRole { member_pseudonym: String, role_id: u32 },
    RotateMek { channel_id: String },
    TransferOwnership { new_owner_pseudonym: String },
    RegisterChannelRecord { member_pseudonym: String, channel_id: String, record_key: String },
}

/// Response to a governance operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GovernanceOpResponse {
    Ok(serde_json::Value),
    PermissionDenied { required: String },
    NotFound { entity: String },
    Failed { reason: String },
}

/// Bootstrap request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapRequest {
    pub joiner_pseudonym: String,
    pub governance_key: String,
}

/// Bootstrap response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub governance_entries: Vec<Vec<u8>>,
    pub member_list: Vec<Vec<u8>>,
    pub channel_meks: Vec<Vec<u8>>,
    pub recent_messages: Vec<Vec<u8>>,
    pub wrapped_owner_keypair: Vec<u8>,
}

/// Sync request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub channel_id: String,
    pub since_timestamp: u64,
}

/// Sync response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub channel_id: String,
    pub messages: Vec<Vec<u8>>,
}

/// Inbound RPC call variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InboundCall {
    CommunityLeave(CommunityLeaveNotification),
    CommunityGovOp(GovernanceRequest),
    Sync(SyncRequest),
    Dm(Vec<u8>),
}

/// Response from a handler to an inbound RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CallResponse {
    Ok(Vec<u8>),
    Ack,
    Rejected { reason: String },
}
