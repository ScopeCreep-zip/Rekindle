use serde::{Deserialize, Serialize};
use veilid_core::{
    DHTSchema, KeyPair, RecordKey, RoutingContext, ValueSubkeyRangeSet,
    CRYPTO_KIND_VLD0,
};

use crate::dht::short_array::DHTShortArray;
use crate::error::ProtocolError;

/// Default number of entries per segment DHTShortArray.
const DEFAULT_SEGMENT_CAPACITY: u16 = 255;

/// Internal metadata stored in subkey 0 of the spine DHT record.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogSpine {
    /// Total entries appended (monotonically increasing).
    total_count: u64,
    /// Maximum entries per segment.
    segment_capacity: u16,
    /// Ordered list of segment DHTShortArray record keys (oldest first).
    segments: Vec<String>,
}

/// An append-only log built on DHT records.
///
/// Architecture:
/// - **Spine record**: a single DHT record (1 subkey) holding metadata
///   that tracks total entry count and references to segment records.
/// - **Segments**: each segment is a [`DHTShortArray`] holding up to
///   `segment_capacity` entries. New segments are allocated automatically
///   when the latest segment fills up.
///
/// All segments share the same owner keypair as the spine, so only one
/// keypair needs to be persisted for write access.
pub struct DHTLog {
    routing_context: RoutingContext,
    spine_key: RecordKey,
    owner_keypair: Option<KeyPair>,
}

