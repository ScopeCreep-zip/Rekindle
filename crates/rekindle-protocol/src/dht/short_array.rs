use rekindle_records::lease::LeaseId;
use serde::{Deserialize, Serialize};
use veilid_core::{DHTSchema, KeyPair};

use super::parse_record_key;
use super::pool::RecordPool;
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
///
/// The handle holds a lease in the session's [`RecordPool`] (plan C7.4):
/// writes are signed by the lease's writer, and [`release`](Self::release)
/// ends the borrow. A write the structure depends on that does not reach
/// consensus is an error ([`ProtocolError::NotStored`]).
pub struct DHTShortArray {
    lease: LeaseId,
    record_key: String,
    stride: u16,
}

impl DHTShortArray {
    /// Create a new `DHTShortArray` with the given capacity, owned by
    /// `owner` (a fresh key when `None`), and hold it writable.
    ///
    /// Returns the array and the owner keypair (which must be persisted for
    /// write access across sessions).
    ///
    /// # Errors
    /// The record could not be created, or its empty head not stored.
    pub async fn create(
        pool: &RecordPool,
        capacity: u16,
        owner: Option<KeyPair>,
    ) -> Result<(Self, KeyPair), ProtocolError> {
        let total_subkeys = capacity
            .checked_add(1)
            .ok_or_else(|| ProtocolError::DhtError("capacity overflow (max 65534)".into()))?;
        let schema = DHTSchema::dflt(total_subkeys)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;
        let (lease, key, keypair) = pool.create(schema, owner).await?;
        let array = Self {
            lease,
            record_key: key.to_string(),
            stride: capacity,
        };
        array
            .write_head(
                pool,
                &ShortArrayHead {
                    stride: capacity,
                    slots: Vec::new(),
                },
            )
            .await?;
        tracing::debug!(key = %array.record_key, capacity, "DHTShortArray created");
        Ok((array, keypair))
    }

    /// Open an existing `DHTShortArray`: writable with `writer`, or
    /// read-only with `None`.
    ///
    /// # Errors
    /// The record could not be opened, or its head read.
    pub async fn open(
        pool: &RecordPool,
        key: &str,
        writer: Option<KeyPair>,
    ) -> Result<Self, ProtocolError> {
        let lease = pool.acquire(&parse_record_key(key)?, writer).await?;
        let head = match read_head(pool, lease).await {
            Ok(head) => head,
            Err(e) => {
                pool.release(lease).await;
                return Err(e);
            }
        };
        tracing::debug!(
            key,
            stride = head.stride,
            len = head.slots.len(),
            "DHTShortArray opened"
        );
        Ok(Self {
            lease,
            record_key: key.to_string(),
            stride: head.stride,
        })
    }

    /// End this handle's borrow of the record.
    pub async fn release(self, pool: &RecordPool) {
        pool.release(self.lease).await;
    }

    /// Add an element to the end of the array.
    ///
    /// Returns the logical index of the new element.
    ///
    /// # Errors
    /// The array is full, or a write was not stored.
    pub async fn add(&self, pool: &RecordPool, data: &[u8]) -> Result<u32, ProtocolError> {
        let mut head = read_head(pool, self.lease).await?;
        if head.slots.len() >= usize::from(self.stride) {
            return Err(ProtocolError::DhtError("short array is full".into()));
        }
        let slot = find_free_slot(self.stride, &head);
        let subkey = u32::from(slot) + 1;
        pool.set(self.lease, subkey, data.to_vec(), None)
            .await?
            .require_stored(subkey)?;
        let index = u32::try_from(head.slots.len())
            .map_err(|e| ProtocolError::DhtError(format!("index overflow: {e}")))?;
        head.slots.push(slot);
        self.write_head(pool, &head).await?;
        Ok(index)
    }

    /// Get element data at the given logical index, or `None` when out of
    /// bounds.
    ///
    /// # Errors
    /// The head or the slot could not be read.
    pub async fn get(
        &self,
        pool: &RecordPool,
        index: u32,
    ) -> Result<Option<Vec<u8>>, ProtocolError> {
        let head = read_head(pool, self.lease).await?;
        let Some(&slot) = head.slots.get(index as usize) else {
            return Ok(None);
        };
        let value = pool.get(self.lease, u32::from(slot) + 1, false).await?;
        Ok(value.map(|v| v.data().to_vec()))
    }

    /// Remove the element at the given logical index. Subsequent elements
    /// shift down by one logical index.
    ///
    /// # Errors
    /// The index is out of bounds, or a write was not stored.
    pub async fn remove(&self, pool: &RecordPool, index: u32) -> Result<(), ProtocolError> {
        let mut head = read_head(pool, self.lease).await?;
        let idx = index as usize;
        let Some(&slot) = head.slots.get(idx) else {
            return Err(ProtocolError::DhtError(format!(
                "index {index} out of bounds (len={})",
                head.slots.len()
            )));
        };
        let subkey = u32::from(slot) + 1;
        pool.set(self.lease, subkey, Vec::new(), None)
            .await?
            .require_stored(subkey)?;
        head.slots.remove(idx);
        self.write_head(pool, &head).await
    }

