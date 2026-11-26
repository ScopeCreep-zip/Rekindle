//! The value-change worker: queuing never waits on a handler that is still
//! reading the network, and the changes are applied in the order they came.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_codec::community::envelope::{CommunityEnvelope, SignedEnvelope};
use rekindle_records::port::{RecordValue, ValueChange};
use tokio::sync::Semaphore;

use super::ValueChanges;
use crate::handler::{InboundHandler, TransportEvent};
use crate::payload::rpc::{CallResponse, InboundCall};

/// One applied change: its record key, subkeys and first value.
type Applied = (String, Vec<u32>, Option<Vec<u8>>);

/// A handler whose every value change waits for one permit (a DHT read
/// that has not returned), then records the key.
struct Gated {
    permits: Semaphore,
    applied: Mutex<Vec<Applied>>,
}

impl InboundHandler for Gated {
    fn local_identity(&self) -> Option<[u8; 32]> {
        None
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        rekindle_lifecycle::SessionScope::closed("test")
    }

    fn clock(&self) -> &dyn rekindle_utils::Clock {
        &rekindle_utils::SystemClock
    }

    async fn on_direct(&self, _datagram: &[u8]) {}

    fn on_media(&self, _raw: &[u8], _arrived: std::time::Instant) {}

    async fn on_direct_call(&self, _datagram: &[u8], _route: Option<&str>) -> Option<Vec<u8>> {
        None
    }

    async fn on_gossip(&self, _: &str, _: &str, _: CommunityEnvelope, _: u64) {}

    async fn on_gossip_forward(&self, _envelope: &SignedEnvelope) {}

    async fn on_head_advanced(&self, _community: &str, _sender: &str, _hint: &[u8]) -> bool {
        false
    }

    async fn on_call(&self, _sender: Option<&str>, _request: InboundCall) -> CallResponse {
        CallResponse::Ack
    }

    async fn on_value_change(&self, key: &str, subkeys: Vec<u32>, first: Option<Vec<u8>>) {
        if let Ok(permit) = self.permits.acquire().await {
            permit.forget();
        }
        self.applied.lock().push((key.to_owned(), subkeys, first));
    }

    async fn on_event(&self, _event: TransportEvent) {}
}

fn change(key: &str, subkey: u32, data: Option<&[u8]>) -> ValueChange {
    ValueChange {
        key: key.to_owned(),
        subkeys: vec![subkey],
        count: 1,
        value: data.map(|d| RecordValue {
            seq: 0,
            data: d.to_vec(),
            writer: String::new(),
        }),
    }
}

/// A change whose read has not returned holds no one but the worker: the
/// dispatch loop queues the next ones at once, and they are applied after
/// it, in order, with their first values.
#[tokio::test]
async fn a_change_still_reading_holds_back_no_dispatch_and_order_holds() {
    let handler = Arc::new(Gated {
        permits: Semaphore::new(0),
        applied: Mutex::new(Vec::new()),
    });
    let (worker, run) = ValueChanges::new(Arc::clone(&handler));
    tokio::spawn(run);
    worker.queue(change("profile", 0, Some(b"p")));
    worker.queue(change("inbox", 2, None));
    worker.queue(change("profile", 6, None));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(handler.applied.lock().is_empty(), "the first read waits");
    handler.permits.add_permits(3);
    for _ in 0..100 {
        if handler.applied.lock().len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        *handler.applied.lock(),
        vec![
            ("profile".to_owned(), vec![0], Some(b"p".to_vec())),
            ("inbox".to_owned(), vec![2], None),
            ("profile".to_owned(), vec![6], None),
        ]
    );
}
