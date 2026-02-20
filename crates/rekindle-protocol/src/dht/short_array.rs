use serde::{Deserialize, Serialize};
use veilid_core::{DHTSchema, KeyPair, RecordKey, RoutingContext, CRYPTO_KIND_VLD0};

use crate::error::ProtocolError;

/// Internal metadata stored in subkey 0 of the `DHTShortArray` record.
///
/// Tracks the logical ordering of elements by mapping each logical index
/// to a physical slot number. The actual DHT subkey = slot + 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShortArrayHead {
    /// Maximum number of data slots (subkeys 1..=stride hold element data).
    stride: u16,
    /// Ordered list of occupied slot indices (0-based).
    /// The logical index of an element is its position in this Vec.
    /// Physical DHT subkey = slots[i] + 1.
    slots: Vec<u16>,
}

/// An ordered collection stored across DHT subkeys (max 255 elements).
///
/// Layout:
/// - Subkey 0: head record with index map and stride
/// - Subkeys 1..=stride: data slots holding element bytes
///
/// Elements are addressed by logical index (position in the ordered list).
/// The head record maps logical indices to physical subkey slots, enabling
/// O(1) removal without shifting data in DHT.
pub struct DHTShortArray {
    routing_context: RoutingContext,
    record_key: RecordKey,
    owner_keypair: Option<KeyPair>,
    stride: u16,
}

