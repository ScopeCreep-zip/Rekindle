use veilid_core::{DHTSchema, KeyPair, RecordKey, RoutingContext, CRYPTO_KIND_VLD0};

use super::parse_record_key;
use crate::capnp_codec::account::{
    decode_account_header, decode_chat_entry, decode_contact_entry, encode_account_header,
    encode_chat_entry, encode_contact_entry, AccountHeader, ChatEntry, ContactEntry,
};
use crate::dht::short_array::DHTShortArray;
use crate::error::ProtocolError;
use rekindle_crypto::DhtRecordKey;

/// A user's private account DHT record.
///
/// Contains pointers to child `DHTShortArray`s (contact list, chat list,
/// invitation list) and an encrypted `AccountHeader` in subkey 0.
///
/// Only the owner can read this record — it's encrypted with a key derived
/// from the identity's Ed25519 secret.
pub struct AccountRecord {
    routing_context: RoutingContext,
    record_key: RecordKey,
    owner_keypair: KeyPair,
    encryption_key: DhtRecordKey,
    contact_list_key: Option<String>,
    chat_list_key: Option<String>,
    invitation_list_key: Option<String>,
    /// Owner keypair for the contact list `DHTShortArray` (unique per child).
    contact_list_keypair: Option<KeyPair>,
    /// Owner keypair for the chat list `DHTShortArray` (unique per child).
    chat_list_keypair: Option<KeyPair>,
    /// Owner keypair for the invitation list `DHTShortArray` (unique per child).
    invitation_list_keypair: Option<KeyPair>,
}

impl AccountRecord {
    /// Create a new account record with child `DHTShortArray`s.
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

        // Create child DHTShortArrays with unique random keypairs (None = random).
        // Passing the same owner keypair to all three would produce the same
        // deterministic record key, causing "record already exists" errors.
        let (contacts, contacts_kp) = DHTShortArray::create(rc, 255, None).await?;
        let (chats, chats_kp) = DHTShortArray::create(rc, 255, None).await?;
        let (invitations, invitations_kp) = DHTShortArray::create(rc, 255, None).await?;

