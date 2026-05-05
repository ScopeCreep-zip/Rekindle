//! Architecture §28.4 — cross-device sync types.
//!
//! Each user owns one personal DFLT record at a stable key derived from
//! their master identity. The record holds 4 well-known subkeys, each
//! storing one CRDT-merged document type. All payloads on-the-wire are
//! AES-256-GCM encrypted with a sync key derived from the master
//! identity (see `rekindle-secrets::sync_key`).

use serde::{Deserialize, Serialize};

pub const SUBKEY_MANIFEST: u32 = 0;
pub const SUBKEY_READ_STATE: u32 = 1;
pub const SUBKEY_PREFERENCES: u32 = 2;
pub const SUBKEY_DEVICE_LIST: u32 = 3;

/// Subkey 0 — top-level record listing the communities this identity
/// participates in. Each device merges by latest-Lamport-per-entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncManifest {
    pub communities: Vec<SyncCommunityRef>,
    pub lamport: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCommunityRef {
    pub community_id: String,
    pub joined_at: u64,
    pub display_name: String,
}

/// Subkey 1 — per-community, per-channel last-read Lamport. CRDT merge:
/// max Lamport wins (reading is monotonic — you can't unread a
/// message; arch §28.4 line 3074).
///
/// `onboarding_complete` is layered into the same subkey because it
/// follows the same monotonic-OR semantics: once any paired device
/// finishes the onboarding wizard for a community, every other device
/// must skip it. The map is keyed by community id; entries are only
/// ever written `true`, never `false` (clearing requires leaving and
/// rejoining the community, at which point the local SQLite mirror
/// resets too).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadState {
    pub entries: Vec<ReadStateEntry>,
    /// Map of `community_id` → `true` for communities whose onboarding
    /// wizard has been completed on at least one paired device. CRDT
    /// merge is a logical OR (see `merge_read_state` in the host
    /// crate's cross_device_sync module).
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub onboarding_complete: std::collections::HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadStateEntry {
    pub community_id: String,
    pub channel_id: String,
    pub last_read_lamport: u64,
}

/// Subkey 2 — UI / notification preferences. CRDT merge: latest-Lamport
/// per-field wins (last-writer-wins).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreferences {
    pub notification_default_level: Option<u32>,
    pub theme: Option<String>,
    pub language: Option<String>,
    pub quiet_hours_start: Option<String>,
    pub quiet_hours_end: Option<String>,
    pub lamport: u64,
}

/// Subkey 3 — registered devices. Each device adds itself when paired,
/// and an `unpaired_at` field is set when one is removed (kept around
/// so other devices learn about the removal even after the entry is
/// replaced).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub devices: Vec<DeviceListEntry>,
    pub lamport: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceListEntry {
    pub device_id: String,
    pub device_public_key: String,
    pub display_name: String,
    pub paired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpaired_at: Option<u64>,
}

/// Wire payload sent device-to-device during the pairing handshake.
/// `wrapped_master_secret` is AES-256-GCM(`derived_pairing_key`,
/// nonce, master_identity_secret_bytes); `pairing_salt` accompanies
/// the pairing code so the receiver can re-derive the wrapping key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingPayload {
    pub wrapped_master_secret: Vec<u8>,
    pub nonce: Vec<u8>,
    pub pairing_salt: Vec<u8>,
    pub display_name: String,
}

/// Wire payload an existing device sends back to confirm pairing
/// success and share the personal DFLT record key the new device
/// should start watching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingAccept {
    pub personal_record_key: String,
    pub assigned_device_id: String,
}

/// `app_call` envelope for the cross-device sync subsystem. Carries
/// its own `type` discriminant so the central app_call dispatcher in
/// `services/veilid/network.rs` can distinguish sync traffic from
/// `CommunityEnvelope` traffic without ambiguity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum SyncEnvelope {
    PairingRequest(PairingPayload),
}
