//! What the engine needs from its host: the storage of its rows, the send
//! attempt, the clock and randomness, and the delivery of a state change.
//! Implemented once, over the identity database, by
//! `rekindle-node/src/host/adapters/outbox/`.

use async_trait::async_trait;
use rekindle_types::message::SendState;
use rekindle_utils::random::RandomSource;
use rekindle_utils::time::Clock;

use crate::stream::StreamKind;

/// A storage failure.
#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    #[error("outbox storage: {0}")]
    Storage(String),
    /// The row's kind is the engine's to settle, never the user's to retry
    /// (a call media key, whose deadline is its session's end).
    #[error("a {} row is not retried by the user", .0.as_str())]
    NotRetryable(StreamKind),
}

/// A send intent to store at the end of its stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutboxRow {
    pub stream: String,
    pub client_id: String,
    pub kind: StreamKind,
    pub payload: Vec<u8>,
    pub next_at: i64,
    pub deadline: i64,
}

/// One stored send intent. Times are ms since the epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub stream: String,
    pub stream_seq: i64,
    pub client_id: String,
    pub kind: StreamKind,
    pub payload: Vec<u8>,
    pub state: SendState,
    pub attempts: u32,
    pub next_at: i64,
    pub deadline: i64,
    pub wedge_reason: Option<String>,
}

/// A row's state and schedule after the engine decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowUpdate {
    pub state: SendState,
    pub attempts: u32,
    pub next_at: i64,
    pub deadline: i64,
    pub wedge_reason: Option<String>,
}

/// How one send attempt went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// The frame left this device. A row of a kind that
    /// [`StreamKind::awaits_ack`] waits for the peer; any other settles
    /// `sent`.
    Sent,
    /// The frame left this device, and this row waits for the peer's
    /// acknowledgement of it whatever its kind (`Unfriended`, settled by
    /// `UnfriendedAck` or its deadline), re-sent meanwhile.
    Awaiting,
    /// The peer's own reply received the frame (an `app_call`'s receipt):
    /// the row settles `delivered`.
    Receipted,
    /// It could not be sent now (no route, no sender yet); try again.
    Deferred,
    /// It can never be sent as it stands (`prekey_rejected`): the row
    /// fails and wedges its stream until Retry or Discard.
    Wedged(String),
    /// It is no longer worth sending (a call media key a later rotation
    /// replaced, or whose recipient left): the row settles `failed` with
    /// the reason, unsent, and leaves its stream, so the rows behind it go
    /// on.
    Superseded(String),
}

/// A row's state as the user sees it, published on every change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChange {
    pub stream: String,
    pub stream_seq: i64,
    pub client_id: String,
    pub kind: StreamKind,
    pub state: SendState,
    pub wedge_reason: Option<String>,
}

impl StateChange {
    pub(crate) fn of(row: &OutboxRow, state: SendState, wedge_reason: Option<String>) -> Self {
        Self {
            stream: row.stream.clone(),
            stream_seq: row.stream_seq,
            client_id: row.client_id.clone(),
            kind: row.kind,
            state,
            wedge_reason,
        }
    }
}

/// The engine's host.
#[async_trait]
pub trait OutboxDeps: Send + Sync {
    /// The local echo written with a row (a direct message's `messages`
    /// row), in the same transaction.
    type Echo: Send + 'static;

    fn clock(&self) -> &dyn Clock;
    fn rng(&self) -> &dyn RandomSource;

    /// Store `row` (`queued`, no attempts) with `echo`; its `stream_seq`.
    async fn insert(&self, row: NewOutboxRow, echo: Self::Echo) -> Result<i64, OutboxError>;
    /// The unsettled rows due at `now` that no stream head blocks.
    async fn due(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError>;
    /// The unsettled rows of `stream`, in `stream_seq` order.
    async fn stream_rows(&self, stream: &str) -> Result<Vec<OutboxRow>, OutboxError>;
    /// The unsettled rows of every stream whose name starts with `prefix`
    /// (one call session's `media_key` streams), in stream and
    /// `stream_seq` order.
    async fn prefix_rows(&self, prefix: &str) -> Result<Vec<OutboxRow>, OutboxError>;
    /// The unsettled rows whose deadline is at or before `now`.
    async fn expired(&self, now: i64) -> Result<Vec<OutboxRow>, OutboxError>;
    /// The earliest due time or deadline of an unsettled row.
    async fn next_wake(&self) -> Result<Option<i64>, OutboxError>;
    async fn row(&self, client_id: &str) -> Result<Option<OutboxRow>, OutboxError>;
    async fn row_at(&self, stream: &str, stream_seq: i64)
        -> Result<Option<OutboxRow>, OutboxError>;
    /// Store `update` for `client_id` and project its state.
    async fn update(&self, client_id: &str, update: &RowUpdate) -> Result<(), OutboxError>;
    /// Project `state` and remove the row.
    async fn settle(&self, client_id: &str, state: SendState) -> Result<(), OutboxError>;
    /// The peer read `stream` (scope `scope`) through `up_to`: raise every
    /// echo at or below it to `read` and settle its rows still waiting, in
    /// one transaction; the entries raised, as `(stream_seq, client_id)`.
    async fn read_through(
        &self,
        stream: &str,
        scope: &str,
        up_to: i64,
    ) -> Result<Vec<(i64, String)>, OutboxError>;
    /// Record a received client id; `false` when it was already applied.
    async fn mark_received(&self, stream: &str, client_id: &str) -> Result<bool, OutboxError>;
    async fn unsettled_count(&self) -> Result<u64, OutboxError>;

    /// Send `row` once.
    async fn attempt(&self, row: &OutboxRow) -> AttemptOutcome;
    /// Make again the external write that carries `row`, a row of a kind
    /// the engine never attempts (the user's Retry of a failed channel
    /// post, under its client id); the write's settle settles the row.
    /// `Err(reason)` when it cannot be made.
    async fn reissue(&self, row: &OutboxRow) -> Result<(), String>;
    /// Deliver a state change to the user's windows and frontends.
    fn publish(&self, change: StateChange);
}