        let contact_list_key = contacts.record_key();
        let chat_list_key = chats.record_key();
        let invitation_list_key = invitations.record_key();

        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);

        let header = AccountHeader {
            contact_list_key: contact_list_key.clone(),
            chat_list_key: chat_list_key.clone(),
            invitation_list_key: invitation_list_key.clone(),
            display_name: display_name.to_string(),
            status_message: status_message.to_string(),
            avatar_hash: Vec::new(),
            created_at: now,
            updated_at: now,
            // Persist child keypairs in the encrypted header so they survive restarts
            contact_list_keypair: Some(contacts_kp.to_string()),
            chat_list_keypair: Some(chats_kp.to_string()),
            invitation_list_keypair: Some(invitations_kp.to_string()),
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
                contact_list_key: Some(contact_list_key),
                chat_list_key: Some(chat_list_key),
                invitation_list_key: Some(invitation_list_key),
                contact_list_keypair: Some(contacts_kp),
                chat_list_keypair: Some(chats_kp),
                invitation_list_keypair: Some(invitations_kp),
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
            .map_err(|e| ProtocolError::DhtError(format!("open account record: {e}")))?;

        // Read and decrypt header to populate child key pointers and keypairs
        let value = rc
            .get_dht_value(record_key.clone(), 0, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("read account header: {e}")))?;

        let (
            contact_list_key,
            chat_list_key,
            invitation_list_key,
            contact_list_keypair,
            chat_list_keypair,
            invitation_list_keypair,
        ) = match value {
            Some(v) => {
                let plaintext = encryption_key.decrypt(v.data())?;
                let header = decode_account_header(&plaintext)?;
                let clk = header.contact_list_keypair.and_then(|s| s.parse().ok());
                let chk = header.chat_list_keypair.and_then(|s| s.parse().ok());
                let ilk = header.invitation_list_keypair.and_then(|s| s.parse().ok());
                (
                    Some(header.contact_list_key),
                    Some(header.chat_list_key),
                    Some(header.invitation_list_key),
                    clk,
                    chk,
                    ilk,
                )
            }
            None => (None, None, None, None, None, None),
        };

        tracing::debug!(key, "AccountRecord opened");

        Ok(Self {
            routing_context: rc.clone(),
            record_key,
            owner_keypair,
            encryption_key,
            contact_list_key,
            chat_list_key,
            invitation_list_key,
            contact_list_keypair,
            chat_list_keypair,
            invitation_list_keypair,
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

    /// Add a contact entry to the contact list `DHTShortArray`.
    pub async fn add_contact(&self, entry: &ContactEntry) -> Result<u32, ProtocolError> {
        let key = self
            .contact_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("contact list key not set".into()))?;

        let arr = DHTShortArray::open(
            &self.routing_context,
            key,
            self.contact_list_keypair.clone(),
        )
        .await?;

        let data = encode_contact_entry(entry);
        let index = arr.add(&data).await?;
        Ok(index)
    }

    /// Read all contact entries from the contact list.
    pub async fn read_contacts(&self) -> Result<Vec<ContactEntry>, ProtocolError> {
        let key = self
            .contact_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("contact list key not set".into()))?;

        let arr = DHTShortArray::open(
            &self.routing_context,
            key,
            self.contact_list_keypair.clone(),
        )
        .await?;

        let all_data = arr.get_all().await?;
        let mut entries = Vec::with_capacity(all_data.len());
        for data in all_data {
            if !data.is_empty() {
                entries.push(decode_contact_entry(&data)?);
            }
        }
        Ok(entries)
    }

    /// Remove a contact by public key from the contact list.
    pub async fn remove_contact(&self, public_key: &[u8]) -> Result<(), ProtocolError> {
        let key = self
            .contact_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("contact list key not set".into()))?;

        let arr = DHTShortArray::open(
            &self.routing_context,
            key,
            self.contact_list_keypair.clone(),
        )
        .await?;

        let all_data = arr.get_all().await?;
        for (i, data) in all_data.iter().enumerate() {
            if !data.is_empty() {
                if let Ok(entry) = decode_contact_entry(data) {
                    if entry.public_key == public_key {
                        arr.remove(u32::try_from(i).unwrap_or(u32::MAX)).await?;
                        return Ok(());
                    }
                }
            }
        }

        Err(ProtocolError::DhtError("contact not found in list".into()))
    }

    /// Add a chat entry to the chat list `DHTShortArray`.
    pub async fn add_chat(&self, entry: &ChatEntry) -> Result<u32, ProtocolError> {
        let key = self
            .chat_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("chat list key not set".into()))?;

        let arr =
            DHTShortArray::open(&self.routing_context, key, self.chat_list_keypair.clone()).await?;

        let data = encode_chat_entry(entry);
        let index = arr.add(&data).await?;
        Ok(index)
    }

    /// Read all chat entries from the chat list.
    pub async fn read_chats(&self) -> Result<Vec<ChatEntry>, ProtocolError> {
        let key = self
            .chat_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("chat list key not set".into()))?;

        let arr =
            DHTShortArray::open(&self.routing_context, key, self.chat_list_keypair.clone()).await?;

        let all_data = arr.get_all().await?;
        let mut entries = Vec::with_capacity(all_data.len());
        for data in all_data {
            if !data.is_empty() {
                entries.push(decode_chat_entry(&data)?);
            }
        }
        Ok(entries)
    }

    /// Read all entries from the invitation list `DHTShortArray`.
    pub async fn read_invitations(&self) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let key = self
            .invitation_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("invitation list key not set".into()))?;

        let arr = DHTShortArray::open(
            &self.routing_context,
            key,
            self.invitation_list_keypair.clone(),
        )
        .await?;

        arr.get_all().await
    }

    /// Add raw data to the invitation list `DHTShortArray`.
    pub async fn add_invitation(&self, data: &[u8]) -> Result<u32, ProtocolError> {
        let key = self
            .invitation_list_key
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("invitation list key not set".into()))?;

        let arr = DHTShortArray::open(
            &self.routing_context,
            key,
            self.invitation_list_keypair.clone(),
        )
        .await?;

        arr.add(data).await
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

    /// Get the invitation list `DHTShortArray` key (if loaded).
    pub fn invitation_list_key(&self) -> Option<&str> {
        self.invitation_list_key.as_deref()
    }

    /// Get the contact list `DHTShortArray` key (if loaded).
    pub fn contact_list_key(&self) -> Option<&str> {
        self.contact_list_key.as_deref()
    }

    /// Get the chat list `DHTShortArray` key (if loaded).
    pub fn chat_list_key(&self) -> Option<&str> {
        self.chat_list_key.as_deref()
    }

    /// Return all DHT record keys owned by this account (parent + children).
    ///
    /// Used for bulk close on logout / app exit.
    pub fn all_record_keys(&self) -> Vec<String> {
        let mut keys = vec![self.record_key.to_string()];
        if let Some(ref k) = self.contact_list_key {
            keys.push(k.clone());
        }
        if let Some(ref k) = self.chat_list_key {
            keys.push(k.clone());
        }
        if let Some(ref k) = self.invitation_list_key {
            keys.push(k.clone());
        }
        keys
    }
}
