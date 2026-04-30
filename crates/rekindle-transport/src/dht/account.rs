//! Account DHT record operations (DFLT, 1 subkey + child DHTShortArrays).
//!
//! The account record contains an encrypted header pointing to child arrays
//! for contacts, chats, and invitations. The header is encrypted with a key
//! derived from the identity's Ed25519 secret (only the owner can read it).
//!
//! Child arrays use the short-array pattern: subkey 0 holds an index map
//! (ordered list of occupied slot numbers), subkeys 1..=capacity hold data.

use serde::{Deserialize, Serialize};
use veilid_core::{KeyPair, RoutingContext, CRYPTO_KIND_VLD0, DHTSchema};

use super::record;
use crate::error::{TransportError, Result};

/// Encrypted account header stored in subkey 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountHeader {
    pub contact_list_key: String,
    pub chat_list_key: String,
    pub invitation_list_key: String,
    pub display_name: String,
    pub status_message: String,
    pub avatar_hash: Vec<u8>,
    pub created_at: u64,
    pub updated_at: u64,
    pub contact_list_keypair: Option<String>,
    pub chat_list_keypair: Option<String>,
    pub invitation_list_keypair: Option<String>,
}

/// Short array head metadata (subkey 0 of a child DHTShortArray).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShortArrayHead {
    stride: u16,
    slots: Vec<u16>,
}

/// An ordered collection stored across DHT subkeys (max 255 elements).
///
/// Layout:
/// - Subkey 0: head record with index map and stride
/// - Subkeys 1..=stride: data slots holding element bytes
pub struct ShortArray {
    rc: RoutingContext,
    record_key: veilid_core::RecordKey,
    stride: u16,
}

impl ShortArray {
    /// Create a new short array with the given capacity.
    pub async fn create(
        rc: &RoutingContext,
        capacity: u16,
        owner: Option<KeyPair>,
    ) -> Result<(Self, KeyPair)> {
        let total = capacity.checked_add(1).ok_or_else(|| {
            TransportError::RecordCreateFailed { reason: "capacity overflow".into() }
        })?;

        let schema = DHTSchema::dflt(total)
            .map_err(|e| TransportError::RecordCreateFailed { reason: format!("{e}") })?;

        let desc = rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, owner)
            .await
            .map_err(|e| TransportError::RecordCreateFailed { reason: format!("{e}") })?;

        let key = desc.key().clone();
        let keypair = desc
            .owner_secret()
            .map(|s| KeyPair::new_from_parts(desc.owner().clone(), s.value()))
            .ok_or_else(|| TransportError::RecordCreateFailed {
                reason: "no owner secret".into(),
            })?;

        let head = ShortArrayHead { stride: capacity, slots: Vec::new() };
        let head_bytes = postcard::to_stdvec(&head)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        rc.set_dht_value(key.clone(), 0, head_bytes, None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("write head: {e}") })?;

        Ok((Self { rc: rc.clone(), record_key: key, stride: capacity }, keypair))
    }

    /// Open an existing short array.
    pub async fn open(rc: &RoutingContext, key: &str, writer: Option<KeyPair>) -> Result<Self> {
        let rk = record::parse_key(key)?;
        let _ = rc.open_dht_record(rk.clone(), writer)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("open short array: {e}") })?;

        let head = read_head_raw(rc, &rk).await?;
        Ok(Self { rc: rc.clone(), record_key: rk, stride: head.stride })
    }

    /// Add an element. Returns the logical index.
    pub async fn add(&self, data: &[u8]) -> Result<u32> {
        let mut head = self.read_head().await?;
        if head.slots.len() >= usize::from(self.stride) {
            return Err(TransportError::DhtError { reason: "short array full".into() });
        }

        let slot = find_free_slot(self.stride, &head);
        let subkey = u32::from(slot) + 1;
        self.rc.set_dht_value(self.record_key.clone(), subkey, data.to_vec(), None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("write slot: {e}") })?;

        #[allow(clippy::cast_possible_truncation)] // ShortArray max 255 elements
        let index = head.slots.len() as u32;
        head.slots.push(slot);
        self.write_head(&head).await?;
        Ok(index)
    }

    /// Get element at logical index.
    pub async fn get(&self, index: u32) -> Result<Option<Vec<u8>>> {
        let head = self.read_head().await?;
        let idx = index as usize;
        if idx >= head.slots.len() { return Ok(None); }

        let subkey = u32::from(head.slots[idx]) + 1;
        let value = self.rc.get_dht_value(self.record_key.clone(), subkey, false)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("read slot: {e}") })?;
        Ok(value.map(|v| v.data().to_vec()))
    }

    /// Remove element at logical index.
    pub async fn remove(&self, index: u32) -> Result<()> {
        let mut head = self.read_head().await?;
        let idx = index as usize;
        if idx >= head.slots.len() {
            return Err(TransportError::DhtError {
                reason: format!("index {index} out of bounds (len={})", head.slots.len()),
            });
        }

        let slot = head.slots[idx];
        let subkey = u32::from(slot) + 1;
        self.rc.set_dht_value(self.record_key.clone(), subkey, vec![], None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("clear slot: {e}") })?;

        head.slots.remove(idx);
        self.write_head(&head).await
    }

    /// Get all elements in logical order.
    pub async fn get_all(&self) -> Result<Vec<Vec<u8>>> {
        let head = self.read_head().await?;
        let mut results = Vec::with_capacity(head.slots.len());
        for &slot in &head.slots {
            let subkey = u32::from(slot) + 1;
            let value = self.rc.get_dht_value(self.record_key.clone(), subkey, false)
                .await
                .map_err(|e| TransportError::DhtError { reason: format!("read: {e}") })?;
            results.push(value.map(|v| v.data().to_vec()).unwrap_or_default());
        }
        Ok(results)
    }

    /// Close the underlying record.
    pub async fn close(&self) -> Result<()> {
        self.rc.close_dht_record(self.record_key.clone())
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("close: {e}") })
    }

    pub fn record_key(&self) -> String { self.record_key.to_string() }

    async fn read_head(&self) -> Result<ShortArrayHead> {
        read_head_raw(&self.rc, &self.record_key).await
    }

    async fn write_head(&self, head: &ShortArrayHead) -> Result<()> {
        let bytes = postcard::to_stdvec(head)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        self.rc.set_dht_value(self.record_key.clone(), 0, bytes, None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("write head: {e}") })?;
        Ok(())
    }
}

async fn read_head_raw(rc: &RoutingContext, key: &veilid_core::RecordKey) -> Result<ShortArrayHead> {
    let value = rc.get_dht_value(key.clone(), 0, false)
        .await
        .map_err(|e| TransportError::DhtError { reason: format!("read head: {e}") })?;
    match value {
        Some(v) => postcard::from_bytes(v.data()).map_err(|e| {
            TransportError::DeserializationFailed { type_id: 0, reason: format!("head: {e}") }
        }),
        None => Err(TransportError::DhtError { reason: "head subkey not set".into() }),
    }
}

fn find_free_slot(stride: u16, head: &ShortArrayHead) -> u16 {
    (0..stride).find(|s| !head.slots.contains(s)).unwrap_or(stride)
}
