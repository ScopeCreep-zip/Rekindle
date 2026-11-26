//! A suspended process ([`crate::Fault::Sleep`]): while a node sleeps, none
//! of its code reaches the network. Every DHT call, app message, app call,
//! reply and route allocation it makes parks at the port until it wakes, and
//! the answer to a call in flight when it fell asleep is handed back only
//! after the wake; its storage tick does not run (`records/watches.rs`) and
//! nothing sent to it arrives (`network.rs` `lost_in_transit`). Its wall
//! clock reads the instant it fell asleep until the wake, then jumps across
//! the sleep ([`crate::SimClock::freeze`]): a suspended process reads no
//! clock, and its first reading after the wake sees the whole gap.

use std::sync::Arc;

use rekindle_records::port::{
    DhtBackend, DhtCallError, OpenIntent, RecordDescriptor, RecordReport, RecordSchema,
    RecordValue, ReportScope, WriterKey,
};

use crate::network::Network;

impl Network {
    /// Wait until `node` is awake (at once when it is).
    pub(crate) async fn awake(&self, node: usize) {
        let mut woke = self.woke.subscribe();
        while self.inner.lock().faults.asleep.contains(&node) {
            if woke.changed().await.is_err() {
                return;
            }
        }
    }

    /// `node` fell asleep or woke: wake every call parked on a sleep.
    pub(crate) fn sleep_changed(&self) {
        self.woke.send_replace(());
    }
}

/// A session pool's backend on a node that may sleep: each call waits for
/// the node to be awake before it starts and before it answers.
pub(crate) struct Sleepable<B> {
    node: usize,
    net: Arc<Network>,
    inner: B,
}

impl<B> Sleepable<B> {
    pub(crate) fn new(node: usize, net: Arc<Network>, inner: B) -> Self {
        Self { node, net, inner }
    }

    async fn run<T>(&self, call: impl std::future::Future<Output = T>) -> T {
        self.net.awake(self.node).await;
        let out = call.await;
        self.net.awake(self.node).await;
        out
    }
}

#[async_trait::async_trait]
impl<B: DhtBackend> DhtBackend for Sleepable<B> {
    fn parse_writer(&self, text: &str) -> Result<WriterKey, DhtCallError> {
        self.inner.parse_writer(text)
    }

    fn check_key(&self, key: &str) -> Result<String, DhtCallError> {
        self.inner.check_key(key)
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        self.run(self.inner.create(schema, owner)).await
    }

    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
        intent: OpenIntent,
    ) -> Result<RecordDescriptor, DhtCallError> {
        self.run(self.inner.open(key, writer, intent)).await
    }

    async fn close(&self, key: &str) -> Result<(), DhtCallError> {
        self.run(self.inner.close(key)).await
    }

    async fn get(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.run(self.inner.get(key, subkey, force_refresh)).await
    }

    async fn get_newest(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.run(self.inner.get_newest(key, subkey)).await
    }

    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.run(self.inner.set(key, subkey, data, writer)).await
    }

    async fn set_above(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
        min_seq: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.run(self.inner.set_above(key, subkey, data, writer, min_seq))
            .await
    }

    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        self.run(self.inner.inspect(key, subkeys, scope)).await
    }

    async fn watch(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        expiration_us: Option<u64>,
        count: Option<u32>,
    ) -> Result<bool, DhtCallError> {
        self.run(self.inner.watch(key, subkeys, expiration_us, count))
            .await
    }

    async fn cancel_watch(&self, key: &str, subkeys: Option<&[u32]>) -> Result<bool, DhtCallError> {
        self.run(self.inner.cancel_watch(key, subkeys)).await
    }
}

#[cfg(test)]
#[path = "sleep_tests.rs"]
mod tests;
