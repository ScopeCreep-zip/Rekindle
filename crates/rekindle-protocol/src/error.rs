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
}

impl From<rekindle_crypto::CryptoError> for ProtocolError {
    fn from(e: rekindle_crypto::CryptoError) -> Self {
        Self::CryptoError(e.to_string())
    }
}