impl DHTShortArray {
    /// Create a new `DHTShortArray` with the given capacity.
    ///
    /// If `owner` is `Some`, the record is created with that keypair as owner.
    /// If `None`, a new random keypair is generated.
    ///
    /// Returns the array and the owner keypair (which must be persisted for
    /// write access across sessions).
    pub async fn create(
        rc: &RoutingContext,
        capacity: u16,
        owner: Option<KeyPair>,
    ) -> Result<(Self, KeyPair), ProtocolError> {
        let total_subkeys = capacity
            .checked_add(1)
            .ok_or_else(|| ProtocolError::DhtError("capacity overflow (max 65534)".into()))?;

        let schema = DHTSchema::dflt(total_subkeys)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;

        let descriptor = rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, owner)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("create short array record: {e}")))?;

        let key = descriptor.key().clone();
        let keypair = descriptor
            .owner_secret()
            .map(|secret| KeyPair::new_from_parts(descriptor.owner().clone(), secret.value()))
            .ok_or_else(|| ProtocolError::DhtError("no owner secret after create".into()))?;

        // Write initial empty head
        let head = ShortArrayHead {
            stride: capacity,
            slots: Vec::new(),
        };
        let head_bytes =
            serde_json::to_vec(&head).map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        rc.set_dht_value(key.clone(), 0, head_bytes, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write head: {e}")))?;

        tracing::debug!(key = %key, capacity, "DHTShortArray created");

        Ok((
            Self {
                routing_context: rc.clone(),
                record_key: key,
                owner_keypair: Some(keypair.clone()),
                stride: capacity,
            },
            keypair,
        ))
    }

    /// Open an existing `DHTShortArray` for reading or writing.
    ///
    /// Pass `writer: Some(keypair)` for write access, or `None` for read-only.
    pub async fn open(
        rc: &RoutingContext,
        key: &str,
        writer: Option<KeyPair>,
    ) -> Result<Self, ProtocolError> {
        let record_key: RecordKey = key
            .parse()
            .map_err(|e| ProtocolError::DhtError(format!("invalid key '{key}': {e}")))?;

        let _ = rc
            .open_dht_record(record_key.clone(), writer.clone())
            .await
            .map_err(|e| ProtocolError::DhtError(format!("open short array: {e}")))?;

        let head = read_head_raw(rc, &record_key).await?;

        tracing::debug!(
            key,
            stride = head.stride,
            len = head.slots.len(),
            "DHTShortArray opened"
        );

        Ok(Self {
            routing_context: rc.clone(),
            record_key,
            owner_keypair: writer,
            stride: head.stride,
        })
    }

    /// Add an element to the end of the array.
    ///
    /// Returns the logical index of the new element.
    pub async fn add(&self, data: &[u8]) -> Result<u32, ProtocolError> {
        let mut head = self.read_head().await?;

        if head.slots.len() >= usize::from(self.stride) {
            return Err(ProtocolError::DhtError("short array is full".into()));
        }

        let slot = find_free_slot(self.stride, &head);
        let subkey = u32::from(slot) + 1;

        // Write data to the physical slot
        self.routing_context
            .set_dht_value(self.record_key.clone(), subkey, data.to_vec(), None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write slot {slot}: {e}")))?;

        // Append slot to index map
        let index = u32::try_from(head.slots.len())
            .map_err(|e| ProtocolError::DhtError(format!("index overflow: {e}")))?;
        head.slots.push(slot);
        self.write_head(&head).await?;

        Ok(index)
    }

    /// Get element data at the given logical index.
    ///
    /// Returns `None` if the index is out of bounds.
    pub async fn get(&self, index: u32) -> Result<Option<Vec<u8>>, ProtocolError> {
        let head = self.read_head().await?;
        let idx = index as usize;

        if idx >= head.slots.len() {
            return Ok(None);
        }

        let subkey = u32::from(head.slots[idx]) + 1;
        let value = self
            .routing_context
            .get_dht_value(self.record_key.clone(), subkey, false)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("read slot: {e}")))?;

        Ok(value.map(|v| v.data().to_vec()))
    }

    /// Remove the element at the given logical index.
    ///
    /// Subsequent elements shift down by one logical index.
    pub async fn remove(&self, index: u32) -> Result<(), ProtocolError> {
        let mut head = self.read_head().await?;
        let idx = index as usize;

        if idx >= head.slots.len() {
            return Err(ProtocolError::DhtError(format!(
                "index {index} out of bounds (len={})",
                head.slots.len()
            )));
        }

        let slot = head.slots[idx];
        let subkey = u32::from(slot) + 1;

        // Clear the physical data slot
        self.routing_context
            .set_dht_value(self.record_key.clone(), subkey, vec![], None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("clear slot {slot}: {e}")))?;

        // Remove from index map
        head.slots.remove(idx);
        self.write_head(&head).await?;

        Ok(())
    }

    /// Return the number of elements in the array.
    pub async fn len(&self) -> Result<u32, ProtocolError> {
        let head = self.read_head().await?;
        u32::try_from(head.slots.len())
            .map_err(|e| ProtocolError::DhtError(format!("len overflow: {e}")))
    }

    /// Return whether the array is empty.
    pub async fn is_empty(&self) -> Result<bool, ProtocolError> {
        Ok(self.len().await? == 0)
    }

    /// Clear all elements from the array.
    pub async fn clear(&self) -> Result<(), ProtocolError> {
        let head = self.read_head().await?;

        // Clear all occupied data slots
        for &slot in &head.slots {
            let subkey = u32::from(slot) + 1;
            self.routing_context
                .set_dht_value(self.record_key.clone(), subkey, vec![], None)
                .await
                .map_err(|e| ProtocolError::DhtError(format!("clear slot {slot}: {e}")))?;
        }

        // Reset head to empty
        let empty_head = ShortArrayHead {
            stride: self.stride,
            slots: Vec::new(),
        };
        self.write_head(&empty_head).await
    }

    /// Get all elements as a Vec of byte arrays, in logical order.
    pub async fn get_all(&self) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let head = self.read_head().await?;
        let mut results = Vec::with_capacity(head.slots.len());

        for &slot in &head.slots {
            let subkey = u32::from(slot) + 1;
            let value = self
                .routing_context
                .get_dht_value(self.record_key.clone(), subkey, false)
                .await
                .map_err(|e| ProtocolError::DhtError(format!("read slot: {e}")))?;
            results.push(value.map(|v| v.data().to_vec()).unwrap_or_default());
        }

        Ok(results)
    }

    /// Close the underlying DHT record.
    pub async fn close(&self) -> Result<(), ProtocolError> {
        self.routing_context
            .close_dht_record(self.record_key.clone())
            .await
            .map_err(|e| ProtocolError::DhtError(format!("close: {e}")))?;
        Ok(())
    }

    /// Get the record key as a string.
    pub fn record_key(&self) -> String {
        self.record_key.to_string()
    }

    /// Get the maximum capacity of this array.
    pub fn capacity(&self) -> u16 {
        self.stride
    }

    /// Get the owner keypair (if this array was opened with write access).
    pub fn owner_keypair(&self) -> Option<&KeyPair> {
        self.owner_keypair.as_ref()
    }

    // -- Internal helpers --

    async fn read_head(&self) -> Result<ShortArrayHead, ProtocolError> {
        read_head_raw(&self.routing_context, &self.record_key).await
    }

    async fn write_head(&self, head: &ShortArrayHead) -> Result<(), ProtocolError> {
        let bytes =
            serde_json::to_vec(head).map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        self.routing_context
            .set_dht_value(self.record_key.clone(), 0, bytes, None)
            .await
            .map_err(|e| ProtocolError::DhtError(format!("write head: {e}")))?;
        Ok(())
    }
}

/// Read the head metadata from a DHT record.
async fn read_head_raw(
    rc: &RoutingContext,
    key: &RecordKey,
) -> Result<ShortArrayHead, ProtocolError> {
    let value = rc
        .get_dht_value(key.clone(), 0, false)
        .await
        .map_err(|e| ProtocolError::DhtError(format!("read head: {e}")))?;

    match value {
        Some(v) => serde_json::from_slice(v.data())
            .map_err(|e| ProtocolError::Deserialization(format!("head parse: {e}"))),
        None => Err(ProtocolError::DhtError("head subkey not set".into())),
    }
}

/// Find the lowest unused slot index in the head's slot list.
fn find_free_slot(stride: u16, head: &ShortArrayHead) -> u16 {
    for slot in 0..stride {
        if !head.slots.contains(&slot) {
            return slot;
        }
    }
    // Caller checks capacity before calling this, so this should not happen.
    // But if it does, return stride (will be caught by DHT write failure).
    stride
}
