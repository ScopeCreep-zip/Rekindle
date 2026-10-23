//! The DHT backend the record pool runs on (plan E2.B, seam design
//! 2026-10-21): the `RoutingContext` calls the pool makes, Veilid-free.
//!
//! Keys and keypairs are the text Veilid prints them in, values are
//! [`RecordValue`], schemas [`RecordSchema`], inspects [`RecordReport`].
//! `rekindle-protocol` implements it over Veilid (`dht::backend::VeilidDht`);
//! the netsim implements it over a simulated storage manager. Value changes
//! do not come through here: they keep arriving through
//! [`RecordPort::on_value_change`](super::RecordPort::on_value_change).

use super::{RecordReport, RecordSchema, RecordValue, ReportScope};

/// A writer keypair, with its public key (the writer a stored value names).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterKey {
    /// The keypair text.
    pub keypair: String,
    /// The public key text, as [`RecordValue::writer`] carries it.
    pub public: String,
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
    /// (`storage_manager/create_record.rs:88-104`, `record_key.rs:67-94`,
    /// fork 0cd847de): a reader that compares it with the key it expects
    /// pins the record to that owner.
    pub owner_public: String,
    /// The schema's subkey count.
    pub subkey_count: u32,
}

/// The kind of a DHT failure, one for one with veilid-core's
/// `VeilidAPIError` (`veilid_api/error.rs:131-215`).
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

    /// Open a record, writable with `writer` when given.
    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
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

    /// Write a subkey as `writer` (the open's writer when `None`), never
    /// offline: `Some` is a newer value the network holds.
    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
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
