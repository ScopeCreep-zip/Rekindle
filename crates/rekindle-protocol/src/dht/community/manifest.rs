//! Manifest record operations for a community's DFLT 16-subkey manifest.
//!
//! The manifest is a single-owner DFLT record owned by the community creator.
//! It stores community metadata, channel directory, categories, roles,
//! bans, coordinator info, policies, and invites across 16 subkeys.

use crate::dht::DHTManager;
use crate::error::ProtocolError;

use super::automod::AutoModConfig;
use super::onboarding::{OnboardingConfig, WelcomeScreen};
use super::types::{
    BanEntry, CategoryEntry, ChannelEntryV2, CommunityMetadataV2, CommunityPolicy, CoordinatorInfo,
    InviteEntry, RoleEntryV2, MANIFEST_AUDIT_LOG_KEY, MANIFEST_AUTOMOD, MANIFEST_BANS,
    MANIFEST_CATEGORIES, MANIFEST_CHANNELS, MANIFEST_COORDINATOR, MANIFEST_INVITES,
    MANIFEST_METADATA, MANIFEST_ONBOARDING, MANIFEST_POLICIES, MANIFEST_ROLES,
    MANIFEST_SUBKEY_COUNT, MANIFEST_WELCOME,
};

/// Create a new manifest DFLT record for a community.
///
/// Returns `(record_key, owner_keypair)`. The owner keypair must be persisted
/// by the caller (coordinator) for future write access.
pub async fn create_manifest(
    dht: &DHTManager,
    metadata: &CommunityMetadataV2,
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    let (key, owner_keypair) = dht.create_record(MANIFEST_SUBKEY_COUNT).await?;

    // Write initial metadata to subkey 0
    let meta_bytes = serde_json::to_vec(metadata)
        .map_err(|e| ProtocolError::Serialization(format!("manifest metadata: {e}")))?;
    dht.set_value(&key, MANIFEST_METADATA, meta_bytes).await?;

    // Initialize empty channel list
    let empty_channels: Vec<ChannelEntryV2> = Vec::new();
    write_channels(dht, &key, &empty_channels).await?;

    // Initialize empty categories
    let empty_categories: Vec<CategoryEntry> = Vec::new();
    write_categories(dht, &key, &empty_categories).await?;

    // Initialize empty roles list
    let empty_roles: Vec<RoleEntryV2> = Vec::new();
    write_roles(dht, &key, &empty_roles).await?;

    tracing::info!(key = %key, name = %metadata.name, "manifest record created");
    Ok((key, owner_keypair))
}

// ── Metadata (subkey 0) ──

/// Read community metadata from the manifest.
pub async fn read_metadata(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<CommunityMetadataV2>, ProtocolError> {
    match dht.get_value(key, MANIFEST_METADATA).await? {
        Some(data) => {
            let meta: CommunityMetadataV2 = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("manifest metadata: {e}")))?;
            Ok(Some(meta))
        }
        None => Ok(None),
    }
}

/// Write community metadata to the manifest.
pub async fn write_metadata(
    dht: &DHTManager,
    key: &str,
    metadata: &CommunityMetadataV2,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(metadata)
        .map_err(|e| ProtocolError::Serialization(format!("manifest metadata: {e}")))?;
    dht.set_value(key, MANIFEST_METADATA, bytes).await
}

// ── Channels (subkey 1) ──

