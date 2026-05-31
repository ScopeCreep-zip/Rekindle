use thiserror::Error;

/// All Lost Cargo errors. Variants are distinguished so callers can differentiate
/// "drop this peer's chunk and re-request elsewhere" (`ChunkHashMismatch`) from
/// "the announcer or the file itself is corrupt" (`MerkleRootMismatch`).
#[derive(Debug, Error)]
pub enum FilesError {
    #[error("file too large: {actual} bytes (max {max})")]
    FileTooLarge { actual: u64, max: u64 },

    #[error("chunk hash mismatch at index {index}")]
    ChunkHashMismatch { index: u32 },

    #[error("merkle root mismatch")]
    MerkleRootMismatch,

    #[error("chunk index {index} out of range (chunk_count {chunk_count})")]
    ChunkIndexOutOfRange { index: u32, chunk_count: u32 },

    #[error("offer has {hashes} chunk hashes but chunk_count is {chunk_count}")]
    OfferHashCountMismatch { hashes: usize, chunk_count: u32 },

    #[error("cache I/O at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("invalid attachment id hex: {0}")]
    InvalidAttachmentId(String),

    // ── Phase 15 — surface variants for FilesDeps-parameterised flows ──
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("transport: {0}")]
    Transport(String),

    #[error("database: {0}")]
    Db(String),

    #[error("MEK unavailable for generation {generation} (community {community})")]
    MekUnavailable { community: String, generation: u64 },

    #[error("identity not loaded")]
    IdentityNotLoaded,

    #[error("encrypt failed: {0}")]
    Encrypt(String),

    #[error("decrypt failed: {0}")]
    Decrypt(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("slowmode: {0}")]
    Slowmode(String),

    #[error("attachment offer invalid: {0}")]
    OfferInvalid(String),

    #[error("download incomplete: have {have}/{total} chunks")]
    DownloadIncomplete { have: u32, total: u32 },
}

impl FilesError {
    pub(crate) fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