    /// Return the number of elements in the array.
    ///
    /// # Errors
    /// The head could not be read.
    pub async fn len(&self, pool: &RecordPool) -> Result<u32, ProtocolError> {
        let head = read_head(pool, self.lease).await?;
        u32::try_from(head.slots.len())
            .map_err(|e| ProtocolError::DhtError(format!("len overflow: {e}")))
    }

    /// Return whether the array is empty.
    ///
    /// # Errors
    /// The head could not be read.
    pub async fn is_empty(&self, pool: &RecordPool) -> Result<bool, ProtocolError> {
        Ok(self.len(pool).await? == 0)
    }

    /// Clear all elements from the array.
    ///
    /// # Errors
    /// A write was not stored.
    pub async fn clear(&self, pool: &RecordPool) -> Result<(), ProtocolError> {
        let head = read_head(pool, self.lease).await?;
        for &slot in &head.slots {
            let subkey = u32::from(slot) + 1;
            pool.set(self.lease, subkey, Vec::new(), None)
                .await?
                .require_stored(subkey)?;
        }
        self.write_head(
            pool,
            &ShortArrayHead {
                stride: self.stride,
                slots: Vec::new(),
            },
        )
        .await
    }

    /// Get all elements in logical order.
    ///
    /// # Errors
    /// The head or a slot could not be read.
    pub async fn get_all(&self, pool: &RecordPool) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let head = read_head(pool, self.lease).await?;
        let mut results = Vec::with_capacity(head.slots.len());
        for &slot in &head.slots {
            let value = pool.get(self.lease, u32::from(slot) + 1, false).await?;
            results.push(value.map(|v| v.data().to_vec()).unwrap_or_default());
        }
        Ok(results)
    }

    /// Get the record key as a string.
    #[must_use]
    pub fn record_key(&self) -> &str {
        &self.record_key
    }

    /// Get the maximum capacity of this array.
    #[must_use]
    pub fn capacity(&self) -> u16 {
        self.stride
    }

    async fn write_head(
        &self,
        pool: &RecordPool,
        head: &ShortArrayHead,
    ) -> Result<(), ProtocolError> {
        let bytes =
            serde_json::to_vec(head).map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        pool.set(self.lease, 0, bytes, None)
            .await?
            .require_stored(0)
    }
}

/// Read the head metadata of a leased array record.
async fn read_head(pool: &RecordPool, lease: LeaseId) -> Result<ShortArrayHead, ProtocolError> {
    match pool.get(lease, 0, false).await? {
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

#[cfg(test)]
mod wire_tests {
    use super::{find_free_slot, ShortArrayHead};
    use serde::{Deserialize, Serialize};

    /// `rekindle-transport`'s head struct, copied verbatim from
    /// `broadcast/dht/account.rs` at commit 51e9815, immediately before
    /// that duplicate was deleted in favour of this one.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TransportShortArrayHead {
        stride: u16,
        slots: Vec<u16>,
    }

    #[test]
    fn head_wire_matches_the_replaced_transport_engine() {
        let mine = ShortArrayHead {
            stride: 255,
            slots: vec![0, 3, 7, 254],
        };
        let theirs = TransportShortArrayHead {
            stride: 255,
            slots: vec![0, 3, 7, 254],
        };
        assert_eq!(
            serde_json::to_vec(&mine).unwrap(),
            serde_json::to_vec(&theirs).unwrap(),
            "segment head bytes diverged from the engine this one replaced"
        );
        assert_eq!(
            serde_json::to_string(&mine).unwrap(),
            r#"{"stride":255,"slots":[0,3,7,254]}"#
        );
    }

    /// Slot order is the logical element order — it must survive a
    /// round trip intact, not be normalised or sorted.
    #[test]
    fn transport_written_head_loads_with_slot_order_intact() {
        let bytes = serde_json::to_vec(&TransportShortArrayHead {
            stride: 255,
            slots: vec![9, 1, 4],
        })
        .unwrap();
        let read: ShortArrayHead = serde_json::from_slice(&bytes).expect("must load");
        assert_eq!(read.slots, vec![9, 1, 4]);
    }

    /// Slot allocation drives which subkey an element lands on, so both
    /// engines had to agree on it. They did — identical implementations.
    #[test]
    fn free_slot_picks_lowest_gap() {
        let head = ShortArrayHead {
            stride: 8,
            slots: vec![0, 1, 3],
        };
        assert_eq!(find_free_slot(8, &head), 2, "must reuse the lowest gap");

        let full = ShortArrayHead {
            stride: 3,
            slots: vec![0, 1, 2],
        };
        assert_eq!(
            find_free_slot(3, &full),
            3,
            "a full array returns stride; callers check capacity first"
        );
    }
}
