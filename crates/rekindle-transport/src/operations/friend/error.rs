//! Why a friend or 1:1 operation failed, in the classes a frontend answers
//! differently (the Host maps each to its response status).

/// A friend or 1:1 operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FriendError {
    /// The request is malformed or not allowed (400).
    #[error("{0}")]
    Invalid(String),
    /// What it names does not exist (404).
    #[error("{0}")]
    NotFound(String),
    /// The friendship is not in a state that allows it (409).
    #[error("{0}")]
    Conflict(String),
    /// No identity is unlocked (423).
    #[error("no identity is unlocked")]
    Locked,
    /// The network is not ready yet; retry shortly (503).
    #[error("{0}")]
    NotReady(String),
    /// It failed (500).
    #[error("{0}")]
    Failed(String),
}

impl FriendError {
    /// A store read or write failed.
    pub(crate) fn store(what: &str, e: impl std::fmt::Display) -> Self {
        Self::Failed(format!("{what}: {e}"))
    }

    /// Whether the input was at fault ([`Self::Invalid`]), not us.
    pub(crate) fn is_input(&self) -> bool {
        matches!(self, Self::Invalid(_))
    }

    /// A database call failed: [`Self::Invalid`] when the values it was
    /// given are at fault (a constraint, a size; `rekindle_db::input_fault`,
    /// review RV5 M2), else [`Self::Failed`] (ours).
    pub(crate) fn db(what: &str, e: &rekindle_db::CallError) -> Self {
        if rekindle_db::input_fault(e) {
            Self::Invalid(format!("{what}: {e}"))
        } else {
            Self::store(what, e)
        }
    }
}

impl From<crate::error::TransportError> for FriendError {
    fn from(e: crate::error::TransportError) -> Self {
        Self::Failed(e.to_string())
    }
}
