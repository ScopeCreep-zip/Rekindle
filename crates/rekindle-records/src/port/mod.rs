//! The Veilid-free port (plan C9.1, D28): the record, route and app-message
//! operations the transport needs, with keys, keypairs and route ids as the
//! text Veilid prints them in, values as [`RecordValue`], and schemas as
//! [`RecordSchema`]. `rekindle-protocol` implements them over Veilid;
//! `rekindle-netsim` implements them in memory, so the real host and its
//! engines run in a seeded, deterministic simulation.

mod dht;
mod message;
mod record;
mod route;

pub use dht::{DhtBackend, DhtCallError, DhtErrorKind, RecordDescriptor, WriterKey};
pub use message::{check_message_len, AppCallId, AppMessagePort};
pub use record::{
    RecordPort, RecordReport, RecordSchema, RecordStore, RecordValue, ReportScope, SchemaMember,
    SetOutcome, Settled, UnsentWrite, ValueChange, ValueChangeReport,
};
pub use route::{PeerTarget, RouteClass, RoutePort, RouteState};

use std::sync::Arc;

/// A port operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    #[error("DHT operation failed: {0}")]
    Dht(String),
    #[error("DHT record unreachable (retryable): {0}")]
    Unreachable(String),
    #[error("routing error: {0}")]
    Routing(String),
    #[error("message send failed: {0}")]
    Send(String),
    /// The target route has no connection or is invalid: forget it and
    /// re-import the peer's current blob.
    #[error("route unusable: {0}")]
    RouteUnusable(String),
    #[error("message of {len} bytes exceeds the {max}-byte message limit")]
    MessageTooLarge { len: usize, max: usize },
    #[error("subkey {subkey} value of {len} bytes exceeds the record's {cap}-byte subkey limit")]
    SubkeyTooLarge { subkey: u32, len: usize, cap: usize },
    #[error("record lease {0} is not held")]
    LeaseNotHeld(u64),
    #[error("record pool is shut down")]
    Closed,
    #[error("record {0} is not held writable")]
    NotWritable(String),
    #[error("subkey {subkey} was not stored: {outcome}")]
    NotStored { subkey: u32, outcome: String },
    /// A record key, keypair or route id text that does not parse.
    #[error("bad key text: {0}")]
    KeyText(String),
    /// The node is not started, not attached, or failed internally.
    #[error("node: {0}")]
    Node(String),
    #[error("encoding: {0}")]
    Codec(String),
    #[error("crypto: {0}")]
    Crypto(String),
}

/// What happened on the network, as the transport dispatches it.
#[derive(Debug, Clone)]
pub enum PortEvent {
    /// A fire-and-forget message.
    AppMessage { message: Vec<u8> },
    /// A request awaiting [`AppMessagePort::reply`].
    AppCall {
        call_id: AppCallId,
        message: Vec<u8>,
    },
    /// A watched record changed.
    ValueChange(ValueChange),
    /// Routes died: ours (`dead`) and peers' we imported (`dead_remote`),
    /// by id text.
    RouteChange {
        dead: Vec<String>,
        dead_remote: Vec<String>,
    },
    /// The attachment changed, with the network's health.
    Attachment {
        /// The attachment state as the network names it.
        state: String,
        is_attached: bool,
        public_internet_ready: bool,
        reliable_peers: u64,
        live_peers: u64,
        network_size: u64,
        /// Median peer latency in microseconds (0 when unknown).
        median_latency_us: u64,
    },
    /// The node is shutting down.
    Shutdown,
}

/// A node this process started, for its owner to stop.
#[async_trait::async_trait]
pub trait NodeLifecycle: Send + Sync {
    /// Leave the network.
    ///
    /// # Errors
    /// The node refused to detach.
    async fn detach(&self) -> Result<(), PortError>;
    /// Stop the node.
    async fn shutdown(&self);
}

/// The ports a transport node runs on.
#[derive(Clone)]
pub struct PortSet {
    pub records: Arc<dyn RecordStore>,
    pub routes: Arc<dyn RoutePort>,
    pub messages: Arc<dyn AppMessagePort>,
    /// `Some` only when this node started the network itself; an adopted or
    /// simulated node is stopped by its owner.
    pub lifecycle: Option<Arc<dyn NodeLifecycle>>,
}
