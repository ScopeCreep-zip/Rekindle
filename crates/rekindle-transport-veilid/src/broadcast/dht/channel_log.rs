//! Per-channel SMPL message record and append-only log operations.
//!
//! Each channel has its own DHT record for storing message history.
//! Channel records use zero-owner SMPL: each member writes to the
//! subkey matching their registry slot index.
//!
//! Additionally provides a `DhtLog` — an append-only log built on DHT
//! records for conversation message persistence, using a spine + segments
//! architecture.

use serde::{Deserialize, Serialize};
use veilid_core::{KeyPair, RoutingContext, ValueSubkeyRangeSet, CRYPTO_KIND_VLD0, DHTSchema};

use super::record;
use super::short_array::ShortArray;
use crate::error::{TransportError, Result};
use crate::payload::dht_types::ChannelMessage;

/// Operations on per-channel SMPL message records.
pub struct ChannelLogOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> ChannelLogOps<'a> {
    pub fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Read a channel message from a member's subkey.
    pub async fn read_message(
        &self,
        key: &str,
        slot_index: u32,
        force_refresh: bool,
    ) -> Result<Option<ChannelMessage>> {
        match record::get(self.rc, key, slot_index, force_refresh).await? {
            Some(data) if !data.is_empty() => {
                let msg: ChannelMessage = serde_json::from_slice(&data).map_err(|e| {
                    TransportError::DeserializationFailed {
                        type_id: 0,
                        reason: format!("channel message: {e}"),
                    }
                })?;
                Ok(Some(msg))
            }
            _ => Ok(None),
        }
    }

    /// Write a channel message to the member's subkey.
    pub async fn write_message(
        &self,
        key: &str,
        slot_index: u32,
        message: &ChannelMessage,
        writer: KeyPair,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(message)
            .map_err(|e| TransportError::SerializationFailed {
                reason: format!("channel message: {e}"),
            })?;
        record::set(self.rc, key, slot_index, bytes, Some(writer)).await
    }

    /// Open a channel record for reading.
    pub async fn open_readonly(&self, key: &str) -> Result<()> {
        record::open_readonly(self.rc, key).await
    }

    /// Open a channel record with write access.
    pub async fn open_writable(&self, key: &str, writer: KeyPair) -> Result<()> {
        record::open_writable(self.rc, key, writer).await
    }

    /// Watch all subkeys of a channel record.
    pub async fn watch(&self, key: &str, subkey_count: u32) -> Result<bool> {
        let subkeys: Vec<u32> = (0..subkey_count).collect();
        record::watch(self.rc, key, &subkeys).await
    }

    /// Close the channel record.
    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }
}

// ── Append-only DhtLog (spine + segments) ────────────────────────────

/// Default number of entries per segment ShortArray.
const DEFAULT_SEGMENT_CAPACITY: u16 = 255;

/// Spine metadata stored in subkey 0 of the log's root record.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogSpine {
    total_count: u64,
    segment_capacity: u16,
    segments: Vec<String>,
}

/// An append-only log built on DHT records.
///
/// Architecture:
/// - **Spine record**: DFLT(1) holding metadata (total count + segment keys)
/// - **Segments**: each segment is a [`ShortArray`] holding up to
///   `segment_capacity` entries. New segments allocated when full.
///
/// The `append_guard` mutex serializes spine read-modify-write to prevent
/// lost updates when concurrent dispatch processes two messages for the
/// same channel simultaneously.
pub struct DhtLog {
    rc: RoutingContext,
    spine_key: veilid_core::RecordKey,
    owner_keypair: Option<KeyPair>,
    /// Guards the append path's spine read-modify-write against concurrent calls.
    /// None for read-only logs (no append possible, no mutex needed).
    append_guard: Option<tokio::sync::Mutex<()>>,
}

impl DhtLog {
    /// Create a new empty log.
    pub async fn create(rc: &RoutingContext) -> Result<(Self, KeyPair)> {
        let schema = DHTSchema::dflt(1)
            .map_err(|e| TransportError::RecordCreateFailed { reason: format!("{e}") })?;
        let desc = rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| TransportError::RecordCreateFailed { reason: format!("{e}") })?;

        let key = desc.key().clone();
        let keypair = desc
            .owner_secret()
            .map(|s| KeyPair::new_from_parts(desc.owner().clone(), s.value()))
            .ok_or_else(|| TransportError::RecordCreateFailed { reason: "no secret".into() })?;