impl DHTLog {
    /// Create a new empty DHTLog.
    ///
    /// Returns the log and the owner keypair (which must be persisted for
    /// write access across sessions).
    pub async fn create(
        rc: &RoutingContext,
    ) -> Result<(Self, KeyPair), ProtocolError> {
        let schema = DHTSchema::dflt(1)
            .map_err(|e| {
                ProtocolError::DhtError(format!("invalid schema: {e}"))
            })?;

        let descriptor = rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, None)
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("create log spine: {e}"))
            })?;

        let key = descriptor.key().clone();
        let keypair = descriptor
            .owner_secret()
            .map(|secret| {
                KeyPair::new_from_parts(
                    descriptor.owner().clone(),
                    secret.value(),
                )
            })
            .ok_or_else(|| {
                ProtocolError::DhtError(
                    "no owner secret after create".into(),
                )
            })?;

        let spine = LogSpine {
            total_count: 0,
            segment_capacity: DEFAULT_SEGMENT_CAPACITY,
            segments: Vec::new(),
        };
        let spine_bytes = serde_json::to_vec(&spine)
            .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        rc.set_dht_value(key.clone(), 0, spine_bytes, None)
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("write spine: {e}"))
            })?;

        tracing::debug!(key = %key, "DHTLog created");

        Ok((
            Self {
                routing_context: rc.clone(),
                spine_key: key,
                owner_keypair: Some(keypair.clone()),
            },
            keypair,
        ))
    }

    /// Open an existing DHTLog with write access.
    ///
    /// The `writer` must be the keypair returned by [`create`].
    pub async fn open_write(
        rc: &RoutingContext,
        key: &str,
        writer: KeyPair,
    ) -> Result<Self, ProtocolError> {
        let spine_key: RecordKey = key
            .parse()
            .map_err(|e| {
                ProtocolError::DhtError(format!(
                    "invalid key '{key}': {e}"
                ))
            })?;

        let _ = rc
            .open_dht_record(spine_key.clone(), Some(writer.clone()))
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("open log spine: {e}"))
            })?;

        tracing::debug!(key, "DHTLog opened (write)");

        Ok(Self {
            routing_context: rc.clone(),
            spine_key,
            owner_keypair: Some(writer),
        })
    }

    /// Open an existing DHTLog for reading only.
    pub async fn open_read(
        rc: &RoutingContext,
        key: &str,
    ) -> Result<Self, ProtocolError> {
        let spine_key: RecordKey = key
            .parse()
            .map_err(|e| {
                ProtocolError::DhtError(format!(
                    "invalid key '{key}': {e}"
                ))
            })?;

        let _ = rc
            .open_dht_record(spine_key.clone(), None)
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("open log spine: {e}"))
            })?;

        tracing::debug!(key, "DHTLog opened (read)");

        Ok(Self {
            routing_context: rc.clone(),
            spine_key,
            owner_keypair: None,
        })
    }

    /// Append an entry to the log.
    ///
    /// Allocates a new segment if the latest segment is full.
    /// Returns the absolute position of the new entry.
    pub async fn append(
        &self,
        data: &[u8],
    ) -> Result<u64, ProtocolError> {
        let writer = self.owner_keypair.as_ref().ok_or_else(|| {
            ProtocolError::DhtError(
                "cannot append to read-only log".into(),
            )
        })?;

        let mut spine = self.read_spine().await?;
        let cap = spine.segment_capacity;

        // Determine if the latest segment is full (or no segments exist)
        let needs_new_segment = spine.segments.is_empty()
            || (spine.total_count > 0
                && spine.total_count % u64::from(cap) == 0);

        if needs_new_segment {
            // Allocate a new segment with the same owner keypair
            let (segment, _) = DHTShortArray::create(
                &self.routing_context,
                cap,
                Some(writer.clone()),
            )
            .await?;

            spine.segments.push(segment.record_key());

            // Write data to the new segment (it's already open from create)
            segment.add(data).await?;
        } else {
            // Open and write to the latest segment
            let latest_key =
                spine.segments.last().ok_or_else(|| {
                    ProtocolError::DhtError(
                        "no segments in spine".into(),
                    )
                })?;

            let segment = DHTShortArray::open(
                &self.routing_context,
                latest_key,
                Some(writer.clone()),
            )
            .await?;

            segment.add(data).await?;
        }

        // Update spine metadata
        let position = spine.total_count;
        spine.total_count += 1;
        self.write_spine(&spine).await?;

        Ok(position)
    }

    /// Read an entry at the given absolute position.
    ///
    /// Returns `None` if the position is beyond the current length.
    pub async fn get(
        &self,
        pos: u64,
    ) -> Result<Option<Vec<u8>>, ProtocolError> {
        let spine = self.read_spine().await?;

        if pos >= spine.total_count {
            return Ok(None);
        }

        let cap = u64::from(spine.segment_capacity);
        let segment_idx = (pos / cap) as usize;
        let offset = (pos % cap) as u32;

        if segment_idx >= spine.segments.len() {
            return Ok(None);
        }

        let segment = DHTShortArray::open(
            &self.routing_context,
            &spine.segments[segment_idx],
            self.owner_keypair.clone(),
        )
        .await?;

        segment.get(offset).await
    }

    /// Return the total number of entries in the log.
    pub async fn len(&self) -> Result<u64, ProtocolError> {
        let spine = self.read_spine().await?;
        Ok(spine.total_count)
    }

    /// Return whether the log is empty.
    pub async fn is_empty(&self) -> Result<bool, ProtocolError> {
        Ok(self.len().await? == 0)
    }

    /// Read the last `count` entries from the log.
    ///
    /// Returns entries in chronological order (oldest first).
    pub async fn tail(
        &self,
        count: u32,
    ) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let spine = self.read_spine().await?;
        let total = spine.total_count;

        if total == 0 || count == 0 {
            return Ok(Vec::new());
        }

        let start = total.saturating_sub(u64::from(count));
        let result_count = (total - start) as usize;
        let mut results = Vec::with_capacity(result_count);
        let cap = u64::from(spine.segment_capacity);

        // Group reads by segment for efficiency
        let mut current_segment_idx = (start / cap) as usize;
        let mut current_segment: Option<DHTShortArray> = None;

        for pos in start..total {
            let seg_idx = (pos / cap) as usize;
            let offset = (pos % cap) as u32;

            // Open new segment if we've moved to the next one
            if current_segment.is_none()
                || seg_idx != current_segment_idx
            {
                current_segment_idx = seg_idx;
                if seg_idx < spine.segments.len() {
                    current_segment = Some(
                        DHTShortArray::open(
                            &self.routing_context,
                            &spine.segments[seg_idx],
                            self.owner_keypair.clone(),
                        )
                        .await?,
                    );
                } else {
                    break;
                }
            }

            if let Some(ref segment) = current_segment {
                if let Some(data) = segment.get(offset).await? {
                    results.push(data);
                }
            }
        }

        Ok(results)
    }

    /// Watch the spine record for changes (new entries appended).
    ///
    /// When entries are appended, the spine's `total_count` changes,
    /// triggering a `VeilidUpdate::ValueChange` notification.
    pub async fn watch(&self) -> Result<bool, ProtocolError> {
        let subkeys: ValueSubkeyRangeSet =
            [0u32].iter().copied().collect();

        let active = self
            .routing_context
            .watch_dht_values(
                self.spine_key.clone(),
                Some(subkeys),
                None,
                None,
            )
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("watch spine: {e}"))
            })?;

        tracing::debug!(
            key = %self.spine_key,
            "DHTLog watch requested"
        );

        Ok(active)
    }

    /// Close the spine and all open segment records.
    pub async fn close(&self) -> Result<(), ProtocolError> {
        // Best-effort close all segments
        if let Ok(spine) = self.read_spine().await {
            for seg_key_str in &spine.segments {
                if let Ok(seg_key) =
                    seg_key_str.parse::<RecordKey>()
                {
                    let _ = self
                        .routing_context
                        .close_dht_record(seg_key)
                        .await;
                }
            }
        }

        // Close spine
        self.routing_context
            .close_dht_record(self.spine_key.clone())
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("close spine: {e}"))
            })?;

        Ok(())
    }

    /// Get the spine record key as a string.
    pub fn spine_key(&self) -> String {
        self.spine_key.to_string()
    }

    // -- Internal helpers --

    async fn read_spine(&self) -> Result<LogSpine, ProtocolError> {
        let value = self
            .routing_context
            .get_dht_value(self.spine_key.clone(), 0, false)
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("read spine: {e}"))
            })?;

        match value {
            Some(v) => serde_json::from_slice(v.data()).map_err(
                |e| {
                    ProtocolError::Deserialization(format!(
                        "spine parse: {e}"
                    ))
                },
            ),
            None => Err(ProtocolError::DhtError(
                "spine subkey not set".into(),
            )),
        }
    }

    async fn write_spine(
        &self,
        spine: &LogSpine,
    ) -> Result<(), ProtocolError> {
        let bytes = serde_json::to_vec(spine)
            .map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        self.routing_context
            .set_dht_value(self.spine_key.clone(), 0, bytes, None)
            .await
            .map_err(|e| {
                ProtocolError::DhtError(format!("write spine: {e}"))
            })?;
        Ok(())
    }
}
