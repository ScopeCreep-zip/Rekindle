use veilid_core::{DHTSchema, KeyPair, RecordKey, RoutingContext, CRYPTO_KIND_VLD0};

use super::parse_record_key;
use crate::capnp_codec::account::{decode_account_header, encode_account_header, AccountHeader};
use crate::error::ProtocolError;
use rekindle_crypto::DhtRecordKey;

/// A user's private account DHT record.
///
/// Holds an encrypted `AccountHeader` in subkey 0.
///
/// Only the owner can read this record — it's encrypted with a key derived
/// from the identity's Ed25519 secret.
///
/// It used to own three child `DHTShortArray`s — contact list, chat
/// list, invitation list — allocated here at creation and reopened at
/// every login. Nothing ever wrote an entry to any of them: friends
/// live in the friend-list record, conversations in `ConversationRecord`,
/// and friend requests in the friend inbox. Three empty DHT records per
/// account, with their owner keypairs persisted in this header to keep
/// them writable.
pub struct AccountRecord {
    routing_context: RoutingContext,
    record_key: RecordKey,
    owner_keypair: KeyPair,
    encryption_key: DhtRecordKey,
}

impl AccountRecord {
    /// Create a new account record.
    ///
    /// Returns the record and the owner keypair (caller must persist both).
    pub async fn create(
        rc: &RoutingContext,
        encryption_key: DhtRecordKey,
        display_name: &str,
        status_message: &str,
    ) -> Result<(Self, KeyPair), ProtocolError> {
        let schema = DHTSchema::dflt(1)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;

        let descriptor = rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create account record: {e}")))?;

        let key = descriptor.key().clone();
        let keypair = descriptor
            .owner_secret()
            .map(|secret| KeyPair::new_from_parts(descriptor.owner().clone(), secret.value()))
            .ok_or_else(|| ProtocolError::DhtError("no owner secret after create".into()))?;

        let now = rekindle_utils::timestamp_ms();

        let header = AccountHeader {
            display_name: display_name.to_string(),
            status_message: status_message.to_string(),
            avatar_hash: Vec::new(),
            created_at: now,
            updated_at: now,
        };

        // Encode, encrypt, and write to subkey 0
        let plaintext = encode_account_header(&header);
        let ciphertext = encryption_key.encrypt(&plaintext)?;
        rc.set_dht_value(key.clone(), 0, ciphertext, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write account header: {e}")))?;

        tracing::debug!(key = %key, "AccountRecord created");

        Ok((
            Self {
                routing_context: rc.clone(),
                record_key: key,
                owner_keypair: keypair.clone(),
                encryption_key,
            },
            keypair,
        ))
    }

    /// Open an existing account record with write access.
    pub async fn open(
        rc: &RoutingContext,
        key: &str,
        owner_keypair: KeyPair,
        encryption_key: DhtRecordKey,
    ) -> Result<Self, ProtocolError> {
        let record_key = parse_record_key(key)?;

        let _ = rc
            .open_dht_record(record_key.clone(), Some(owner_keypair.clone()))
            .await
            .map_err(|e| super::classify_dht_open_error("open account record", &e))?;

        tracing::debug!(key, "AccountRecord opened");

        Ok(Self {
            routing_context: rc.clone(),
            record_key,
            owner_keypair,
            encryption_key,
        })
    }

    /// Read and decrypt the account header.
    pub async fn read_header(&self) -> Result<AccountHeader, ProtocolError> {
        let value = self
            .routing_context
            .get_dht_value(self.record_key.clone(), 0, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("read account header: {e}")))?
            .ok_or_else(|| ProtocolError::DhtError("account header not set".into()))?;

        let plaintext = self.encryption_key.decrypt(value.data())?;
        decode_account_header(&plaintext)
    }

    /// Encrypt and write a new account header.
    pub async fn write_header(&self, header: &AccountHeader) -> Result<(), ProtocolError> {
        let plaintext = encode_account_header(header);
        let ciphertext = self.encryption_key.encrypt(&plaintext)?;
        self.routing_context
            .set_dht_value(self.record_key.clone(), 0, ciphertext, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write account header: {e}")))?;
        Ok(())
    }

    /// Close the underlying DHT record.
    pub async fn close(&self) -> Result<(), ProtocolError> {
        self.routing_context
            .close_dht_record(self.record_key.clone())
            .await
            .map_err(|e| ProtocolError::DhtError(format!("close account record: {e}")))?;
        Ok(())
    }

    /// Get the record key as a string.
    pub fn record_key(&self) -> String {
        self.record_key.to_string()
    }

    /// Get the parent record's owner keypair.
    pub fn owner_keypair(&self) -> &KeyPair {
        &self.owner_keypair
    }

    /// Return all DHT record keys owned by this account.
    ///
    /// Used for bulk close on logout / app exit. Kept as a `Vec` rather
    /// than collapsed to `record_key()`: the caller is the record-close
    /// sweep, and this is the account's answer to "everything of mine
    /// that is open" — a shape that survives the account owning more
    /// than one record again.
    pub fn all_record_keys(&self) -> Vec<String> {
        vec![self.record_key.to_string()]
    }
}
