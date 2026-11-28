//! What every Host-call command answers (plan E1.1, U1.6): its value with
//! the undo an action offers, or why it did not run. These cross only the
//! Tauri and E2E JSON boundaries, never the postcard bus, so the adjacent
//! tagging of [`CommandError`] is safe; the `IntentReview` inside keeps its
//! own tagging.

use serde::{Deserialize, Serialize};

use crate::intent::{IntentReview, UndoOffer};

/// A Host call that ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Done<T> {
    pub value: T,
    /// How to take the action back while the offer lasts.
    pub undo: Option<UndoOffer>,
}

/// A Host call that did not run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum CommandError {
    /// Refused or failed: the host's code, its message and what to do.
    Failed {
        code: u32,
        message: String,
        remediation: Option<String>,
    },
    /// Held by the intent gate: render the review and commit it with the
    /// proof it asks for.
    IntentRequired(IntentReview),
    /// The user cancelled the review the call waited on, so it did not run
    /// (a backend hold's own cancel action ran instead). `intent_cancel`
    /// answers it, and the cancelled call rejects with it.
    Cancelled,
}

impl CommandError {
    /// A failure the shell itself raises (code 500).
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Failed {
            code: 500,
            message: message.into(),
            remediation: None,
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed { message, .. } => f.write_str(message),
            Self::IntentRequired(review) => write!(f, "{:?} needs your review", review.kind),
            Self::Cancelled => f.write_str("Cancelled"),
        }
    }
}