/// Read channel directory from the manifest.
pub async fn read_channels(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<ChannelEntryV2>, ProtocolError> {
    match dht.get_value(key, MANIFEST_CHANNELS).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("manifest channels: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write channel directory to the manifest.
pub async fn write_channels(
    dht: &DHTManager,
    key: &str,
    channels: &[ChannelEntryV2],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(channels)
        .map_err(|e| ProtocolError::Serialization(format!("manifest channels: {e}")))?;
    dht.set_value(key, MANIFEST_CHANNELS, bytes).await
}

// ── Categories (subkey 2) ──

/// Read category directory from the manifest.
pub async fn read_categories(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<CategoryEntry>, ProtocolError> {
    match dht.get_value(key, MANIFEST_CATEGORIES).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("manifest categories: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write category directory to the manifest.
pub async fn write_categories(
    dht: &DHTManager,
    key: &str,
    categories: &[CategoryEntry],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(categories)
        .map_err(|e| ProtocolError::Serialization(format!("manifest categories: {e}")))?;
    dht.set_value(key, MANIFEST_CATEGORIES, bytes).await
}

// ── Roles (subkey 3) ──

/// Read role definitions from the manifest.
pub async fn read_roles(dht: &DHTManager, key: &str) -> Result<Vec<RoleEntryV2>, ProtocolError> {
    match dht.get_value(key, MANIFEST_ROLES).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("manifest roles: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write role definitions to the manifest.
pub async fn write_roles(
    dht: &DHTManager,
    key: &str,
    roles: &[RoleEntryV2],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(roles)
        .map_err(|e| ProtocolError::Serialization(format!("manifest roles: {e}")))?;
    dht.set_value(key, MANIFEST_ROLES, bytes).await
}

// ── Bans (subkey 4) ──

/// Read ban list from the manifest.
pub async fn read_bans(dht: &DHTManager, key: &str) -> Result<Vec<BanEntry>, ProtocolError> {
    match dht.get_value(key, MANIFEST_BANS).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("manifest bans: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write ban list to the manifest.
pub async fn write_bans(
    dht: &DHTManager,
    key: &str,
    bans: &[BanEntry],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(bans)
        .map_err(|e| ProtocolError::Serialization(format!("manifest bans: {e}")))?;
    dht.set_value(key, MANIFEST_BANS, bytes).await
}

// ── Coordinator info (subkey 5) ──

/// Read coordinator info from the manifest.
pub async fn read_coordinator(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<CoordinatorInfo>, ProtocolError> {
    match dht.get_value(key, MANIFEST_COORDINATOR).await? {
        Some(data) => {
            let info: CoordinatorInfo = serde_json::from_slice(&data).map_err(|e| {
                ProtocolError::Deserialization(format!("manifest coordinator: {e}"))
            })?;
            Ok(Some(info))
        }
        None => Ok(None),
    }
}

/// Write coordinator info to the manifest.
pub async fn write_coordinator(
    dht: &DHTManager,
    key: &str,
    info: &CoordinatorInfo,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(info)
        .map_err(|e| ProtocolError::Serialization(format!("manifest coordinator: {e}")))?;
    dht.set_value(key, MANIFEST_COORDINATOR, bytes).await
}

// ── Policies (subkey 6) ──

/// Read community policies from the manifest.
pub async fn read_policies(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<CommunityPolicy>, ProtocolError> {
    match dht.get_value(key, MANIFEST_POLICIES).await? {
        Some(data) => {
            let policy: CommunityPolicy = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("manifest policies: {e}")))?;
            Ok(Some(policy))
        }
        None => Ok(None),
    }
}

/// Write community policies to the manifest.
pub async fn write_policies(
    dht: &DHTManager,
    key: &str,
    policy: &CommunityPolicy,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(policy)
        .map_err(|e| ProtocolError::Serialization(format!("manifest policies: {e}")))?;
    dht.set_value(key, MANIFEST_POLICIES, bytes).await
}

// ── Invites (subkey 7) ──

/// Read invite list from the manifest.
pub async fn read_invites(dht: &DHTManager, key: &str) -> Result<Vec<InviteEntry>, ProtocolError> {
    match dht.get_value(key, MANIFEST_INVITES).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("manifest invites: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write invite list to the manifest.
pub async fn write_invites(
    dht: &DHTManager,
    key: &str,
    invites: &[InviteEntry],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(invites)
        .map_err(|e| ProtocolError::Serialization(format!("manifest invites: {e}")))?;
    dht.set_value(key, MANIFEST_INVITES, bytes).await
}

// ── AutoMod (subkey 9) ──

/// Read automod configuration from the manifest.
pub async fn read_automod(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<AutoModConfig>, ProtocolError> {
    match dht.get_value(key, MANIFEST_AUTOMOD).await? {
        Some(data) => {
            let config: AutoModConfig = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("manifest automod: {e}")))?;
            Ok(Some(config))
        }
        None => Ok(None),
    }
}

/// Write automod configuration to the manifest.
pub async fn write_automod(
    dht: &DHTManager,
    key: &str,
    config: &AutoModConfig,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(config)
        .map_err(|e| ProtocolError::Serialization(format!("manifest automod: {e}")))?;
    dht.set_value(key, MANIFEST_AUTOMOD, bytes).await
}

// ── Onboarding (subkey 10) ──

/// Read onboarding configuration from the manifest.
pub async fn read_onboarding(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<OnboardingConfig>, ProtocolError> {
    match dht.get_value(key, MANIFEST_ONBOARDING).await? {
        Some(data) => {
            let config: OnboardingConfig = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("manifest onboarding: {e}")))?;
            Ok(Some(config))
        }
        None => Ok(None),
    }
}

/// Write onboarding configuration to the manifest.
pub async fn write_onboarding(
    dht: &DHTManager,
    key: &str,
    config: &OnboardingConfig,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(config)
        .map_err(|e| ProtocolError::Serialization(format!("manifest onboarding: {e}")))?;
    dht.set_value(key, MANIFEST_ONBOARDING, bytes).await
}

// ── Welcome screen (subkey 11) ──

/// Read welcome screen from the manifest.
pub async fn read_welcome(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<WelcomeScreen>, ProtocolError> {
    match dht.get_value(key, MANIFEST_WELCOME).await? {
        Some(data) => {
            let screen: WelcomeScreen = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("manifest welcome: {e}")))?;
            Ok(Some(screen))
        }
        None => Ok(None),
    }
}

/// Write welcome screen to the manifest.
pub async fn write_welcome(
    dht: &DHTManager,
    key: &str,
    screen: &WelcomeScreen,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(screen)
        .map_err(|e| ProtocolError::Serialization(format!("manifest welcome: {e}")))?;
    dht.set_value(key, MANIFEST_WELCOME, bytes).await
}

// ── Audit log key (subkey 14) ──

/// Read the audit log DHT record key from the manifest.
pub async fn read_audit_log_key(
    dht: &DHTManager,
    key: &str,
) -> Result<Option<String>, ProtocolError> {
    match dht.get_value(key, MANIFEST_AUDIT_LOG_KEY).await? {
        Some(data) => {
            let audit_key: String = serde_json::from_slice(&data).map_err(|e| {
                ProtocolError::Deserialization(format!("manifest audit log key: {e}"))
            })?;
            Ok(Some(audit_key))
        }
        None => Ok(None),
    }
}

/// Write the audit log DHT record key to the manifest.
pub async fn write_audit_log_key(
    dht: &DHTManager,
    key: &str,
    audit_key: &str,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(audit_key)
        .map_err(|e| ProtocolError::Serialization(format!("manifest audit log key: {e}")))?;
    dht.set_value(key, MANIFEST_AUDIT_LOG_KEY, bytes).await
}

// ── Watch ──

/// Watch all manifest subkeys for changes.
pub async fn watch_manifest(dht: &DHTManager, key: &str) -> Result<bool, ProtocolError> {
    let subkeys: Vec<u32> = (0..MANIFEST_SUBKEY_COUNT).collect();
    dht.watch_record(key, &subkeys).await
}
