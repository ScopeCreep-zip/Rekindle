//! Explicit edits to the active identity's profile in one community.

use serde::{Deserialize, Serialize};

/// What to do with one profile field.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "op",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ProfileField<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

impl<T> ProfileField<T> {
    /// The proposed value, when this edit supplies one.
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Unchanged | Self::Clear => None,
        }
    }

    /// Whether this edit leaves the stored field as it is.
    #[must_use]
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }

    /// Apply this edit to an optional stored field.
    pub fn apply_to(self, stored: &mut Option<T>) {
        match self {
            Self::Unchanged => {}
            Self::Clear => *stored = None,
            Self::Set(value) => *stored = Some(value),
        }
    }
}

/// A patch in which each field's operation is explicit on the wire.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityProfilePatch {
    pub bio: ProfileField<String>,
    pub pronouns: ProfileField<String>,
    pub theme_color: ProfileField<u32>,
    pub badges: ProfileField<Vec<String>>,
    pub avatar_ref: ProfileField<String>,
    pub banner_ref: ProfileField<String>,
    pub custom_status: ProfileField<String>,
    pub display_name: ProfileField<String>,
}
