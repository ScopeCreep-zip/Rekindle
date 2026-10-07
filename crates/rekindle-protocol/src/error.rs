use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("failed to start veilid node: {0}")]
    NodeStartup(String),

    #[error("failed to attach to network: {0}")]
    AttachFailed(String),

    #[error("node not initialized")]
    NodeNotInitialized,

    #[error("DHT operation failed: {0}")]
    DhtError(String),

    /// A DHT record open/get could not reach any node holding the record
    /// (veilid `KeyNotFound`/`TryAgain` — the outbound fanout returned no
    /// descriptor). On a freshly-attached node with a sparse routing table
    /// this is TRANSIENT and indistinguishable from "record genuinely
    /// absent", so callers must RETRY (gated on routing-table readiness)
    /// before concluding a record is gone and recreating it. Distinct from
    /// `DhtError` so the open-or-recreate sites don't churn record keys on a
    /// transient cold-start failure.
    #[error("DHT record unreachable (retryable): {0}")]
    DhtRecordUnreachable(String),

    #[error("routing error: {0}")]
    RoutingError(String),

    #[error("message send failed: {0}")]
    SendFailed(String),

    #[error("message receive failed: {0}")]
    ReceiveFailed(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// A Cap'n Proto union returned a discriminant the local schema
    /// doesn't know about. Distinct from `Deserialization` so callers
    /// can implement gossip-relay forward-compat (verify signature,
    /// decrement TTL, forward bytes intact, do not dispatch).
    #[error("unknown union variant: {0}")]
    UnknownVariant(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("verification failed: {0}")]
    Verification(String),

    #[error("peer not found: {0}")]
    PeerNotFound(String),

    #[error("crypto error: {0}")]
    CryptoError(String),

    #[error("internal error: {0}")]
    Internal(String),

    /// A value larger than the record's per-subkey limit
    /// (`min(32768, 1 MiB / subkey_count)`), refused before Veilid sees it.
    #[error("subkey {subkey} value of {len} bytes exceeds the record's {cap}-byte subkey limit")]
    SubkeyTooLarge { subkey: u32, len: usize, cap: usize },

    /// The lease was released (or never issued by this pool).
    #[error("record lease {0} is not held")]
    LeaseNotHeld(u64),

    /// The pool's session ended (logout): no new Veilid calls start.
    #[error("record pool is shut down")]
    PoolClosed,

    /// A write with no writer to a record the session does not hold
    /// writable: Veilid would refuse it ("value is not writable"), so the
    /// pool does not send it.
    #[error("record {0} is not held writable")]
    NotWritable(String),

    /// A send to a remote private route failed with `NoConnection` or
    /// `InvalidTarget`: the route is unusable, and the importer forgets it
    /// (`RouteImports::invalidate_after_send_failure`). Any other send
    /// failure is `SendFailed`, which keeps the route.
    #[error("route unusable: {0}")]
    RouteUnusable(String),

    /// A write that a structure depends on did not reach consensus (or lost
    /// to a newer value), so the structure was not updated
    /// ([`SetOutcome::missed`](crate::dht::pool::SetOutcome::missed)).
    #[error("subkey {subkey} not stored: {outcome}")]
    NotStored { subkey: u32, outcome: String },
}

impl From<rekindle_codec::CodecError> for ProtocolError {
    fn from(e: rekindle_codec::CodecError) -> Self {
        use rekindle_codec::CodecError;
        match e {
            CodecError::Serialization(m) => Self::Serialization(m),
            CodecError::Deserialization(m) => Self::Deserialization(m),
            CodecError::UnknownVariant(m) => Self::UnknownVariant(m),
            CodecError::Verification(m) => Self::Verification(m),
        }
    }
}

impl From<rekindle_crypto::CryptoError> for ProtocolError {
    fn from(e: rekindle_crypto::CryptoError) -> Self {
        Self::CryptoError(e.to_string())
    }
}
