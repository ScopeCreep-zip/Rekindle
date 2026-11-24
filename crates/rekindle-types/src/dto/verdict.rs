//! An action's verdict as the Host decides it (memory: backend owns policy),
//! on the wire `{allowed: true}` or `{allowed: false, reason}`: a refusal
//! always names its reason, an allowed action never carries one. One shape
//! for every view that carries verdicts (`ServerVerdict`, `LobbyVerdict`),
//! each with its own refusal enum.

use serde::{Deserialize, Serialize};

/// `Allowed`, or `Refused` with an `R` reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "RawVerdict<R>",
    into = "RawVerdict<R>",
    bound(
        serialize = "R: Serialize + Clone",
        deserialize = "R: Deserialize<'de>"
    )
)]
pub enum ActionVerdict<R> {
    Allowed,
    Refused { reason: R },
}

impl<R> ActionVerdict<R> {
    #[must_use]
    pub const fn refused(reason: R) -> Self {
        Self::Refused { reason }
    }

    #[must_use]
    pub const fn allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    bound(serialize = "R: Serialize", deserialize = "R: Deserialize<'de>")
)]
struct RawVerdict<R> {
    allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<R>,
}

impl<R> TryFrom<RawVerdict<R>> for ActionVerdict<R> {
    type Error = &'static str;

    fn try_from(raw: RawVerdict<R>) -> Result<Self, Self::Error> {
        match (raw.allowed, raw.reason) {
            (true, None) => Ok(Self::Allowed),
            (false, Some(reason)) => Ok(Self::Refused { reason }),
            (true, Some(_)) => Err("an allowed verdict carries no reason"),
            (false, None) => Err("a refused verdict names its reason"),
        }
    }
}

impl<R> From<ActionVerdict<R>> for RawVerdict<R> {
    fn from(v: ActionVerdict<R>) -> Self {
        match v {
            ActionVerdict::Allowed => Self {
                allowed: true,
                reason: None,
            },
            ActionVerdict::Refused { reason } => Self {
                allowed: false,
                reason: Some(reason),
            },
        }
    }
}
