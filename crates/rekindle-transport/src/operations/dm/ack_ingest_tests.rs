//! `ack::ingest` over an outbox double whose rows and echoes behave as the
//! Host's: a named seq settles its row or raises its echo once; `through`
//! raises every entry at or below it not delivered yet.

use std::collections::BTreeSet;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_codec::capnp_codec::direct::DmAck;
use rekindle_db::repo::outbox::{ListWrite, Row};

use super::{ingest, Ingested};
use crate::operations::dm::outbox::{DmOutbox, OutgoingDm};
use crate::operations::friend::FriendError;

/// Entries 1..=n sent, none delivered (each entry's chain seq is its
/// stream seq).
struct Rows(Mutex<BTreeSet<i64>>);

impl Rows {
    fn sent(n: i64) -> Self {
        Self(Mutex::new((1..=n).collect()))
    }
}

fn unused() -> FriendError {
    FriendError::Failed("not used by ingest".into())
}

#[async_trait]
impl DmOutbox for Rows {
    async fn enqueue(&self, _: &str, _: OutgoingDm) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_group(&self, _: &str, _: OutgoingDm) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_control(&self, _: &str, _: String, _: Vec<u8>) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_handshake(&self, _: &str, _: String, _: Vec<u8>) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_handshake_until(
        &self,
        _: &str,
        _: String,
        _: Vec<u8>,
        _: i64,
    ) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_removal(
        &self,
        _: &str,
        _: String,
        _: Vec<u8>,
        _: ListWrite,
    ) -> Result<Vec<Row>, FriendError> {
        Err(unused())
    }
    fn echo_failed(&self, _: Vec<Row>, _: &str) {}
    fn wake(&self) {}
    async fn bring_forward(&self, _: &str) -> Result<usize, FriendError> {
        Err(unused())
    }
    async fn acknowledge_client(&self, _: &str) -> Result<bool, FriendError> {
        Err(unused())
    }
    async fn discard(&self, _: &str) -> Result<bool, FriendError> {
        Err(unused())
    }
    async fn receipt_handshakes_to(&self, _: &str, _: &str) -> Result<usize, FriendError> {
        Err(unused())
    }
    async fn retry(&self, _: &str) -> Result<bool, FriendError> {
        Err(unused())
    }
    async fn acknowledge(&self, _: &str, stream_seq: i64) -> Result<bool, FriendError> {
        Ok(self.0.lock().remove(&stream_seq))
    }
    async fn acknowledge_chain_through(&self, _: &str, through: u64) -> Result<usize, FriendError> {
        let through = i64::try_from(through).unwrap_or(i64::MAX);
        let mut rows = self.0.lock();
        let before = rows.len();
        rows.retain(|seq| *seq > through);
        Ok(before - rows.len())
    }
    async fn read_up_to(&self, _: &str, _: i64) -> Result<usize, FriendError> {
        Err(unused())
    }
    async fn receive(&self, _: &str, _: &str) -> Result<bool, FriendError> {
        Err(unused())
    }
    async fn reask_soon(&self, _: &str) -> Result<usize, FriendError> {
        Err(unused())
    }
    async fn reconcile(&self) -> Result<usize, FriendError> {
        Err(unused())
    }
    fn appending(&self, _: &str) -> bool {
        false
    }
}

/// An ack naming 3 and 4 with `through` = 4 (E2.5i R2-3): the named seqs
/// settle by name and `through` raises the two below them; applied the
/// other way round, `through` took all four first and the log read
/// `settled=[] unmatched=[3, 4]`.
#[tokio::test]
async fn an_acks_named_seqs_settle_before_its_through() {
    let rows = Rows::sent(4);
    let ack = DmAck {
        stream_seqs: vec![3, 4],
        through: 4,
    };
    assert_eq!(
        ingest(&rows, "bob", &ack).await,
        Ingested {
            settled: vec![3, 4],
            unmatched: Vec::new(),
            through_raised: 2,
        }
    );
    // The same ack again (a re-ack of a duplicate) matches nothing.
    assert_eq!(
        ingest(&rows, "bob", &ack).await,
        Ingested {
            settled: Vec::new(),
            unmatched: vec![3, 4],
            through_raised: 0,
        }
    );
}
