//! Per-channel SMPL message record and append-only log operations.
//!
//! Each channel has its own DHT record for storing message history.
//! Channel records use zero-owner SMPL: each member writes to the
//! subkey matching their registry slot index.
//!
//! Additionally provides a `DhtLog` — an append-only log built on DHT
//! records for conversation message persistence, using a spine + segments
//! architecture.

use veilid_core::{KeyPair, RoutingContext};

use super::record;
use crate::error::{Result, TransportError};
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
        let bytes =
            serde_json::to_vec(message).map_err(|e| TransportError::SerializationFailed {
                reason: format!("channel message: {e}"),
            })?;
        record::set(self.rc, key, slot_index, bytes, Some(writer))
            .await
            .map(|_| ())
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
// ── Append-only log ──────────────────────────────────────────────────
//
// `DhtLog` is `rekindle_protocol`'s `DHTLog`, not a second copy of it.
// This crate carried a parallel implementation — same LogSpine fields,
// same DEFAULT_SEGMENT_CAPACITY, same spine+segments algorithm, with
// `append()` line-for-line identical bar the error type — over its own
// duplicate of DHTShortArray underneath. Both wrote the identical wire
// format, so the duplication bought nothing and could only drift; it
// had already started to (transport's `add` clamped an index overflow
// to u32::MAX where protocol returns an error).
//
// `?` lifts ProtocolError into TransportError via the per-variant
// conversion in crate::error.
pub use rekindle_protocol::dht::log::DHTLog as DhtLog;
