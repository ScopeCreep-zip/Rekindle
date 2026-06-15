//! DM domain error type.

use rekindle_types::dm_store::DmStoreError;

#[derive(Debug, thiserror::Error)]
pub enum DmError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("invalid session state: {0}")]
    InvalidSessionState(String),

    #[error("identity not loaded")]
    IdentityNotLoaded,

    #[error("MEK chain unavailable: {0}")]
    MekChainUnavailable(String),

    #[error("encrypt failed: {0}")]
    EncryptFailed(String),

    #[error("decrypt failed: {0}")]
    DecryptFailed(String),

    #[error("envelope decode: {0}")]
    EnvelopeDecode(String),

    #[error("transport: {0}")]
    Transport(String),

    #[error("storage: {0}")]
    Storage(String),
}

impl From<DmStoreError> for DmError {
    fn from(e: DmStoreError) -> Self {
        match e {
            DmStoreError::NotFound(s) => DmError::SessionNotFound(s),
            DmStoreError::InvalidData(s) => DmError::InvalidInput(s),
            DmStoreError::Storage(s) => DmError::Storage(s),
        }
    }
}

impl From<DmError> for crate::ChatError {
    fn from(e: DmError) -> Self {
        match e {
            DmError::IdentityNotLoaded => crate::ChatError::IdentityNotLoaded,
            DmError::SessionNotFound(p) => crate::ChatError::NoSession { peer_key: p },
            DmError::MekChainUnavailable(s) => crate::ChatError::Internal(format!("dm mek chain: {s}")),
            DmError::InvalidInput(s) | DmError::InvalidSessionState(s) => {
                crate::ChatError::Internal(s)
            }
            DmError::EncryptFailed(s) | DmError::DecryptFailed(s) => {
                crate::ChatError::Internal(s)
            }
            DmError::EnvelopeDecode(s) => crate::ChatError::Deserialization(s),
            DmError::Transport(s) => crate::ChatError::Internal(format!("dm transport: {s}")),
            DmError::Storage(s) => crate::ChatError::Internal(format!("dm storage: {s}")),
        }
    }
}
