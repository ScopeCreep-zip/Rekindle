//! The watched-record notifications' own worker.
//!
//! A value change's handler reads the network: the Host inspects and gets
//! the changed subkeys of a friend's profile or participant record, an
//! invite inbox, or a community's governance record, and the record pool
//! retries those reads while the DHT is unreachable. Run inline in the
//! dispatch loop, that read held every event behind it, so a 1:1 hint (an
//! `app_message`, which needs no DHT) waited on a DHT read (netsim
//! `dm_new_friendship::a_re_hint_then_its_chain_copy_is_one_message_acked_by_its_chain_seq`:
//! a profile change that came in just before Jon's DHT went offline kept
//! Ida's "solo" hint undelivered for the whole offline span). The loop
//! hands the change here and goes on. One worker applies the changes in
//! the order they came, so a record's notifications never overtake each
//! other. The pool's own bookkeeping (re-arming a dead watch) stays in the
//! loop: it is local and immediate.

use std::sync::Arc;

use rekindle_records::port::ValueChange;
use tokio::sync::mpsc;

use crate::handler::InboundHandler;

/// The dispatch loop's side of the worker. Dropping it (the loop ended)
/// ends the worker's future once the queue is drained.
pub(crate) struct ValueChanges {
    tx: mpsc::UnboundedSender<ValueChange>,
}

impl ValueChanges {
    /// The queue and the worker that applies its changes to `handler`. The
    /// dispatch loop owns the worker task and stops it at shutdown, so a
    /// change still reading the network never holds an app message back.
    ///
    /// The queue is unbounded so the loop never waits on it: what fills it
    /// is the record watches' notifications, at most one per watched record
    /// per change, and every queued change is applied, never dropped (a
    /// lost notification is a change no one reads until the next one).
    pub(crate) fn new<H: InboundHandler>(
        handler: Arc<H>,
    ) -> (Self, impl std::future::Future<Output = ()> + Send) {
        let (tx, mut rx) = mpsc::unbounded_channel::<ValueChange>();
        let worker = async move {
            while let Some(change) = rx.recv().await {
                let first_value = change.value.map(|v| v.data);
                handler
                    .on_value_change(&change.key, change.subkeys, first_value)
                    .await;
            }
        };
        (Self { tx }, worker)
    }

    /// Queue `change` for the worker; returns at once.
    pub(crate) fn queue(&self, change: ValueChange) {
        if self.tx.send(change).is_err() {
            tracing::warn!("value change worker stopped; change dropped");
        }
    }
}

#[cfg(test)]
#[path = "value_changes_tests.rs"]
mod tests;
