use thiserror::Error;

#[derive(Debug, Error)]
pub enum GameDetectError {
    #[error("failed to enumerate processes: {0}")]
    ProcessEnumeration(String),

    #[error("game database error: {0}")]
    DatabaseError(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
