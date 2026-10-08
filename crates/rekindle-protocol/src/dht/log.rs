use rekindle_records::lease::{LeaseId, SubkeySet};
use serde::{Deserialize, Serialize};
use veilid_core::{DHTSchema, KeyPair};

use super::parse_record_key;
use super::pool::RecordPool;
use crate::dht::short_array::DHTShortArray;
use crate::error::ProtocolError;

/// Default number of entries per segment `DHTShortArray`.
const DEFAULT_SEGMENT_CAPACITY: u16 = 255;

/// Internal metadata stored in subkey 0 of the spine DHT record.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogSpine {
    /// Total entries appended (monotonically increasing).
    total_count: u64,
    /// Maximum entries per segment.
    segment_capacity: u16,
    /// Ordered list of segment `DHTShortArray` record keys (oldest first).
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
///
/// The handle holds a lease on the spine in the session's [`RecordPool`]
/// (plan C7.4); each segment is borrowed for the one operation that needs
/// it and released after, so no segment open outlives its use. Call
/// [`release`](Self::release) when done.
pub struct DHTLog {
    spine: LeaseId,
    spine_key: String,
    writer: Option<KeyPair>,
}

impl DHTLog {
    /// Create a new empty `DHTLog`, held writable.
    ///
    /// Returns the log and the owner keypair (which must be persisted for
    /// write access across sessions).
    ///
    /// # Errors
    /// The spine could not be created, or its first value not stored.
    pub async fn create(pool: &RecordPool) -> Result<(Self, KeyPair), ProtocolError> {
        let schema = DHTSchema::dflt(1)
            .map_err(|e| ProtocolError::DhtError(format!("invalid schema: {e}")))?;
        let (spine, key, keypair) = pool.create(schema, None).await?;
        let log = Self {
            spine,
            spine_key: key.to_string(),
            writer: Some(keypair.clone()),
        };
        log.write_spine(
            pool,
            &LogSpine {
                total_count: 0,
                segment_capacity: DEFAULT_SEGMENT_CAPACITY,
                segments: Vec::new(),
            },
        )
        .await?;
        tracing::debug!(key = %log.spine_key, "DHTLog created");
        Ok((log, keypair))
    }

    /// Open an existing `DHTLog` with write access. The `writer` must be the
    /// keypair returned by [`create`](Self::create).
    ///
    /// # Errors
    /// The spine could not be opened.
    pub async fn open_write(
        pool: &RecordPool,
        key: &str,
        writer: KeyPair,
    ) -> Result<Self, ProtocolError> {
        let spine = pool
            .acquire(&parse_record_key(key)?, Some(writer.clone()))
            .await?;
        tracing::debug!(key, "DHTLog opened (write)");
        Ok(Self {
            spine,
            spine_key: key.to_string(),
            writer: Some(writer),
        })
    }

    /// Open an existing `DHTLog` for reading only.
    ///
    /// # Errors
    /// The spine could not be opened.
    pub async fn open_read(pool: &RecordPool, key: &str) -> Result<Self, ProtocolError> {
        let spine = pool.acquire(&parse_record_key(key)?, None).await?;
        tracing::debug!(key, "DHTLog opened (read)");
        Ok(Self {
            spine,
            spine_key: key.to_string(),
            writer: None,
        })
    }

    /// End this handle's borrow of the spine.
    pub async fn release(self, pool: &RecordPool) {
        pool.release(self.spine).await;
    }

    /// Append an entry to the log, allocating a new segment if the latest
    /// one is full. Returns the absolute position of the new entry.
    ///
    /// # Errors
    /// The log is read-only, or a write was not stored.
    pub async fn append(&self, pool: &RecordPool, data: &[u8]) -> Result<u64, ProtocolError> {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| ProtocolError::DhtError("cannot append to read-only log".into()))?;
        let mut spine = self.read_spine(pool).await?;
        let cap = spine.segment_capacity;
        let needs_new_segment = spine.segments.is_empty()
            || (spine.total_count > 0 && spine.total_count % u64::from(cap) == 0);

        let segment = if needs_new_segment {
            let (segment, _) = DHTShortArray::create(pool, cap, Some(writer.clone())).await?;
            spine.segments.push(segment.record_key().to_string());
            segment
        } else {
            let latest_key = spine
                .segments
                .last()
                .ok_or_else(|| ProtocolError::DhtError("no segments in spine".into()))?;
            DHTShortArray::open(pool, latest_key, Some(writer.clone())).await?
        };
        let added = segment.add(pool, data).await;
        segment.release(pool).await;
        added?;

