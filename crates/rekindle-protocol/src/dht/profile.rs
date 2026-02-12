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
    dht.set_value(&key, SUBKEY_DISPLAY_NAME, display_name.as_bytes().to_vec()).await?;
    dht.set_value(&key, SUBKEY_STATUS_MESSAGE, status_message.as_bytes().to_vec()).await?;
    dht.set_value(&key, SUBKEY_STATUS, vec![0]).await?; // 0 = online
    dht.set_value(&key, SUBKEY_PREKEY_BUNDLE, prekey_bundle.to_vec()).await?;
    dht.set_value(&key, SUBKEY_ROUTE_BLOB, route_blob.to_vec()).await?;

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
        Some(bytes) => Ok(Some(
            String::from_utf8(bytes)
                .map_err(|e| ProtocolError::Deserialization(e.to_string()))?,
        )),
        None => Ok(None),
    }
}

/// Read the status from a profile record.
pub async fn read_status(
    dht: &DHTManager,
    profile_key: &str,
) -> Result<Option<u8>, ProtocolError> {
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
