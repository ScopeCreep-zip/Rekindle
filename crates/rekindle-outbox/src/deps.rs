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
    /// The frame left this device.
    Sent,
    /// It could not be sent now (no route, no sender yet); try again.
    Deferred,
    /// It can never be sent as it stands (`prekey_rejected`): the row
    /// fails and wedges its stream until Retry or Discard.
    Wedged(String),
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
    /// Deliver a state change to the user's windows and frontends.
    fn publish(&self, change: StateChange);
}