        let spine = LogSpine {
            total_count: 0,
            segment_capacity: DEFAULT_SEGMENT_CAPACITY,
            segments: Vec::new(),
        };
        let bytes = serde_json::to_vec(&spine)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        rc.set_dht_value(key.clone(), 0, bytes, None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("write spine: {e}") })?;

        Ok((Self { rc: rc.clone(), spine_key: key, owner_keypair: Some(keypair.clone()), append_guard: Some(tokio::sync::Mutex::new(())) }, keypair))
    }

    /// Open an existing log with write access.
    pub async fn open_write(rc: &RoutingContext, key: &str, writer: KeyPair) -> Result<Self> {
        let spine_key = record::parse_key(key)?;
        let _ = rc.open_dht_record(spine_key.clone(), Some(writer.clone()))
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("open log: {e}") })?;
        Ok(Self { rc: rc.clone(), spine_key, owner_keypair: Some(writer), append_guard: Some(tokio::sync::Mutex::new(())) })
    }

    /// Open an existing log for reading only.
    pub async fn open_read(rc: &RoutingContext, key: &str) -> Result<Self> {
        let spine_key = record::parse_key(key)?;
        let _ = rc.open_dht_record(spine_key.clone(), None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("open log: {e}") })?;
        Ok(Self { rc: rc.clone(), spine_key, owner_keypair: None, append_guard: None })
    }

    /// Append an entry to the log. Returns the absolute position.
    ///
    /// Serialized by `append_guard` to prevent lost updates when concurrent
    /// dispatch processes two messages for the same channel simultaneously.
    pub async fn append(&self, data: &[u8]) -> Result<u64> {
        let writer = self.owner_keypair.as_ref().ok_or_else(|| {
            TransportError::DhtError { reason: "cannot append to read-only log".into() }
        })?;

        let _guard = match &self.append_guard {
            Some(m) => Some(m.lock().await),
            None => None,
        };
        let mut spine = self.read_spine().await?;
        let cap = spine.segment_capacity;

        let needs_new = spine.segments.is_empty()
            || (spine.total_count > 0 && spine.total_count % u64::from(cap) == 0);

        if needs_new {
            let (segment, _) = ShortArray::create(&self.rc, cap, Some(writer.clone())).await?;
            spine.segments.push(segment.record_key());
            segment.add(data).await?;
        } else {
            let latest_key = spine.segments.last().ok_or_else(|| {
                TransportError::DhtError { reason: "no segments".into() }
            })?;
            let segment = ShortArray::open(&self.rc, latest_key, Some(writer.clone())).await?;
            segment.add(data).await?;
        }

        let position = spine.total_count;
        spine.total_count += 1;
        self.write_spine(&spine).await?;
        Ok(position)
    }

    /// Read the last N entries (oldest first).
    ///
    /// Uses force_refresh on spine read to ensure we see entries written
    /// by other nodes (cross-node DhtLog reads bypass Veilid's local cache).
    pub async fn tail(&self, count: u32) -> Result<Vec<Vec<u8>>> {
        let spine = self.read_spine_fresh().await?;
        if spine.total_count == 0 || count == 0 {
            return Ok(Vec::new());
        }

        let start = spine.total_count.saturating_sub(u64::from(count));
        let cap = u64::from(spine.segment_capacity);
        let mut results = Vec::new();
        let mut current_seg_idx = usize::MAX;
        let mut current_seg: Option<ShortArray> = None;

        for pos in start..spine.total_count {
            #[allow(clippy::cast_possible_truncation)] // segment index bounded by spine.segments.len()
            let seg_idx = (pos / cap) as usize;
            #[allow(clippy::cast_possible_truncation)] // offset < segment_capacity (u32)
            let offset = (pos % cap) as u32;

            if current_seg.is_none() || seg_idx != current_seg_idx {
                current_seg_idx = seg_idx;
                if seg_idx < spine.segments.len() {
                    current_seg = Some(
                        ShortArray::open(&self.rc, &spine.segments[seg_idx], self.owner_keypair.clone()).await?,
                    );
                } else {
                    break;
                }
            }

            if let Some(ref seg) = current_seg {
                if let Some(data) = seg.get_fresh(offset).await? {
                    results.push(data);
                }
            }
        }
        Ok(results)
    }

    /// Total entries in the log.
    pub async fn len(&self) -> Result<u64> {
        Ok(self.read_spine().await?.total_count)
    }

    /// Watch the spine for new entries.
    pub async fn watch(&self) -> Result<bool> {
        let range: ValueSubkeyRangeSet = [0u32].iter().copied().collect();
        self.rc
            .watch_dht_values(self.spine_key.clone(), Some(range), None, None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("watch: {e}") })
    }

    /// Close spine and all segment records.
    pub async fn close(&self) -> Result<()> {
        if let Ok(spine) = self.read_spine().await {
            for seg_key_str in &spine.segments {
                if let Ok(seg_key) = seg_key_str.parse::<veilid_core::RecordKey>() {
                    let _ = self.rc.close_dht_record(seg_key).await;
                }
            }
        }
        self.rc.close_dht_record(self.spine_key.clone())
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("close: {e}") })
    }

    pub fn spine_key(&self) -> String { self.spine_key.to_string() }

    async fn read_spine(&self) -> Result<LogSpine> {
        if let Ok(spine) = self.read_spine_impl(false).await {
            Ok(spine)
        } else {
            // Local cache may be empty on first access to a remote record.
            // Retry with force_refresh to fetch from the network.
            tracing::debug!(spine_key = %self.spine_key, "spine cache miss — retrying with force_refresh");
            self.read_spine_impl(true).await
        }
    }

    /// Read spine with force_refresh — bypasses Veilid's local DHT cache.
    /// Used by tail() to ensure cross-node reads see the latest entries.
    async fn read_spine_fresh(&self) -> Result<LogSpine> {
        self.read_spine_impl(true).await
    }

    async fn read_spine_impl(&self, force_refresh: bool) -> Result<LogSpine> {
        let value = self.rc.get_dht_value(self.spine_key.clone(), 0, force_refresh)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("read spine: {e}") })?;
        match value {
            Some(v) => serde_json::from_slice(v.data()).map_err(|e| {
                TransportError::DeserializationFailed { type_id: 0, reason: format!("spine: {e}") }
            }),
            None => Err(TransportError::DhtError { reason: "spine not set".into() }),
        }
    }

    async fn write_spine(&self, spine: &LogSpine) -> Result<()> {
        let bytes = serde_json::to_vec(spine)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        self.rc.set_dht_value(self.spine_key.clone(), 0, bytes, None)
            .await
            .map_err(|e| TransportError::DhtError { reason: format!("write spine: {e}") })?;
        Ok(())
    }
}
