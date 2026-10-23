//! The unlocked identity's outbox (plan E2.5, D22) on both hosts: built at
//! unlock over [`HostOutbox`], run in the unlock scope with the `dm` and
//! `friend_handshake` senders installed, a `dm` row's state changes
//! published as `DmSendStatus`, a settled `Unfriended` row ending its
//! removal (the friend is forgotten), and stopped at the teardown. The 1:1 operations reach it through
//! `FriendHost::outbox` as a [`DmOutbox`].

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_lifecycle::SessionScope;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxRow, PeerAck, StateChange, StreamKind,
};
use rekindle_transport::operations::dm::handshake;
use rekindle_transport::operations::dm::outbox::{
    self as dm_outbox, Attempt, DmOutbox, OutgoingDm,
};
use rekindle_transport::operations::friend::{self, FriendError, FriendHost};
use rekindle_types::subscription_events::{ChannelMessageEvent, GroupDmEvent, SubscriptionEvent};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::{DmEcho, HostOutbox, Senders, StreamSender};
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::friends::HostFriends;

/// The outbox of the unlocked identity, while one is.
#[derive(Default)]
pub struct OutboxSlot {
    pub(super) current: RwLock<Option<Arc<Outbox<HostOutbox>>>>,
    pub(super) senders: Arc<Senders>,
}

impl OutboxSlot {
    /// Build the outbox for `owner`, install the `dm` sender and run the
    /// engine and its state publisher in `scope`.
    ///
    /// # Errors
    /// The scope is closing.
    pub(crate) fn start(
        &self,
        ctx: &DaemonContext,
        scope: &Arc<SessionScope>,
        owner: &str,
    ) -> Result<(), String> {
        let host = HostOutbox::new(
            Arc::clone(&ctx.db),
            owner.to_owned(),
            Arc::clone(&ctx.clock),
            Arc::clone(&ctx.rng),
            Arc::clone(&self.senders),
        );
        let changes = host.subscribe();
        let outbox = Arc::new(Outbox::new(host));
        self.senders
            .install(StreamKind::Dm, Arc::new(DmSender(HostFriends::of(ctx))));
        self.senders.install(
            StreamKind::FriendHandshake,
            Arc::new(HandshakeSender(HostFriends::of(ctx))),
        );
        self.senders.install(
            StreamKind::GroupDm,
            Arc::new(GroupSender(HostFriends::of(ctx))),
        );
        self.install_governance_op_sender(crate::host::adapters::governance::HostGovernance::new(
            ctx.this.upgrade().expect("a context in use is alive"),
        ));
        *self.current.write() = Some(Arc::clone(&outbox));
        let friends = HostFriends::of(ctx);
        scope
            .spawn_with_token("outbox", move |stop| async move { outbox.run(stop).await })
            .and_then(|()| {
                scope.spawn_with_token("outbox states", move |stop| {
                    publish_states(friends, changes, stop)
                })
            })
            .map_err(|e| e.to_string())
    }

    /// Drop the outbox and its senders (teardown); its rows stay stored.
    pub(crate) fn stop(&self) {
        self.senders.remove(StreamKind::Dm);
        self.senders.remove(StreamKind::FriendHandshake);
        self.senders.remove(StreamKind::GroupDm);
        self.senders.remove(StreamKind::DhtWrite);
        self.senders.remove(StreamKind::MediaKey);
        self.senders.remove(StreamKind::ChannelPost);
        self.current.write().take();
    }

    /// The running outbox as the 1:1 operations use it.
    pub(crate) fn dm(&self) -> Option<Arc<dyn DmOutbox>> {
        let outbox = self.current.read().clone()?;
        Some(Arc::new(DmStream(outbox)))
    }
}

