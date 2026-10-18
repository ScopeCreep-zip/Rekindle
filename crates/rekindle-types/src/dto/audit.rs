//! The tamper-evident audit log as frontends read it (plan E1.1):
//! `AuditVerify` and `AuditExport` answer these, mapped from
//! `rekindle_audit::{Verified, AuditEntry}` with the MACs as hex.

use serde::{Deserialize, Serialize};

/// What verifying the chain end to end found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuditVerified {
    Intact {
        length: u64,
    },
    Broken {
        length: u64,
        /// The first entry that fails, when one does.
        at: Option<u64>,
        detail: String,
    },
}

/// One entry of the chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntryView {
    pub cursor: u64,
    /// The preceding entry's MAC, hex.
    pub prev_mac: String,
    /// This entry's MAC, hex.
    pub mac: String,
    pub at_ms: i64,
    pub actor_pub: String,
    /// The action, by its `AuditKind` name.
    pub kind: String,
    pub payload: serde_json::Value,
}
