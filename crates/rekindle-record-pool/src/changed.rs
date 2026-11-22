//! A change our own read found (lead decision 57, finding RK3).
//!
//! A get that reaches the network stores what it read as our local copy:
//! 0.5.8 `storage_manager/get_value.rs:199-205` consumes the subkey's watch
//! hint and `process_outbound_get_value_result_locked` writes the returned
//! value locally. From then on Veilid reports nothing about that value: an
//! inbound watch report not newer than the local copy is dropped
//! (`watch_value.rs:2204-2229`, `if value.value_data().seq() <= last_seq`),
//! and a change inspection compares the network with the local copy, so it
//! finds nothing newer (`watch_value.rs:1455-1460`). A read that answered
//! with a newer seq than we held before it therefore absorbed the report a
//! watcher was owed: a history lookup that started before a peer's post
//! and answered after it hid the post from every handler.
//!
//! The pool is the only code that reads a record (ADR 0021, plan C7.2), so
//! it publishes that change itself, as the watch report it absorbed, into
//! the consumer the watch reports reach (the node's update stream). Every
//! reader is covered and none can swallow a change. A hinted read
//! ([`RecordPool::read_hinted`](crate::RecordPool::read_hinted)) is not
//! republished: it is the handler of the change that named those subkeys.

use std::sync::Arc;

use rekindle_records::port::{RecordValue, ValueChange};

use super::RecordPool;

/// Where a pool publishes a change its read found: the node's own consumer
/// of value changes (the transport's dispatch loop, the desktop's watch
/// handler, the netsim node's event queue).
pub type ChangeSink = Arc<dyn Fn(ValueChange) + Send + Sync>;

/// The `count` of a change a read found. It ends no watch (a count of zero
/// is a watch's last notification, which the pool re-arms on).
pub const READ_CHANGE_COUNT: u32 = u32::MAX;

impl RecordPool {
    /// Whether a lease watches `key`'s `subkey`: the subkeys whose changes
    /// have a consumer.
    pub(crate) fn watches_subkey(&self, key: &str, subkey: u32) -> bool {
        self.table.lock().watched(key).contains(&subkey)
    }

    /// Publish `value` of `key`'s `subkey` when its seq is newer than
    /// `before`, the local seq held before the read.
    pub(crate) fn publish_if_newer(
        &self,
        key: &str,
        subkey: u32,
        before: Option<u32>,
        value: &RecordValue,
    ) {
        if before.is_some_and(|seq| value.seq <= seq) {
            return;
        }
        tracing::debug!(
            key,
            subkey,
            ?before,
            seq = value.seq,
            "dht get: a read stored a newer value of a watched subkey; publishing the change"
        );
        (self.changes)(ValueChange {
            key: key.to_string(),
            subkeys: vec![subkey],
            count: READ_CHANGE_COUNT,
            value: Some(value.clone()),
        });
    }
}
