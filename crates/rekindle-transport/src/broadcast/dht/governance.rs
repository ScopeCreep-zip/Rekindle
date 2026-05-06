//! Governance manifest record operations (DFLT, 16 subkeys).
//!
//! Typed business logic over raw DHT I/O. All raw reads/writes flow
//! through `dht::record` (which is the same code `broadcast::dht_writes`
//! calls). This module owns serialization/deserialization of governance
//! types — it does NOT call Veilid directly.

use veilid_core::{KeyPair, RoutingContext};

use super::record;
use crate::error::{TransportError, Result};
use crate::payload::dht_types::{
    BanEntry, CategoryEntry, ChannelEntry, CommunityMetadata, InviteEntry,
    MANIFEST_AUDIT_LOG_KEY, MANIFEST_BANS, MANIFEST_CATEGORIES, MANIFEST_CHANNELS,
    MANIFEST_INVITES, MANIFEST_METADATA, MANIFEST_ROLES, MANIFEST_SUBKEY_COUNT, RoleEntry,
};

// ── Generic typed subkey read/write ──────────────────────────────────

async fn read_json_subkey<T: serde::de::DeserializeOwned>(
    rc: &RoutingContext, key: &str, subkey: u32, label: &str,
) -> Result<Option<T>> {
    match record::get(rc, key, subkey, false).await? {
        Some(data) if !data.is_empty() => {
            let value: T = serde_json::from_slice(&data).map_err(|e| {
                TransportError::DeserializationFailed { type_id: 0, reason: format!("{label}: {e}") }
            })?;
            Ok(Some(value))
        }
        _ => Ok(None),
    }
}

async fn read_json_subkey_vec<T: serde::de::DeserializeOwned>(
    rc: &RoutingContext, key: &str, subkey: u32, label: &str,
) -> Result<Vec<T>> {
    read_json_subkey::<Vec<T>>(rc, key, subkey, label)
        .await
        .map(Option::unwrap_or_default)
}

async fn write_json_subkey<T: serde::Serialize>(
    rc: &RoutingContext, key: &str, subkey: u32, value: &T, label: &str,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| TransportError::SerializationFailed { reason: format!("{label}: {e}") })?;
    record::set(rc, key, subkey, bytes, None).await
}

/// Operations on a community governance manifest.
pub struct GovernanceOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> GovernanceOps<'a> {
    pub fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Create a new manifest record and initialize subkeys.
    pub async fn create(&self, metadata: &CommunityMetadata) -> Result<(String, Option<KeyPair>)> {
        #[allow(clippy::cast_possible_truncation)]
        let (key, keypair) = record::create_dflt(self.rc, MANIFEST_SUBKEY_COUNT as u16, None).await?;

        write_json_subkey(self.rc, &key, MANIFEST_METADATA, metadata, "metadata").await?;
        write_json_subkey(self.rc, &key, MANIFEST_CHANNELS, &Vec::<ChannelEntry>::new(), "channels").await?;
        write_json_subkey(self.rc, &key, MANIFEST_CATEGORIES, &Vec::<CategoryEntry>::new(), "categories").await?;
        write_json_subkey(self.rc, &key, MANIFEST_ROLES, &Vec::<RoleEntry>::new(), "roles").await?;
        write_json_subkey(self.rc, &key, MANIFEST_BANS, &Vec::<BanEntry>::new(), "bans").await?;
        write_json_subkey(self.rc, &key, MANIFEST_INVITES, &Vec::<InviteEntry>::new(), "invites").await?;

        tracing::info!(key = %key, name = %metadata.name, "governance manifest created");
        Ok((key, keypair))
    }

    pub async fn read_metadata(&self, key: &str) -> Result<Option<CommunityMetadata>> {
        read_json_subkey(self.rc, key, MANIFEST_METADATA, "metadata").await
    }
    pub async fn write_metadata(&self, key: &str, metadata: &CommunityMetadata) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_METADATA, metadata, "metadata").await
    }

    pub async fn read_channels(&self, key: &str) -> Result<Vec<ChannelEntry>> {
        read_json_subkey_vec(self.rc, key, MANIFEST_CHANNELS, "channels").await
    }
    pub async fn write_channels(&self, key: &str, channels: &[ChannelEntry]) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_CHANNELS, &channels, "channels").await
    }

    pub async fn read_categories(&self, key: &str) -> Result<Vec<CategoryEntry>> {
        read_json_subkey_vec(self.rc, key, MANIFEST_CATEGORIES, "categories").await
    }
    pub async fn write_categories(&self, key: &str, categories: &[CategoryEntry]) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_CATEGORIES, &categories, "categories").await
    }

    pub async fn read_roles(&self, key: &str) -> Result<Vec<RoleEntry>> {
        read_json_subkey_vec(self.rc, key, MANIFEST_ROLES, "roles").await
    }
    pub async fn write_roles(&self, key: &str, roles: &[RoleEntry]) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_ROLES, &roles, "roles").await
    }

    pub async fn read_bans(&self, key: &str) -> Result<Vec<BanEntry>> {
        read_json_subkey_vec(self.rc, key, MANIFEST_BANS, "bans").await
    }
    pub async fn write_bans(&self, key: &str, bans: &[BanEntry]) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_BANS, &bans, "bans").await
    }

    pub async fn read_invites(&self, key: &str) -> Result<Vec<InviteEntry>> {
        read_json_subkey_vec(self.rc, key, MANIFEST_INVITES, "invites").await
    }
    pub async fn write_invites(&self, key: &str, invites: &[InviteEntry]) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_INVITES, &invites, "invites").await
    }

    pub async fn read_audit_log_key(&self, key: &str) -> Result<Option<String>> {
        read_json_subkey(self.rc, key, MANIFEST_AUDIT_LOG_KEY, "audit_log_key").await
    }
    pub async fn write_audit_log_key(&self, key: &str, audit_key: &str) -> Result<()> {
        write_json_subkey(self.rc, key, MANIFEST_AUDIT_LOG_KEY, &audit_key, "audit_log_key").await
    }

    pub async fn watch_all(&self, key: &str) -> Result<bool> {
        let subkeys: Vec<u32> = (0..MANIFEST_SUBKEY_COUNT).collect();
        record::watch(self.rc, key, &subkeys).await
    }

    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }
}
