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
mod tests {
    use std::time::Duration;

    use rekindle_records::port::{PortEvent, RecordSchema, RouteClass, SetOutcome};
    use rekindle_types::config::SafetyProfile;

    use crate::tests::{attached, recv};
    use crate::{Fault, Netsim};

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_sleeping_node_issues_no_dht_call_and_no_send_until_it_wakes() {
        let (sim, mut nodes) = Netsim::nodes(2, 41);
        let pump = sim.start();
        nodes[1].ports.routes.want_own();
        let blob = tokio::time::timeout(
            Duration::from_secs(5),
            nodes[1].ports.routes.available_own(RouteClass::General),
        )
        .await
        .unwrap();
        attached(&mut nodes[1]).await;
        let target = nodes[0].ports.routes.import(&blob).unwrap();
        let pool = nodes[0].ports.records.start_session().unwrap();
        let (lease, key, _) = pool
            .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
            .await
            .unwrap();

        sim.fault(Fault::Sleep {
            node: 0,
            asleep: true,
        });
        let writer = std::sync::Arc::clone(&pool);
        let set = tokio::spawn(async move { writer.set(lease, 0, b"v".to_vec(), None).await });
        let messages = nodes[0].ports.messages.clone();
        let send = tokio::spawn(async move {
            messages
                .send(&target, &SafetyProfile::default_dht(), b"hi".to_vec())
                .await
        });
        // Under the remote route cache's 5-minute idle expiry.
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(!set.is_finished() && !send.is_finished(), "both parked");
        assert_eq!(sim.app_sets(0), 0, "no DHT call left the node");
        assert_eq!(sim.network_get(&key, 0), None);
        assert!(nodes[1].events.try_recv().is_err(), "nothing was sent");

        sim.fault(Fault::Sleep {
            node: 0,
            asleep: false,
        });
        assert_eq!(set.await.unwrap().unwrap(), SetOutcome::Landed);
        send.await.unwrap().unwrap();
        assert!(
            matches!(recv(&mut nodes[1]).await, PortEvent::AppMessage { message, .. } if message == b"hi")
        );
        assert_eq!(sim.app_sets(0), 1);
        pump.abort();
    }
}
