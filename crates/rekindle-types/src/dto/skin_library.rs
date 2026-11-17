//! The skin library over IPC (plan E7.7.2): `SkinList`'s answer and the
//! entry `SkinImport` answers.

use serde::{Deserialize, Serialize};

/// One skin the user can choose: a built-in or a library package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinEntry {
    /// `builtin:<id>` or `pkg:<hash>`.
    pub id: String,
    /// The manifest's display name.
    pub name: String,
    /// When it entered the library (ms); `None` for a built-in.
    pub imported_at: Option<i64>,
}

/// Every skin, the identity's choice and the device login skin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinLibrary {
    /// The built-ins, then the library, oldest first.
    pub skins: Vec<SkinEntry>,
    /// The identity's skin (`app_settings.skin_id`).
    pub active: String,
    /// The device login skin: `builtin:<id>` or `dev:<hash>` (a stripped
    /// copy, which matches no library id by design).
    pub login: String,
}
