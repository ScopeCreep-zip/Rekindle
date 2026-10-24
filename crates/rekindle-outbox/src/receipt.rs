//! Rows a later frame of the peer's proves arrived (live A/B finding 56):
//! their receipts were lost, but the peer's answer to them came.

use rekindle_types::message::SendState;

use crate::deps::{Cover, OutboxDeps, OutboxError, StateChange};
use crate::engine::Outbox;
use crate::state::{merge, PeerAck, StateInput};
use crate::stream::StreamKind;

impl<D: OutboxDeps> Outbox<D> {
    /// The peer's later frame proves every unsettled row of `stream` whose
    /// client id starts with `client_prefix` arrived (their `FriendAccept`
    /// answers our `FriendRequest`): each settles as its receipt would,
    /// `delivered`, and leaves its stream, so the rows behind it go.
    /// Returns how many settled.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn receipt_rows(
        &self,
        stream: &str,
        client_prefix: &str,
    ) -> Result<usize, OutboxError> {
        let rows = self.deps.stream_rows(stream).await?;
        let mut settled = 0;
        for row in rows
            .iter()
            .filter(|row| row.client_id.starts_with(client_prefix))
        {
            let state = merge(row.state, StateInput::Peer(PeerAck::Delivered));
            self.settle_at(row, state).await?;
            settled += 1;
        }
        Ok(settled)
    }

    /// The peer holds the entries `cover` names of our `kind` stream to
    /// `scope` (a cumulative `DmAck`, register 2026-10-24): each echo there
    /// becomes `delivered`, row or none, the landed rows settle and a row
    /// not landed keeps the ack. Returns how many entries changed.
    ///
    /// # Errors
    /// The storage failed; nothing changed.
    pub async fn deliver(
        &self,
        kind: StreamKind,
        scope: &str,
        cover: Cover,
    ) -> Result<usize, OutboxError> {
        let stream = kind.stream(scope);
        let raised = self.deps.deliver(&stream, scope, cover).await?;
        for (stream_seq, client_id) in &raised {
            self.deps.publish(StateChange {
                stream: stream.clone(),
                stream_seq: *stream_seq,
                client_id: client_id.clone(),
                kind,
                state: SendState::Delivered,
                wedge_reason: None,
            });
        }
        if !raised.is_empty() {
            self.wake();
        }
        Ok(raised.len())
    }
}