/// Every state change of a `dm` row, as the one send-state event; a
/// settled `Unfriended` row forgets the removed friend. A removal whose
/// row settled before a restart ends at the first pass.
async fn publish_states(
    friends: HostFriends,
    mut changes: broadcast::Receiver<StateChange>,
    stop: CancellationToken,
) {
    forget_removed(&friends).await;
    loop {
        let change = tokio::select! {
            () = stop.cancelled() => return,
            change = changes.recv() => change,
        };
        match change {
            Ok(change) => {
                if let Some(event) = send_status(&change) {
                    friends.publish(event);
                }
                if ends_removal(&change) {
                    forget_removed(&friends).await;
                }
            }
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "outbox state changes missed; the rows hold them");
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Whether `change` settles an `Unfriended` row (acknowledged, or failed
/// at its deadline): the removal it carried ends.
fn ends_removal(change: &StateChange) -> bool {
    change.kind == StreamKind::FriendHandshake
        && handshake::unfriended_peer(&change.client_id).is_some()
        && !rekindle_outbox::state::is_unsettled(change.state)
}

async fn forget_removed(friends: &HostFriends) {
    if let Err(e) = friend::forget_removed(friends).await {
        tracing::warn!(error = %e, "forgetting removed friends failed");
    }
}

/// The `GroupDmEvent::SendStatus` of a `group_dm` row's change, the
/// `DmSendStatus` of a `dm` row's change, or of our friend request's
/// (its `FriendRequest:` client id names it on the peer's pending row).
fn send_status(change: &StateChange) -> Option<SubscriptionEvent> {
    if change.kind == StreamKind::GroupDm {
        return Some(SubscriptionEvent::GroupDm(GroupDmEvent::SendStatus {
            group_id: change.stream.strip_prefix("group_dm:")?.to_owned(),
            client_id: change.client_id.clone(),
            state: change.state,
        }));
    }
    let peer = match change.kind {
        // A control row (`StreamRecord`) has no message to show a state on.
        StreamKind::Dm if change.client_id.starts_with("StreamRecord:") => return None,
        StreamKind::Dm => change.stream.strip_prefix("dm:")?,
        StreamKind::FriendHandshake if change.client_id.starts_with("FriendRequest:") => {
            change.stream.strip_prefix("friend_handshake:")?
        }
        _ => return None,
    };
    Some(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DmSendStatus {
            conversation_id: peer.to_owned(),
            client_id: change.client_id.clone(),
            state: change.state,
        },
    ))
}

/// The `dm` stream's sender: `dm::outbox::attempt` over the Host.
struct DmSender(HostFriends);

#[async_trait]
impl StreamSender for DmSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let mut outcomes = self.send_stream(std::slice::from_ref(row)).await;
        outcomes.pop().unwrap_or(AttemptOutcome::Deferred)
    }

    /// The stream's due rows go on the chain together (plan E2.B F2).
    async fn send_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        let Some(peer) = rows.first().and_then(|r| r.stream.strip_prefix("dm:")) else {
            return rows
                .iter()
                .map(|r| AttemptOutcome::Wedged(format!("not a dm stream: {}", r.stream)))
                .collect();
        };
        dm_outbox::attempt(&self.0, peer, &row_attempts(rows))
            .await
            .into_iter()
            .map(outcome)
            .collect()
    }
}

/// The `group_dm` stream's sender: `group_dm::content::attempt` (plan
/// E2.6): a row settles `sent` once its entry is on our group chain, and
/// the stream's due rows go on it together (plan E2.B F2).
struct GroupSender(HostFriends);

#[async_trait]
impl StreamSender for GroupSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let mut outcomes = self.send_stream(std::slice::from_ref(row)).await;
        outcomes.pop().unwrap_or(AttemptOutcome::Deferred)
    }

    async fn send_stream(&self, rows: &[OutboxRow]) -> Vec<AttemptOutcome> {
        let Some(group) = rows
            .first()
            .and_then(|r| r.stream.strip_prefix("group_dm:"))
        else {
            return rows
                .iter()
                .map(|r| AttemptOutcome::Wedged(format!("not a group stream: {}", r.stream)))
                .collect();
        };
        rekindle_transport::operations::group_dm::content::attempt(
            &self.0,
            group,
            &row_attempts(rows),
        )
        .await
        .into_iter()
        .map(outcome)
        .collect()
    }
}

/// `rows` as the chain senders take them.
fn row_attempts(rows: &[OutboxRow]) -> Vec<dm_outbox::RowAttempt<'_>> {
    rows.iter()
        .map(|r| dm_outbox::RowAttempt {
            stream_seq: r.stream_seq,
            sent_at: r.sent_at,
            payload: &r.payload,
        })
        .collect()
}

/// The `friend_handshake` stream's sender: `dm::handshake::attempt`.
struct HandshakeSender(HostFriends);

#[async_trait]
impl StreamSender for HandshakeSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let Some(peer) = row.stream.strip_prefix("friend_handshake:") else {
            return AttemptOutcome::Wedged(format!("not a handshake stream: {}", row.stream));
        };
        outcome(handshake::attempt(&self.0, peer, &row.payload).await)
    }
}

fn outcome(attempt: Attempt) -> AttemptOutcome {
    match attempt {
        Attempt::Sent => AttemptOutcome::Sent,
        Attempt::Awaiting => AttemptOutcome::Awaiting,
        Attempt::Receipted => AttemptOutcome::Receipted,
        Attempt::Deferred => AttemptOutcome::Deferred,
        Attempt::Wedged(reason) => AttemptOutcome::Wedged(reason),
    }
}

/// The outbox as a [`DmOutbox`].
struct DmStream(Arc<Outbox<HostOutbox>>);

fn handshake_entry(to: &str, client_id: String, payload: Vec<u8>) -> Entry {
    Entry {
        kind: StreamKind::FriendHandshake,
        scope: to.to_owned(),
        client_id,
        payload,
        deadline: Deadline::Horizon,
    }
}

fn failed(e: &rekindle_outbox::OutboxError) -> FriendError {
    FriendError::Failed(e.to_string())
}

