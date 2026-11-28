//! The implicit acknowledgement of a first contact costs an outbox query
//! only while one to that peer is unsettled (plan HOL1, lead note).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_db::repo::outbox::{ListWrite, Row};

use super::{mark, settle_pending, waits_for};
use crate::operations::dm::outbox::{DmOutbox, OutgoingDm};
use crate::operations::friend::{FriendError, FriendStores};

/// An outbox that counts the queries `settle_pending` runs.
#[derive(Default)]
struct Counting(AtomicUsize);

fn unused() -> FriendError {
    FriendError::Failed("not used by settle_pending".into())
}

#[async_trait]
impl DmOutbox for Counting {
    async fn enqueue(&self, _: &str, _: OutgoingDm) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_group(&self, _: &str, _: OutgoingDm) -> Result<i64, FriendError> {
        Err(unused())
    }
    async fn enqueue_group_control(
        &self,
        _: &str,
        _: String,
        _: Vec<u8>,
    ) -> Result<i64, FriendError> {
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
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(1)
    }
    async fn retry(&self, _: &str) -> Result<bool, FriendError> {
        Err(unused())
    }
    async fn acknowledge(&self, _: &str, _: i64) -> Result<bool, FriendError> {
        Err(unused())
    }
    async fn acknowledge_chain_through(&self, _: &str, _: u64) -> Result<usize, FriendError> {
        Err(unused())
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

/// A frame from a peer with no first contact pending runs no outbox
/// query; one with a first contact committed settles it (one query per
/// first-contact kind) and clears the note, so the next frame runs none.
#[tokio::test]
async fn a_frame_with_nothing_pending_runs_no_outbox_query() {
    let stores = FriendStores::default();
    let counting = Arc::new(Counting::default());
    let outbox = || Ok(Arc::clone(&counting) as Arc<dyn DmOutbox>);

    settle_pending(&stores, outbox, "peer").await;
    assert_eq!(counting.0.load(Ordering::SeqCst), 0, "nothing pending");

    // A presence key is not a first contact: still nothing pending.
    mark(&stores, "peer", "FriendsKey:00");
    settle_pending(&stores, outbox, "peer").await;
    assert_eq!(counting.0.load(Ordering::SeqCst), 0, "not a first contact");

    mark(&stores, "peer", "FriendAccept:00");
    settle_pending(&stores, outbox, "other").await;
    assert_eq!(counting.0.load(Ordering::SeqCst), 0, "another peer's frame");
    settle_pending(&stores, outbox, "peer").await;
    assert_eq!(counting.0.load(Ordering::SeqCst), 3, "settled once");
    settle_pending(&stores, outbox, "peer").await;
    assert_eq!(
        counting.0.load(Ordering::SeqCst),
        3,
        "settled: no query again"
    );
}

/// HOL1g: a first contact's row is committed as `PreKeyInit:` (the
/// frame's own kind, `handshake::enqueue_frame`). It is noted as our
/// unsettled first contact, waits for nothing itself, and a friend-gated
/// frame (a presence key) waits for it.
#[tokio::test]
async fn a_prekey_init_row_is_the_first_contact_a_presence_key_waits_for() {
    let stores = FriendStores::default();
    mark(&stores, "peer", "PreKeyInit:00");
    assert!(stores.unsettled_first_contacts.lock().contains("peer"));
    assert!(waits_for("PreKeyInit:00").is_empty());
    assert!(waits_for("FriendsKey:00").contains(&"PreKeyInit:".to_owned()));
}
