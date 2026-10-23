//! Rows a later frame of the peer's proves arrived (live A/B finding 56):
//! their receipts were lost, but the peer's answer to them came.

use crate::deps::{OutboxDeps, OutboxError};
use crate::engine::Outbox;
use crate::state::{merge, PeerAck, StateInput};

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
}
