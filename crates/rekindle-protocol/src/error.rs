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

    /// An encoded envelope larger than Veilid's message cap
    /// ([`rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN`]), refused
    /// before Veilid sees it. Permanent: no retry or other path fits it.
    #[error("message of {len} bytes exceeds the {max}-byte Veilid message limit")]
    MessageTooLarge { len: usize, max: usize },

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
    /// ([`SetOutcome::missed`](rekindle_records::port::SetOutcome::missed)).
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

impl From<rekindle_records::port::PortError> for ProtocolError {
    fn from(e: rekindle_records::port::PortError) -> Self {
        use rekindle_records::port::PortError;
        match e {
            PortError::Dht(m) => Self::DhtError(m),
            PortError::Unreachable(m) => Self::DhtRecordUnreachable(m),
            PortError::Routing(m) => Self::RoutingError(m),
            PortError::Send(m) => Self::SendFailed(m),
            PortError::RouteUnusable(m) => Self::RouteUnusable(m),
            PortError::MessageTooLarge { len, max } => Self::MessageTooLarge { len, max },
            PortError::SubkeyTooLarge { subkey, len, cap } => {
                Self::SubkeyTooLarge { subkey, len, cap }
            }
            PortError::LeaseNotHeld(id) => Self::LeaseNotHeld(id),
            PortError::Closed => Self::PoolClosed,
            PortError::NotWritable(key) => Self::NotWritable(key),
            PortError::NotStored { subkey, outcome } => Self::NotStored { subkey, outcome },
            PortError::KeyText(m) => Self::DhtError(format!("bad key text: {m}")),
            PortError::Node(m) => Self::Internal(m),
            PortError::Codec(m) => Self::Deserialization(m),
            PortError::Crypto(m) => Self::CryptoError(m),
        }
    }
}

impl From<ProtocolError> for rekindle_records::port::PortError {
    fn from(e: ProtocolError) -> Self {
        match e {
            ProtocolError::DhtError(m) => Self::Dht(m),
            ProtocolError::DhtRecordUnreachable(m) => Self::Unreachable(m),
            ProtocolError::RoutingError(m) => Self::Routing(m),
            ProtocolError::SendFailed(m) | ProtocolError::ReceiveFailed(m) => Self::Send(m),
            ProtocolError::RouteUnusable(m) => Self::RouteUnusable(m),
            ProtocolError::MessageTooLarge { len, max } => Self::MessageTooLarge { len, max },
            ProtocolError::SubkeyTooLarge { subkey, len, cap } => {
                Self::SubkeyTooLarge { subkey, len, cap }
            }
            ProtocolError::LeaseNotHeld(id) => Self::LeaseNotHeld(id),
            ProtocolError::PoolClosed => Self::Closed,
            ProtocolError::NotWritable(key) => Self::NotWritable(key),
            ProtocolError::NotStored { subkey, outcome } => Self::NotStored { subkey, outcome },
            ProtocolError::PeerNotFound(m) => Self::Routing(format!("peer not found: {m}")),
            ProtocolError::NodeStartup(m)
            | ProtocolError::AttachFailed(m)
            | ProtocolError::Network(m)
            | ProtocolError::Internal(m) => Self::Node(m),
            ProtocolError::NodeNotInitialized => Self::Node("node not initialized".into()),
            ProtocolError::Serialization(m) => Self::Codec(format!("serialization: {m}")),
            ProtocolError::Deserialization(m) => Self::Codec(format!("deserialization: {m}")),
            ProtocolError::UnknownVariant(m) => Self::Codec(format!("unknown variant: {m}")),
            ProtocolError::Verification(m) => Self::Crypto(format!("verification: {m}")),
            ProtocolError::CryptoError(m) => Self::Crypto(m),
        }
    }
}

#[cfg(test)]
mod tests {
    use rekindle_records::port::PortError;

    use super::ProtocolError;

    /// A route-unusable failure keeps its class both ways: the importer
    /// must still forget the route.
    #[test]
    fn route_unusable_survives_the_port_round_trip() {
        let port = PortError::from(ProtocolError::RouteUnusable("no connection".into()));
        assert_eq!(port, PortError::RouteUnusable("no connection".into()));
        assert!(matches!(
            ProtocolError::from(port),
            ProtocolError::RouteUnusable(m) if m == "no connection"
        ));
    }
}
