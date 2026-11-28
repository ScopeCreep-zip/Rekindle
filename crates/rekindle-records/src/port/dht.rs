//! The DHT backend the record pool runs on (plan E2.B, seam design
//! 2026-10-21): the `RoutingContext` calls the pool makes, Veilid-free.
//!
//! Keys and keypairs are the text Veilid prints them in, values are
//! [`RecordValue`], schemas [`RecordSchema`], inspects [`RecordReport`].
//! `rekindle-protocol` implements it over Veilid (`dht::backend::VeilidDht`);
//! the netsim implements it over a simulated storage manager. Value changes
//! do not come through here: they keep arriving through
//! [`RecordPort::on_value_change`](super::RecordPort::on_value_change).

use rekindle_secrets::record_writer::RecordWriterSecret;

use super::{RecordReport, RecordSchema, RecordValue, ReportScope};

/// A writer keypair, with its public key (the writer a stored value names).
///
/// The credential is private and the type has no display conversion.
///
/// ```compile_fail
/// use rekindle_records::port::WriterKey;
/// fn plain_copy(writer: &WriterKey) -> String {
///     writer.keypair.clone()
/// }
/// ```
///
/// ```compile_fail
/// use rekindle_records::port::WriterKey;
/// fn display(writer: &WriterKey) -> String {
///     writer.to_string()
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterKey {
    keypair: RecordWriterSecret,
    /// The public key text, as [`RecordValue::writer`] carries it.
    pub public: String,
}

impl WriterKey {
    #[must_use]
    pub fn new(public: String, keypair: RecordWriterSecret) -> Self {
        Self { keypair, public }
    }

    /// Borrow the protected credential for the DHT backend.
    #[must_use]
    pub fn keypair(&self) -> &RecordWriterSecret {
        &self.keypair
    }

    /// Transfer a newly created record's owner to its caller.
    #[must_use]
    pub fn into_keypair(self) -> RecordWriterSecret {
        self.keypair
    }
}

/// What an open or a create returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDescriptor {
    /// The record key text.
    pub key: String,
    /// The owner keypair, when the backend holds the owner's secret (always
    /// after a create).
    pub owner: Option<WriterKey>,
    /// The owner's public key text, which the record key hashes and the
    /// descriptor's signature is made with
    /// (veilid-core 0.5.8 `storage_manager/create_record.rs:95-113`,
    /// `veilid_api/types/dht/derivation.rs:57-83`): a reader that compares it with the key it expects
    /// pins the record to that owner.
    pub owner_public: String,
    /// The schema's subkey count.
    pub subkey_count: u32,
}

/// Why a record is opened (plan VU decision 1, ADR 0021 amendment): the
/// role decides whether a re-open of a record this node already stores
/// asks Veilid to check that the network still carries it.
///
/// Veilid 0.5.8 queues that rehydration on a re-open only when the open
/// asks for it (`OpenDHTRecordOptions::rehydrate`,
/// `veilid_api/types/dht/open_dht_record_options.rs:6-10`), the record was
/// not created by this node this session (`locally_created_records`,
/// `storage_manager/open_record.rs:62-68`, filled by `create_record.rs`),
/// and the last rehydration of it finished at least 60 RPC timeouts ago
/// (300 s at the 5 s default; `storage_manager/mod.rs:178-198`,
/// `open_record.rs:69-80`). A first open of a record not stored locally
/// never rehydrates: it inspects for the descriptor and answers `TryAgain`
/// when no node answered (`open_record.rs:83-111`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenIntent {
    /// Custody: our own records, the keepalive's warming, and the long-lived
    /// leases a watch or a sweep holds. A re-open asks for rehydration.
    Hold,
    /// A check: a one-shot read, inspect or write, or an existence probe.
    /// A re-open never asks for rehydration, so a probe announces no
    /// interest to storage nodes.
    Check,
}

impl OpenIntent {
    /// Whether an open with this intent asks Veilid to rehydrate.
    #[must_use]
    pub fn rehydrates(self) -> bool {
        matches!(self, Self::Hold)
    }
}

/// The kind of a DHT failure, one for one with veilid-core's
/// `VeilidAPIError` (0.5.8 `veilid_api/types/error.rs:132-220`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhtErrorKind {
    NotInitialized,
    AlreadyInitialized,
    Timeout,
    TryAgain,
    InvalidTarget,
    NoConnection,
    Shutdown,
    KeyNotFound,
    Internal,
    Unimplemented,
    ParseError,
    InvalidArgument,
    MissingArgument,
    Generic,
    TransactionNotFound,
}

