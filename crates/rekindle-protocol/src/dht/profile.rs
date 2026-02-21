use crate::dht::DHTManager;
use crate::error::ProtocolError;

// Subkey constants for user profile DHT record.
pub const SUBKEY_DISPLAY_NAME: u32 = 0;
pub const SUBKEY_STATUS_MESSAGE: u32 = 1;
pub const SUBKEY_STATUS: u32 = 2;
pub const SUBKEY_AVATAR: u32 = 3;
pub const SUBKEY_GAME_INFO: u32 = 4;
pub const SUBKEY_PREKEY_BUNDLE: u32 = 5;
pub const SUBKEY_ROUTE_BLOB: u32 = 6;
pub const SUBKEY_METADATA: u32 = 7;

pub const PROFILE_SUBKEY_COUNT: u32 = 8;

/// Create a new profile DHT record and initialize subkeys.
///
/// Returns `(record_key, owner_keypair)`. The keypair must be persisted to retain
/// write access across sessions.
pub async fn create_profile(
    dht: &DHTManager,
    display_name: &str,
    status_message: &str,
    prekey_bundle: &[u8],
    route_blob: &[u8],
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    let (key, owner_keypair) = dht.create_record(PROFILE_SUBKEY_COUNT).await?;

    // Set initial values
    dht.set_value(&key, SUBKEY_DISPLAY_NAME, display_name.as_bytes().to_vec())
        .await?;
    dht.set_value(
        &key,
        SUBKEY_STATUS_MESSAGE,
        status_message.as_bytes().to_vec(),
    )
    .await?;
    let ts: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX);
    let mut status_payload = Vec::with_capacity(9);
    status_payload.push(0u8);
    status_payload.extend_from_slice(&ts.to_be_bytes());
    dht.set_value(&key, SUBKEY_STATUS, status_payload).await?; // 0 = online
    dht.set_value(&key, SUBKEY_PREKEY_BUNDLE, prekey_bundle.to_vec())
        .await?;
    dht.set_value(&key, SUBKEY_ROUTE_BLOB, route_blob.to_vec())
        .await?;

    tracing::info!(key = %key, name = %display_name, "profile record created");
    Ok((key, owner_keypair))
}

/// Update a specific profile subkey.
pub async fn update_subkey(
    dht: &DHTManager,
    profile_key: &str,
    subkey: u32,
    value: Vec<u8>,
) -> Result<(), ProtocolError> {
    dht.set_value(profile_key, subkey, value).await
}

/// Read a specific profile subkey.
pub async fn read_subkey(
    dht: &DHTManager,
    profile_key: &str,
    subkey: u32,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    dht.get_value(profile_key, subkey).await
}

/// Read the display name from a profile record.
pub async fn read_display_name(
    dht: &DHTManager,
    profile_key: &str,
) -> Result<Option<String>, ProtocolError> {
    match dht.get_value(profile_key, SUBKEY_DISPLAY_NAME).await? {
        Some(bytes) => {
            Ok(Some(String::from_utf8(bytes).map_err(|e| {
                ProtocolError::Deserialization(e.to_string())
            })?))
        }
        None => Ok(None),
    }
}

/// Read the status from a profile record.
pub async fn read_status(dht: &DHTManager, profile_key: &str) -> Result<Option<u8>, ProtocolError> {
    match dht.get_value(profile_key, SUBKEY_STATUS).await? {
        Some(bytes) => Ok(bytes.first().copied()),
        None => Ok(None),
    }
}

/// Read the route blob from a profile record.
pub async fn read_route_blob(
    dht: &DHTManager,
    profile_key: &str,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    dht.get_value(profile_key, SUBKEY_ROUTE_BLOB).await
}

/// Read the prekey bundle from a profile record.
pub async fn read_prekey_bundle(
    dht: &DHTManager,
    profile_key: &str,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    dht.get_value(profile_key, SUBKEY_PREKEY_BUNDLE).await
}

/// Open an existing profile DHT record and update all subkeys, or create a new one.
///
/// On reopen: opens with write access via the owner keypair, then updates subkeys
/// 0 (display name), 1 (status message), 2 (status=online), 5 (prekey bundle),
/// and 6 (route blob). If the open or any subkey write fails, falls back to
/// creating a fresh profile record.
///
/// Returns `(key, keypair, is_new)`. When `is_new` is true the keypair must be
/// persisted to `SQLite`.
pub async fn open_or_create_profile(
    dht: &DHTManager,
    existing_key: Option<&str>,
    owner_keypair: Option<veilid_core::KeyPair>,
    display_name: &str,
    status_message: &str,
    prekey_bundle: &[u8],
    route_blob: &[u8],
) -> Result<(String, Option<veilid_core::KeyPair>, bool), ProtocolError> {
    // Try to reopen and update existing record
    if let (Some(key), Some(ref keypair)) = (existing_key, &owner_keypair) {
        match try_reopen_and_update(
            dht,
            key,
            keypair.clone(),
            display_name,
            status_message,
            prekey_bundle,
            route_blob,
        )
        .await
        {
            Ok(()) => {
                tracing::info!(key, "reusing existing DHT profile record");
                return Ok((key.to_string(), owner_keypair, false));
            }
            Err(e) => {
                tracing::warn!(
                    key, error = %e,
                    "failed to reuse existing DHT profile — creating new one"
                );
            }
        }
    } else if existing_key.is_some() {
        tracing::warn!("no owner keypair for existing profile — creating new one");
    }

    let (key, keypair) =
        create_profile(dht, display_name, status_message, prekey_bundle, route_blob).await?;
    Ok((key, keypair, true))
}

/// Open an existing profile record writable and update all content subkeys.
///
/// Returns `Err` if the open fails OR any subkey write fails — the caller
/// should fall back to creating a new record.
async fn try_reopen_and_update(
    dht: &DHTManager,
    key: &str,
    owner_keypair: veilid_core::KeyPair,
    display_name: &str,
    status_message: &str,
    prekey_bundle: &[u8],
    route_blob: &[u8],
) -> Result<(), ProtocolError> {
    dht.open_record_writable(key, owner_keypair).await?;

    update_subkey(dht, key, SUBKEY_DISPLAY_NAME, display_name.as_bytes().to_vec()).await?;
    update_subkey(
        dht,
        key,
        SUBKEY_STATUS_MESSAGE,
        status_message.as_bytes().to_vec(),
    )
    .await?;
    // Status = online (0) + timestamp
    let ts: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX);
    let mut status_payload = Vec::with_capacity(9);
    status_payload.push(0u8);
    status_payload.extend_from_slice(&ts.to_be_bytes());
    update_subkey(dht, key, SUBKEY_STATUS, status_payload).await?;
    update_subkey(dht, key, SUBKEY_PREKEY_BUNDLE, prekey_bundle.to_vec()).await?;
    update_subkey(dht, key, SUBKEY_ROUTE_BLOB, route_blob.to_vec()).await?;

    Ok(())
}