        let position = spine.total_count;
        spine.total_count += 1;
        self.write_spine(pool, &spine).await?;
        Ok(position)
    }

    /// Read the entry at an absolute position, or `None` past the end.
    ///
    /// # Errors
    /// The spine or the segment could not be read.
    pub async fn get(&self, pool: &RecordPool, pos: u64) -> Result<Option<Vec<u8>>, ProtocolError> {
        let spine = self.read_spine(pool).await?;
        if pos >= spine.total_count {
            return Ok(None);
        }
        let cap = u64::from(spine.segment_capacity);
        let segment_idx = usize::try_from(pos / cap).unwrap_or(usize::MAX);
        let offset = u32::try_from(pos % cap).unwrap_or(u32::MAX);
        let Some(segment_key) = spine.segments.get(segment_idx) else {
            return Ok(None);
        };
        let segment = DHTShortArray::open(pool, segment_key, None).await?;
        let value = segment.get(pool, offset).await;
        segment.release(pool).await;
        value
    }

    /// Return the total number of entries in the log.
    ///
    /// # Errors
    /// The spine could not be read.
    pub async fn len(&self, pool: &RecordPool) -> Result<u64, ProtocolError> {
        Ok(self.read_spine(pool).await?.total_count)
    }

    /// Return whether the log is empty.
    ///
    /// # Errors
    /// The spine could not be read.
    pub async fn is_empty(&self, pool: &RecordPool) -> Result<bool, ProtocolError> {
        Ok(self.len(pool).await? == 0)
    }

    /// Read the last `count` entries, oldest first.
    ///
    /// # Errors
    /// The spine or a segment could not be read.
    pub async fn tail(&self, pool: &RecordPool, count: u32) -> Result<Vec<Vec<u8>>, ProtocolError> {
        let spine = self.read_spine(pool).await?;
        let total = spine.total_count;
        if total == 0 || count == 0 {
            return Ok(Vec::new());
        }
        let start = total.saturating_sub(u64::from(count));
        let cap = u64::from(spine.segment_capacity);
        let mut results = Vec::with_capacity(usize::try_from(total - start).unwrap_or(0));

        // One borrow per segment, released before the next.
        let mut pos = start;
        while pos < total {
            let seg_idx = usize::try_from(pos / cap).unwrap_or(usize::MAX);
            let Some(segment_key) = spine.segments.get(seg_idx) else {
                break;
            };
            let segment_end = (u64::try_from(seg_idx).unwrap_or(u64::MAX) + 1)
                .saturating_mul(cap)
                .min(total);
            let segment = DHTShortArray::open(pool, segment_key, None).await?;
            let mut read = Ok(());
            while pos < segment_end {
                let offset = u32::try_from(pos % cap).unwrap_or(u32::MAX);
                match segment.get(pool, offset).await {
                    Ok(Some(data)) => results.push(data),
                    Ok(None) => {}
                    Err(e) => {
                        read = Err(e);
                        break;
                    }
                }
                pos += 1;
            }
            segment.release(pool).await;
            read?;
        }
        Ok(results)
    }

    /// Watch the spine for appends (its `total_count` changes).
    ///
    /// # Errors
    /// Veilid refused the watch request.
    pub async fn watch(&self, pool: &RecordPool) -> Result<(), ProtocolError> {
        pool.watch(self.spine, SubkeySet::from([0])).await
    }

    /// Get the spine record key as a string.
    #[must_use]
    pub fn spine_key(&self) -> &str {
        &self.spine_key
    }

    async fn read_spine(&self, pool: &RecordPool) -> Result<LogSpine, ProtocolError> {
        match pool.get(self.spine, 0, false).await? {
            Some(v) => serde_json::from_slice(v.data())
                .map_err(|e| ProtocolError::Deserialization(format!("spine parse: {e}"))),
            None => Err(ProtocolError::DhtError("spine subkey not set".into())),
        }
    }

    async fn write_spine(&self, pool: &RecordPool, spine: &LogSpine) -> Result<(), ProtocolError> {
        let bytes =
            serde_json::to_vec(spine).map_err(|e| ProtocolError::Serialization(e.to_string()))?;
        pool.set(self.spine, 0, bytes, None)
            .await?
            .require_stored(0)
    }
}

#[cfg(test)]
mod wire_tests {
    use super::LogSpine;
    use serde::{Deserialize, Serialize};

    /// `rekindle-transport`'s spine struct, copied verbatim from
    /// `broadcast/dht/channel_log.rs` as it stood at commit 51e9815,
    /// immediately before that duplicate engine was deleted in favour of
    /// this one.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TransportLogSpine {
        total_count: u64,
        segment_capacity: u16,
        segments: Vec<String>,
    }

    /// The merge is only safe if both engines wrote the same bytes: a
    /// spine record written before it must still load after it. This
    /// compares the real `LogSpine` — not a copy of it — so reordering
    /// or renaming a field here fails the test.
    #[test]
    fn spine_wire_matches_the_replaced_transport_engine() {
        let mine = LogSpine {
            total_count: 511,
            segment_capacity: 255,
            segments: vec!["VLD0:aaaa".into(), "VLD0:bbbb".into()],
        };
        let theirs = TransportLogSpine {
            total_count: 511,
            segment_capacity: 255,
            segments: vec!["VLD0:aaaa".into(), "VLD0:bbbb".into()],
        };
        assert_eq!(
            serde_json::to_vec(&mine).unwrap(),
            serde_json::to_vec(&theirs).unwrap(),
            "spine bytes diverged from the engine this one replaced"
        );
        assert_eq!(
            serde_json::to_string(&mine).unwrap(),
            r#"{"total_count":511,"segment_capacity":255,"segments":["VLD0:aaaa","VLD0:bbbb"]}"#,
            "spine field order is wire-visible; serde emits declaration order"
        );
    }

    /// Decode direction: a spine written by the old engine must load
    /// here, which is what a peer does with an existing record.
    #[test]
    fn transport_written_spine_loads() {
        let bytes = serde_json::to_vec(&TransportLogSpine {
            total_count: 1_000_000,
            segment_capacity: 255,
            segments: vec!["VLD0:zzzz".into()],
        })
        .unwrap();
        let read: LogSpine = serde_json::from_slice(&bytes).expect("must load");
        assert_eq!(read.total_count, 1_000_000);
        assert_eq!(read.segment_capacity, 255);
        assert_eq!(read.segments, vec!["VLD0:zzzz".to_string()]);
    }
}
