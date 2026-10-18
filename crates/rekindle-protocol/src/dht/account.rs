use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, SetOutcome};

use crate::error::ProtocolError;
use rekindle_codec::capnp_codec::account::{
    decode_account_header, encode_account_header, AccountHeader,
};
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
    lease: LeaseId,
    record_key: String,
    encryption_key: DhtRecordKey,
}

impl AccountRecord {
    /// Create the account record (a fresh owner key), write its first
    /// header, and hold it for the session. Returns the record, its owner
    /// keypair, and how the header write went.
    ///
    /// # Errors
    /// The record could not be created, or the header could not be written.
    pub async fn create(
        pool: &dyn RecordPort,
        encryption_key: DhtRecordKey,
        display_name: &str,
        status_message: &str,
    ) -> Result<(Self, String, SetOutcome), ProtocolError> {
        let (lease, record_key, keypair) = pool
            .create(&rekindle_records::schema::single(), None)
            .await?;
        let record = Self {
            lease,
            record_key,
            encryption_key,
        };
        let now = rekindle_utils::timestamp_ms();
        let outcome = record
            .write_header(
                pool,
                &AccountHeader {
                    display_name: display_name.to_string(),
                    status_message: status_message.to_string(),
                    avatar_hash: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
            )
            .await?;
        tracing::debug!(key = %record.record_key, ?outcome, "AccountRecord created");
        Ok((record, keypair, outcome))
    }

    /// Hold an existing account record writable for the session.
    ///
    /// # Errors
    /// The record could not be opened within the pool's retry budget.
    pub async fn open(
        pool: &dyn RecordPort,
        key: &str,
        owner_keypair: &str,
        encryption_key: DhtRecordKey,
    ) -> Result<Self, ProtocolError> {
        let lease = pool.acquire(key, Some(owner_keypair)).await?;
        tracing::debug!(key, "AccountRecord opened");
        Ok(Self {
            lease,
            record_key: key.to_string(),
            encryption_key,
        })
    }

    /// Read and decrypt the account header.
    ///
    /// # Errors
    /// The header is missing, unreadable, or does not decrypt.
    pub async fn read_header(&self, pool: &dyn RecordPort) -> Result<AccountHeader, ProtocolError> {
        let value = pool
            .get(self.lease, 0, false)
            .await?
            .ok_or_else(|| ProtocolError::DhtError("account header not set".into()))?;
        let plaintext = self.encryption_key.decrypt(&value.data)?;
        Ok(decode_account_header(&plaintext)?)
    }

    /// Encrypt and write a new account header.
    ///
    /// # Errors
    /// The write failed outright (network outcomes come back as `SetOutcome`).
    pub async fn write_header(
        &self,
        pool: &dyn RecordPort,
        header: &AccountHeader,
    ) -> Result<SetOutcome, ProtocolError> {
        let ciphertext = self
            .encryption_key
            .encrypt(&encode_account_header(header))?;
        Ok(pool.set_durable(self.lease, 0, ciphertext, None).await?)
    }

    /// The record key as a string.
    pub fn record_key(&self) -> String {
        self.record_key.clone()
    }
}
