use veilid_core::{
    DHTSchema, KeyPair, RecordKey, RoutingContext, ValueSubkeyRangeSet,
    CRYPTO_KIND_VLD0,
};

use crate::capnp_codec::conversation::{
    ConversationHeader, decode_conversation_header, encode_conversation_header,
};
use crate::capnp_codec::identity::{PreKeyBundle, UserProfile};
use crate::dht::log::DHTLog;
use crate::error::ProtocolError;
use rekindle_crypto::DhtRecordKey;

/// A per-contact conversation DHT record.
///
/// Each party creates their own `ConversationRecord` for each contact.
/// Alice's record for Bob contains Alice's profile, route blob, and
/// outbound message log. Bob reads Alice's record, Alice reads Bob's.
///
/// Encryption uses a DH shared secret so both parties can read/write.
pub struct ConversationRecord {
    routing_context: RoutingContext,
    record_key: RecordKey,
    owner_keypair: Option<KeyPair>,
    encryption_key: DhtRecordKey,
    message_log_key: Option<String>,
}

impl ConversationRecord {
    /// Create a new conversation record with a child `DHTLog` for messages.
    ///
    /// Returns the record and the owner keypair.
    pub async fn create(
        rc: &RoutingContext,
        encryption_key: DhtRecordKey,
        identity_public_key: &[u8],
        profile: &UserProfile,
        route_blob: &[u8],
        prekey_bundle: &PreKeyBundle,
    ) -> Result<(Self, KeyPair), ProtocolError> {
        let schema = DHTSchema::dflt(1)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;

        let descriptor = rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create conversation record: {e}")))?;

        let key = descriptor.key().clone();
        let keypair = descriptor
            .owner_secret()
            .map(|secret| KeyPair::new_from_parts(descriptor.owner().clone(), secret.value()))
            .ok_or_else(|| ProtocolError::DhtError("no owner secret after create".into()))?;

        // Create child DHTLog for outbound messages (same owner)
        let (log, _) = DHTLog::create(rc).await?;
        let message_log_key = log.spine_key();

        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);

        let header = ConversationHeader {
            identity_public_key: identity_public_key.to_vec(),
            profile: profile.clone(),
            message_log_key: message_log_key.clone(),
            route_blob: route_blob.to_vec(),
            prekey_bundle: prekey_bundle.clone(),
            created_at: now,
            updated_at: now,
        };

        // Encode, encrypt, and write to subkey 0
        let plaintext = encode_conversation_header(&header);
        let ciphertext = encryption_key.encrypt(&plaintext)?;
        rc.set_dht_value(key.clone(), 0, ciphertext, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write conversation header: {e}")))?;

        tracing::debug!(key = %key, "ConversationRecord created");

        Ok((
            Self {
                routing_context: rc.clone(),
                record_key: key,
                owner_keypair: Some(keypair.clone()),
                encryption_key,
                message_log_key: Some(message_log_key),
            },
            keypair,
        ))
    }

    /// Open an existing conversation record with write access.
    pub async fn open_write(
        rc: &RoutingContext,
        key: &str,
        owner_keypair: KeyPair,
        encryption_key: DhtRecordKey,
    ) -> Result<Self, ProtocolError> {
        let record_key: RecordKey = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid key '{key}': {e}")))?;

        let _ = rc
            .open_dht_record(record_key.clone(), Some(owner_keypair.clone()))
            .await
            .map_err(|e| ProtocolError::DhtError(format!("open conversation record: {e}")))?;

        // Read and decrypt header to get message log key
        let message_log_key = match rc
            .get_dht_value(record_key.clone(), 0, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("read conversation header: {e}")))?
        {
            Some(v) => {
                let plaintext = encryption_key.decrypt(v.data())?;
                let header = decode_conversation_header(&plaintext)?;
                Some(header.message_log_key)
            }
            None => None,
        };

        tracing::debug!(key, "ConversationRecord opened (write)");

