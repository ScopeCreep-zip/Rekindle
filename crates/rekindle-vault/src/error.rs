#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("AES-GCM error: {0}")]
    Aead(String),
    #[error("Argon2 KDF error: {0}")]
    Kdf(String),
    #[error("Vault schema error: {0}")]
    Schema(String),
    #[error("Vault I/O error: {0}")]
    Io(#[from] std::io::Error),
}
