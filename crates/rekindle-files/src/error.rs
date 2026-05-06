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
}

impl FilesError {
    pub(crate) fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
