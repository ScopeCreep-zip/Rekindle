//! The 1:1 typing, inbox, history, read-mark and outbox requests' `Debug`
//! (plans E2.3, E2.5).

use super::super::IpcRequest;

impl IpcRequest {
    /// The 1:1 message requests' `Debug` (`DmSend` is `fmt`'s own).
    pub(super) fn fmt_dm(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DmTyping { peer_key, typing } => f
                .debug_struct("DmTyping")
                .field("peer_key", peer_key)
                .field("typing", typing)
                .finish(),
            Self::DmInbox { limit } => f.debug_struct("DmInbox").field("limit", limit).finish(),
            Self::DmHistory {
                peer_key,
                limit,
                before,
            } => f
                .debug_struct("DmHistory")
                .field("peer_key", peer_key)
                .field("limit", limit)
                .field("before", before)
                .finish(),
            Self::DmMarkRead {
                peer_key,
                rendered_through,
            } => f
                .debug_struct("DmMarkRead")
                .field("peer_key", peer_key)
                .field("rendered_through", rendered_through)
                .finish(),
            Self::OutboxRetry { client_id } => f
                .debug_struct("OutboxRetry")
                .field("client_id", client_id)
                .finish(),
            Self::OutboxDiscard { client_id } => f
                .debug_struct("OutboxDiscard")
                .field("client_id", client_id)
                .finish(),
            other => unreachable!("{} is not a 1:1 message request", other.name()),
        }
    }
}