/// A DHT failure: its kind and the backend's text for it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct DhtCallError {
    pub kind: DhtErrorKind,
    pub message: String,
}

impl DhtCallError {
    /// The backend is shutting down, or the pool released the wait.
    #[must_use]
    pub fn shutdown() -> Self {
        Self {
            kind: DhtErrorKind::Shutdown,
            message: "Shutdown".into(),
        }
    }
}

/// The DHT calls of one routing context (one safety selection).
#[async_trait::async_trait]
pub trait DhtBackend: Send + Sync {
    /// A writer from its keypair text.
    ///
    /// # Errors
    /// The text is not a keypair.
    fn parse_writer(&self, text: &str) -> Result<WriterKey, DhtCallError>;

    /// The canonical text of a record key.
    ///
    /// # Errors
    /// The text is not a record key.
    fn check_key(&self, key: &str) -> Result<String, DhtCallError>;

    /// Create a record owned by `owner` (a fresh keypair when `None`).
    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError>;

    /// Open a record, writable with `writer` when given; `intent` says
    /// whether a re-open of a locally stored record asks for rehydration
    /// ([`OpenIntent`]).
    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
        intent: OpenIntent,
    ) -> Result<RecordDescriptor, DhtCallError>;

    /// Close a record (node-wide; it cancels the record's watch).
    async fn close(&self, key: &str) -> Result<(), DhtCallError>;

    /// Read a subkey; `force_refresh` asks the network.
    async fn get(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, DhtCallError>;

    /// Read a subkey from the newest value any node of a transaction's set
    /// holds, past a get's first agreeing set: a read-only DHT transaction
    /// (begin, get, rollback). Veilid's begin asks `set_value_count`
    /// accepters plus a trim among the closest nodes and keeps each one's
    /// seqs (0.5.8 `storage_manager/transaction.rs:177-181`,
    /// `transaction_begin.rs:338-347,398-402`); its get asks only the nodes
    /// whose begin seq is the newest
    /// (`outbound_transaction_manager/mod.rs:1947-1966`) and stores nothing
    /// locally (`transaction.rs:1541-1582`). A reader signs with an
    /// anonymous key (`transaction.rs:212-224`). A get stops at
    /// `get_value_count` holders of the first seq that reaches them
    /// (`get_value.rs:49-88`), and an inspect returns only seqs
    /// (`veilid_api/types/dht/dht_record_report.rs:9-20`), so neither
    /// fetches a newer value only a minority of the closest nodes holds.
    async fn get_newest(&self, key: &str, subkey: u32)
        -> Result<Option<RecordValue>, DhtCallError>;

    /// Write a subkey as `writer` (the open's writer when `None`), never
    /// offline: `Some` is the value the write settled on, returned when it
    /// differs from ours or when some node holds a rival at its seq, so it
    /// can be ours (Veilid 0.5.8 `set_value.rs:753-760`). The pool tells
    /// the two apart ([`SetOutcome`]). `None` is ours at consensus, or a
    /// fanout that ended below it and stored nothing (fork `e1158844`
    /// `set_value.rs:731-738` → `local_record_store_interface.rs:61-62`, then
    /// `set_value.rs:306-308`): only a landing writes the local copy
    /// (`set_value.rs:741-746`). [`DhtErrorKind::TryAgain`] is offline before
    /// the fanout, an outbound error, or a miss at the max seq.
    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError>;

    /// [`set`](Self::set) at a seq of at least `min_seq`: Veilid's
    /// `SetDHTValueOptions::min_seqnum` (0.5.8 `set_value.rs:353-386`). The
    /// same data and writer as the local copy are re-encrypted at `min_seq`
    /// when it is above the local seq, not re-sent at the old seq. The pool
    /// writes above a seq where its value settled contested with it.
    async fn set_above(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
        min_seq: u32,
    ) -> Result<Option<RecordValue>, DhtCallError>;

    /// Inspect `subkeys` (every subkey when `None`) in `scope`.
    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError>;

    /// Watch `subkeys` (every subkey when `None`); whether the watch state
    /// was accepted locally.
    async fn watch(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        expiration_us: Option<u64>,
        count: Option<u32>,
    ) -> Result<bool, DhtCallError>;

    /// Cancel the watch on `subkeys` (all when `None`); whether a watch
    /// remains.
    async fn cancel_watch(&self, key: &str, subkeys: Option<&[u32]>) -> Result<bool, DhtCallError>;
}

#[cfg(test)]
#[path = "dht_writer_tests.rs"]
mod writer_tests;