#[async_trait]
impl DmOutbox for DmStream {
    async fn enqueue(&self, to: &str, dm: OutgoingDm) -> Result<i64, FriendError> {
        let entry = Entry {
            kind: StreamKind::Dm,
            scope: to.to_owned(),
            client_id: dm.client_id,
            payload: dm.payload,
            deadline: Deadline::Horizon,
        };
        let echo = DmEcho {
            peer_key: to.to_owned(),
            conversation_type: rekindle_types::message::ConversationType::Dm,
            sender_key: dm.sender,
            body: dm.body,
            timestamp: dm.timestamp,
            reply_to: dm.reply_to,
        };
        let enqueued = self.0.enqueue(entry, Some(echo)).await;
        enqueued.map(|e| e.stream_seq).map_err(|e| failed(&e))
    }

    async fn enqueue_group(&self, group_id: &str, dm: OutgoingDm) -> Result<i64, FriendError> {
        let entry = Entry {
            kind: StreamKind::GroupDm,
            scope: group_id.to_owned(),
            client_id: dm.client_id,
            payload: dm.payload,
            deadline: Deadline::Horizon,
        };
        let echo = DmEcho {
            peer_key: group_id.to_owned(),
            conversation_type: rekindle_types::message::ConversationType::GroupDm,
            sender_key: dm.sender,
            body: dm.body,
            timestamp: dm.timestamp,
            reply_to: dm.reply_to,
        };
        let enqueued = self.0.enqueue(entry, Some(echo)).await;
        enqueued.map(|e| e.stream_seq).map_err(|e| failed(&e))
    }

    async fn enqueue_control(
        &self,
        to: &str,
        client_id: String,
        payload: Vec<u8>,
    ) -> Result<i64, FriendError> {
        let entry = Entry {
            kind: StreamKind::Dm,
            scope: to.to_owned(),
            client_id,
            payload,
            deadline: Deadline::Horizon,
        };
        let enqueued = self.0.enqueue(entry, None).await;
        enqueued.map(|e| e.stream_seq).map_err(|e| failed(&e))
    }

    async fn enqueue_handshake(
        &self,
        to: &str,
        client_id: String,
        payload: Vec<u8>,
    ) -> Result<i64, FriendError> {
        let entry = handshake_entry(to, client_id, payload);
        let enqueued = self.0.enqueue(entry, None).await;
        enqueued.map(|e| e.stream_seq).map_err(|e| failed(&e))
    }

    async fn enqueue_removal(
        &self,
        to: &str,
        client_id: String,
        payload: Vec<u8>,
    ) -> Result<(), FriendError> {
        let row = self.0.new_row(handshake_entry(to, client_id, payload));
        self.0
            .deps()
            .mark_removing(to, row)
            .await
            .map_err(|e| failed(&e))?;
        self.0.wake();
        Ok(())
    }

    async fn acknowledge_client(&self, client_id: &str) -> Result<bool, FriendError> {
        let acked = self
            .0
            .acknowledge_client(client_id, PeerAck::Delivered)
            .await;
        acked.map_err(|e| failed(&e))
    }

    async fn discard(&self, client_id: &str) -> Result<bool, FriendError> {
        self.0.discard(client_id).await.map_err(|e| failed(&e))
    }

    async fn retry(&self, client_id: &str) -> Result<bool, FriendError> {
        self.0.retry(client_id).await.map_err(|e| failed(&e))
    }

    async fn acknowledge(&self, from: &str, stream_seq: i64) -> Result<bool, FriendError> {
        let stream = StreamKind::Dm.stream(from);
        let acked = self
            .0
            .acknowledge(&stream, stream_seq, PeerAck::Delivered)
            .await;
        acked.map_err(|e| failed(&e))
    }

    async fn read_up_to(&self, from: &str, stream_seq: i64) -> Result<usize, FriendError> {
        let read = self.0.read_up_to(StreamKind::Dm, from, stream_seq).await;
        read.map_err(|e| failed(&e))
    }

    async fn receive(&self, from: &str, client_id: &str) -> Result<bool, FriendError> {
        let stream = StreamKind::Dm.stream(from);
        self.0
            .receive(&stream, client_id)
            .await
            .map_err(|e| failed(&e))
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::message::SendState;

    use super::*;

    #[test]
    fn only_dm_rows_publish_a_send_status() {
        let change = StateChange {
            stream: "dm:bob".into(),
            stream_seq: 1,
            client_id: "c1".into(),
            kind: StreamKind::Dm,
            state: SendState::Delivered,
            wedge_reason: None,
        };
        let Some(SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
            conversation_id,
            client_id,
            state,
        })) = send_status(&change)
        else {
            panic!("a dm row publishes DmSendStatus");
        };
        assert_eq!(
            (conversation_id.as_str(), client_id.as_str(), state),
            ("bob", "c1", SendState::Delivered)
        );
        let other = StateChange {
            kind: StreamKind::DhtWrite,
            stream: "dht_write:r".into(),
            ..change
        };
        assert!(send_status(&other).is_none());
    }
}