        Ok(Self {
            routing_context: rc.clone(),
            record_key,
            owner_keypair: Some(owner_keypair),
            encryption_key,
            message_log_key,
        })
    }

    /// Open an existing conversation record for reading only.
    pub async fn open_read(
        rc: &RoutingContext,
        key: &str,
        encryption_key: DhtRecordKey,
    ) -> Result<Self, ProtocolError> {
        let record_key: RecordKey = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid key '{key}': {e}")))?;

        let _ = rc
            .open_dht_record(record_key.clone(), None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("open conversation record: {e}")))?;

        // Read and decrypt header to get message log key
        let message_log_key = match rc
            .get_dht_value(record_key.clone(), 0, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("read conversation header: {e}")))?
        {
            Some(v) => {
                let plaintext = encryption_key.decrypt(v.data())?;
                let header = decode_conversation_header(&plaintext)?;
                Some(header.message_log_key)
            }
            None => None,
        };

        tracing::debug!(key, "ConversationRecord opened (read)");

        Ok(Self {
            routing_context: rc.clone(),
            record_key,
            owner_keypair: None,
            encryption_key,
            message_log_key,
        })
    }

    /// Read and decrypt the conversation header.
    pub async fn read_header(&self) -> Result<ConversationHeader, ProtocolError> {
        let value = self
            .routing_context
            .get_dht_value(self.record_key.clone(), 0, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("read conversation header: {e}")))?
            .ok_or_else(|| ProtocolError::DhtError("conversation header not set".into()))?;

        let plaintext = self.encryption_key.decrypt(value.data())?;
        decode_conversation_header(&plaintext)
    }

    /// Encrypt and write a new conversation header.
    pub async fn write_header(&self, header: &ConversationHeader) -> Result<(), ProtocolError> {
        let plaintext = encode_conversation_header(header);
        let ciphertext = self.encryption_key.encrypt(&plaintext)?;
        self.routing_context
            .set_dht_value(self.record_key.clone(), 0, ciphertext, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write conversation header: {e}")))?;
        Ok(())
    }

    /// Update just the route blob in the conversation header.
    pub async fn update_route_blob(&self, route_blob: &[u8]) -> Result<(), ProtocolError> {
        let mut header = self.read_header().await?;
        header.route_blob = route_blob.to_vec();
        header.updated_at = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        self.write_header(&header).await
    }

    /// Update just the profile snapshot in the conversation header.
    pub async fn update_profile(&self, profile: &UserProfile) -> Result<(), ProtocolError> {
        let mut header = self.read_header().await?;
        header.profile = profile.clone();
        header.updated_at = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        self.write_header(&header).await
    }

    /// Watch this conversation record for changes (subkey 0).
    pub async fn watch(&self) -> Result<bool, ProtocolError> {
        let subkeys: ValueSubkeyRangeSet = [0u32].iter().copied().collect();
        let active = self
            .routing_context
            .watch_dht_values(self.record_key.clone(), Some(subkeys), None, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("watch conversation: {e}")))?;

        tracing::debug!(key = %self.record_key, "ConversationRecord watch requested");
        Ok(active)
    }

    /// Close the underlying DHT record.
    pub async fn close(&self) -> Result<(), ProtocolError> {
        self.routing_context
            .close_dht_record(self.record_key.clone())
            .await
            .map_err(|e| ProtocolError::DhtError(format!("close conversation record: {e}")))?;
        Ok(())
    }

    /// Get the record key as a string.
    pub fn record_key(&self) -> String {
        self.record_key.to_string()
    }

    /// Get the message log key (if loaded).
    pub fn message_log_key(&self) -> Option<&str> {
        self.message_log_key.as_deref()
    }

    /// Get the owner keypair (if opened with write access).
    pub fn owner_keypair(&self) -> Option<&KeyPair> {
        self.owner_keypair.as_ref()
    }

    /// Return all DHT record keys owned by this conversation (parent + message log).
    ///
    /// Used for bulk close on logout / app exit.
    pub fn all_record_keys(&self) -> Vec<String> {
        let mut keys = vec![self.record_key.to_string()];
        if let Some(ref k) = self.message_log_key {
            keys.push(k.clone());
        }
        keys
    }
}
