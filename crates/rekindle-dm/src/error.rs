use thiserror::Error;

/// Why a group-DM operation was refused or failed.
#[derive(Debug, Error)]
pub enum DmError {
    /// A roster outside 3-8 members, a repeated member, or a member that is
    /// not an identity key with two record keys.
    #[error("invalid roster: {0}")]
    InvalidRoster(String),
    /// An empty group name or one past its bound.
    #[error("invalid group name: {0}")]
    InvalidName(String),
    /// The invite's signature is not its creator's over its roster.
    #[error("group invite signature does not verify")]
    BadSignature,
    /// The sender (or we) is not on the group's roster.
    #[error("{0} is not on the group's roster")]
    NotOnRoster(String),
    /// We hold no such group (or no such pending invite).
    #[error("unknown group {0}")]
    UnknownGroup(String),
    /// We already hold this group.
    #[error("group {0} is already held")]
    AlreadyMember(String),
    /// A frame or key at an older epoch than the sender's current key.
    #[error("epoch {got} is older than the sender's current {current}")]
    StaleEpoch { got: u32, current: u32 },
    /// A different key for an epoch we already hold.
    #[error("a second key for epoch {0}")]
    ConflictingKey(u32),
    /// A frame at an epoch whose key we do not hold yet.
    #[error("no key of {member} for epoch {epoch}")]
    KeyMissing { member: String, epoch: u32 },
    /// We announced no group chain of ours for this group.
    #[error("no group chain of ours in group {0}")]
    ChainMissing(String),
    /// Our key's epochs are exhausted.
    #[error("sending key epochs exhausted")]
    EpochExhausted,
    /// The cipher failed or a ciphertext did not open.
    #[error("crypto: {0}")]
    Crypto(String),
    /// The host's storage failed.
    #[error("storage: {0}")]
    Storage(String),
}

impl DmError {
    /// A storage failure from the host.
    pub fn storage<S: Into<String>>(s: S) -> Self {
        DmError::Storage(s.into())
    }
}

impl From<rekindle_types::error::CryptoError> for DmError {
    fn from(e: rekindle_types::error::CryptoError) -> Self {
        DmError::Crypto(e.to_string())
    }
}
