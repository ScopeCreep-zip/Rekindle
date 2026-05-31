//! Profile DHT record operations (DFLT, 8 subkeys).
//!
//! Each user's public profile is stored in a DFLT record with 8 subkeys
//! for display name, status, game info, route blob, prekey bundle, etc.

use veilid_core::{KeyPair, RoutingContext};

use super::record;
use crate::error::Result;
use crate::payload::dht_types::{
    PROFILE_SUBKEY_COUNT, PROFILE_SUBKEY_DISPLAY_NAME, PROFILE_SUBKEY_GAME_INFO,
    PROFILE_SUBKEY_PREKEY_BUNDLE, PROFILE_SUBKEY_ROUTE_BLOB, PROFILE_SUBKEY_STATUS,
    PROFILE_SUBKEY_STATUS_MESSAGE, STATUS_ONLINE,
};

/// Operations on a user's profile DHT record.
pub struct ProfileOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> ProfileOps<'a> {
    pub(crate) fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Create a new profile record and initialize subkeys.
    ///
    /// Returns `(key, keypair)`. The keypair MUST be persisted.
    #[allow(clippy::cast_possible_truncation)] // PROFILE_SUBKEY_COUNT is 10, safe
    pub async fn create(
        &self,
        display_name: &str,
        status_message: &str,
        prekey_bundle: &[u8],
        route_blob: &[u8],
    ) -> Result<(String, Option<KeyPair>)> {
        let (key, keypair) =
            record::create_dflt(self.rc, PROFILE_SUBKEY_COUNT as u16, None).await?;

        record::set(
            self.rc,
            &key,
            PROFILE_SUBKEY_DISPLAY_NAME,
            display_name.as_bytes().to_vec(),
            None,
        )
        .await?;
        record::set(
            self.rc,
            &key,
            PROFILE_SUBKEY_STATUS_MESSAGE,
            status_message.as_bytes().to_vec(),
            None,
        )
        .await?;

        let mut status_payload = Vec::with_capacity(9);
        status_payload.push(STATUS_ONLINE);
        status_payload.extend_from_slice(&rekindle_utils::timestamp_ms_i64().to_be_bytes());
        record::set(self.rc, &key, PROFILE_SUBKEY_STATUS, status_payload, None).await?;

        record::set(
            self.rc,
            &key,
            PROFILE_SUBKEY_PREKEY_BUNDLE,
            prekey_bundle.to_vec(),
            None,
        )
        .await?;
        tracing::info!(
            subkey = PROFILE_SUBKEY_PREKEY_BUNDLE,
            bytes = prekey_bundle.len(),
            "pqxdh_bundle_published kind=LastResort+OneTimeBatch (profile create)",
        );
        record::set(
            self.rc,
            &key,
            PROFILE_SUBKEY_ROUTE_BLOB,
            route_blob.to_vec(),
            None,
        )
        .await?;

        tracing::info!(key = %key, name = %display_name, "profile record created");
        Ok((key, keypair))
    }

    /// Open an existing profile and update all content subkeys,
    /// or create a new one if the open fails.
    ///
    /// Returns `(key, keypair, is_new)`.
    pub async fn open_or_create(
        &self,
        existing_key: Option<&str>,
        existing_keypair: Option<KeyPair>,
        display_name: &str,
        status_message: &str,
        prekey_bundle: &[u8],
        route_blob: &[u8],
    ) -> Result<(String, Option<KeyPair>, bool)> {
        if let (Some(key), Some(kp)) = (existing_key, &existing_keypair) {
            match self
                .try_reopen_and_update(
                    key,
                    kp.clone(),
                    display_name,
                    status_message,
                    prekey_bundle,
                    route_blob,
                )
                .await
            {
                Ok(()) => {
                    tracing::info!(key, "reusing existing profile record");
                    return Ok((key.to_string(), existing_keypair, false));
                }
                Err(e) => {
                    tracing::warn!(key, error = %e, "failed to reuse profile, creating new");
                }
            }
        }

        let (key, keypair) = self
            .create(display_name, status_message, prekey_bundle, route_blob)
            .await?;
        Ok((key, keypair, true))
    }

    /// Update a specific subkey.
    pub async fn set_subkey(&self, key: &str, subkey: u32, data: Vec<u8>) -> Result<()> {
        record::set(self.rc, key, subkey, data, None).await
    }

    /// Read a specific subkey.
    pub async fn get_subkey(&self, key: &str, subkey: u32) -> Result<Option<Vec<u8>>> {
        record::get(self.rc, key, subkey, false).await
    }

    /// Read a subkey with network refresh.
    pub async fn get_subkey_fresh(&self, key: &str, subkey: u32) -> Result<Option<Vec<u8>>> {
        record::get(self.rc, key, subkey, true).await
    }

    /// Watch presence subkeys (status, game_info, route_blob).
    pub async fn watch_presence(&self, key: &str) -> Result<bool> {
        record::watch(
            self.rc,
            key,
            &[
                PROFILE_SUBKEY_STATUS,
                PROFILE_SUBKEY_GAME_INFO,
                PROFILE_SUBKEY_ROUTE_BLOB,
            ],
        )
        .await
    }

    /// Read the prekey bundle and return the count of available prekeys.
    ///
    /// Prekey bundle format: 4-byte LE count prefix followed by the prekeys.
    /// Returns 0 if the subkey is empty or the format is invalid.
    pub async fn prekey_count(&self, key: &str) -> Result<u32> {
        match self.get_subkey(key, PROFILE_SUBKEY_PREKEY_BUNDLE).await? {
            Some(data) if data.len() >= 4 => {
                Ok(u32::from_le_bytes(data[..4].try_into().unwrap_or([0; 4])))
            }
            _ => Ok(0),
        }
    }

    /// Close the profile record.
    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }

    async fn try_reopen_and_update(
        &self,
        key: &str,
        keypair: KeyPair,
        display_name: &str,
        status_message: &str,
        prekey_bundle: &[u8],
        route_blob: &[u8],
    ) -> Result<()> {
        record::open_writable(self.rc, key, keypair).await?;

        record::set(
            self.rc,
            key,
            PROFILE_SUBKEY_DISPLAY_NAME,
            display_name.as_bytes().to_vec(),
            None,
        )
        .await?;
        record::set(
            self.rc,
            key,
            PROFILE_SUBKEY_STATUS_MESSAGE,
            status_message.as_bytes().to_vec(),
            None,
        )
        .await?;

        let mut status_payload = Vec::with_capacity(9);
        status_payload.push(STATUS_ONLINE);
        status_payload.extend_from_slice(&rekindle_utils::timestamp_ms_i64().to_be_bytes());
        record::set(self.rc, key, PROFILE_SUBKEY_STATUS, status_payload, None).await?;

        record::set(
            self.rc,
            key,
            PROFILE_SUBKEY_PREKEY_BUNDLE,
            prekey_bundle.to_vec(),
            None,
        )
        .await?;
        tracing::info!(
            subkey = PROFILE_SUBKEY_PREKEY_BUNDLE,
            bytes = prekey_bundle.len(),
            "pqxdh_bundle_published kind=LastResort+OneTimeBatch (profile reopen)",
        );
        record::set(
            self.rc,
            key,
            PROFILE_SUBKEY_ROUTE_BLOB,
            route_blob.to_vec(),
            None,
        )
        .await?;

        Ok(())
    }
}
