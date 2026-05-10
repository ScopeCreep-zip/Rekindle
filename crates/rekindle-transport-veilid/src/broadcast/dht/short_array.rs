//! Short array — an ordered collection stored across DHT subkeys.
//!
//! Layout:
//! - Subkey 0: head record with index map and stride
//! - Subkeys 1..=stride: data slots holding element bytes
//!
//! Used by `DhtLog` (channel_log.rs) for append-only message logs.
//! Max 255 elements per array. Multiple arrays are chained for larger logs.

use serde::{Deserialize, Serialize};
use veilid_core::{KeyPair, RoutingContext, CRYPTO_KIND_VLD0, DHTSchema};

use super::record;
use crate::error::{TransportError, Result};

/// Short array head metadata (subkey 0).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShortArrayHead {
    stride: u16,
    slots: Vec<u16>,
}

/// An ordered collection stored across DHT subkeys (max 255 elements).
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
        let head_bytes = serde_json::to_vec(&head)
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

        #[allow(clippy::cast_possible_truncation)]
        let index = head.slots.len() as u32;
        head.slots.push(slot);
        self.write_head(&head).await?;
        Ok(index)
    }

    /// Get element at logical index.
    pub async fn get(&self, index: u32) -> Result<Option<Vec<u8>>> {
        self.get_impl(index, false).await
    }

    /// Get element at logical index, bypassing Veilid's local DHT cache.
    pub async fn get_fresh(&self, index: u32) -> Result<Option<Vec<u8>>> {
        self.get_impl(index, true).await
    }

    async fn get_impl(&self, index: u32, force_refresh: bool) -> Result<Option<Vec<u8>>> {
        let head = self.read_head().await?;
        let idx = index as usize;
        if idx >= head.slots.len() { return Ok(None); }

        let subkey = u32::from(head.slots[idx]) + 1;
        let value = self.rc.get_dht_value(self.record_key.clone(), subkey, force_refresh)
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
        let bytes = serde_json::to_vec(head)
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
        Some(v) => serde_json::from_slice(v.data()).map_err(|e| {
            TransportError::DeserializationFailed { type_id: 0, reason: format!("head: {e}") }
        }),
        None => Err(TransportError::DhtError { reason: "head subkey not set".into() }),
    }
}

fn find_free_slot(stride: u16, head: &ShortArrayHead) -> u16 {
    (0..stride).find(|s| !head.slots.contains(s)).unwrap_or(stride)
}
