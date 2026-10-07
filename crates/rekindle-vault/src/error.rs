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
    /// The passphrase does not decrypt this vault, or the file is corrupt.
    /// SQLCipher reports a page that will not decrypt as `SQLITE_NOMEM`
    /// ("out of memory") or `SQLITE_NOTADB` once the codec errs
    /// (`sqlite3Codec` returns NULL; the pager's `CODEC1`/`CODEC2` map that
    /// to NOMEM), so the keying phase of `open` reports it as this.
    #[error("wrong passphrase or corrupt vault")]
    WrongPassphrase,
}
