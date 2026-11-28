//! The three-state edit of a role's exclusion-group slug (architecture
//! §19.4), as `RoleUpdate` carries it.

use serde::{Deserialize, Serialize};

/// Leave the slug, clear it, or set a new one.
///
/// Externally tagged (`"unchanged"`, `"clear"`, `{"set": "slug"}`): the
/// request rides postcard to the daemon as well as JSON from the webview,
/// and an adjacently tagged unit variant does not round-trip under
/// postcard (its struct form has one field written and two read).
/// `Option<Option<String>>` would say the same three states and is banned
/// by `clippy::option_option`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExclusionGroupEdit {
    /// Keep the current slug.
    #[default]
    Unchanged,
    /// Remove the slug.
    Clear,
    /// Replace the slug.
    Set(String),
}
