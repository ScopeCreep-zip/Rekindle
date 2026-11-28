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
//!
//! What counts as "newer" is the ledger of lead decision (61): per watched
//! subkey, the newest seq already handed to a consumer, by a delivered watch
//! report's value, a hinted read's result, or a publish of the pool's own.
//! A read publishes only a seq past that ledger, never against a snapshot of
//! the local seq taken when the read started: two concurrent reads of one
//! subkey both start before either stores, so a snapshot let both publish
//! the same seq. Checked and advanced under one lock, the ledger publishes
//! each seq at most once, whichever read answers first. A first local copy
//! (no ledger entry yet) is past the ledger: Veilid's change inspection on a
//! watch's establishment reports a network value newer than the local copy,
//! a missing one included (0.5.8 `watch_value.rs:1433-1516`).

use std::collections::HashMap;
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

/// The newest seq handed to a consumer, per (record, watched subkey).
pub(crate) type Handed = HashMap<(String, u32), u32>;

impl RecordPool {
    /// Whether a lease watches `key`'s `subkey`: the subkeys whose changes
    /// have a consumer.
    pub(crate) fn watches_subkey(&self, key: &str, subkey: u32) -> bool {
        self.table.lock().watched(key).contains(&subkey)
    }

    /// Record `seq` of `key`'s `subkey` as handed to a consumer; true when
    /// it is past every seq handed before (and so new to the consumer).
    fn hand(&self, key: &str, subkey: u32, seq: u32) -> bool {
        let mut handed = self.handed.lock();
        let at = (key.to_string(), subkey);
        if handed.get(&at).is_some_and(|&last| seq <= last) {
            return false;
        }
        handed.insert(at, seq);
        true
    }

    /// A delivered watch report hands its value (the first subkey's,
    /// `VeilidValueChange::value`) to the consumer.
    pub(crate) fn note_report(&self, change: &ValueChange) {
        if let (Some(&subkey), Some(value)) = (change.subkeys.first(), change.value.as_ref()) {
            if self.watches_subkey(&change.key, subkey) {
                self.hand(&change.key, subkey, value.seq);
            }
        }
    }

    /// A read of `key`'s `subkey` answered `value`. For a watched subkey the
    /// value is handed to the reader; when `publish` (any read but a hinted
    /// one, whose caller is the change's handler) and its seq is past the
    /// ledger, the change is published too.
    pub(crate) fn hand_read(&self, key: &str, subkey: u32, value: &RecordValue, publish: bool) {
        if !self.watches_subkey(key, subkey) || !self.hand(key, subkey, value.seq) || !publish {
            return;
        }
        tracing::debug!(
            key,
            subkey,
            seq = value.seq,
            "dht get: a read found a watched subkey's seq no consumer had; publishing the change"
        );
        (self.changes)(ValueChange {
            key: key.to_string(),
            subkeys: vec![subkey],
            count: READ_CHANGE_COUNT,
            value: Some(value.clone()),
        });
    }

    /// A closed record's ledger goes with it.
    pub(crate) fn forget_handed(&self, key: &str) {
        self.handed.lock().retain(|(k, _), _| k != key);
    }
}
