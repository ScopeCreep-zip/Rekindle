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
    /// The manifest's credits; a library skin's are as the skin states
    /// them (self-claimed, not verified).
    pub credits: Option<crate::dto::skin::SkinCredits>,
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
    /// The library skin on trial, shown in place of `active` until it is
    /// kept or reverted.
    pub trial: Option<SkinTrial>,
}

/// How long a library skin is tried before the Host reverts it unless the
/// user keeps it (plan E7.7.2 "Apply", Telegram's "Keep this theme?").
pub const SKIN_TRIAL_MS: u64 = 15_000;

/// A library skin on trial (`SkinTrialChanged`'s payload): every window
/// shows it, the identity's stored choice is still `previous`, and at
/// `reverts_at_ms` the Host puts `previous` back unless `SkinKeep` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinTrial {
    /// `pkg:<hash>`.
    pub id: String,
    /// Its manifest's display name.
    pub name: String,
    /// The identity's skin before the trial, which a revert restores.
    pub previous: String,
    /// When the Host reverts it (ms since the epoch, the Host's clock).
    pub reverts_at_ms: u64,
}

/// What a window applies (`SkinGet`'s answer and `SkinChanged`'s payload):
/// the skin, its active variant's base for `:root[data-skin-base]`, the
/// `skin://` href of its `@layer skin` stylesheet and the region slots it
/// paints, and the palette subset a terminal frontend applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinApplied {
    /// `builtin:<id>`, `pkg:<hash>` or `dev:<hash>`.
    pub id: String,
    pub base: crate::dto::skin::SkinBase,
    pub tokens_href: String,
    pub regions: Vec<String>,
    /// The skin's resolved colours for the TUI's roles (plan E7.7.2 "the
    /// TUI applies the palette subset from the same DTO"), mapped by the
    /// generator's TUI table, so a terminal never parses CSS.
    pub palette: Vec<SkinPaletteColour>,
}

/// One TUI colour role and its opaque sRGB value (a translucent token is
/// composited over the skin's window background by the Host).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinPaletteColour {
    /// The TUI theme's token, e.g. `accent.primary`.
    pub role: String,
    pub rgb: [u8; 3],
}

/// One file the `skin://` scheme serves (`SkinFile`'s answer): the shell
/// copies it into the HTTP response unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinServed {
    pub content_type: String,
    /// `immutable` for a built-in, `no-store` for everything else.
    pub cache_control: String,
    #[serde(with = "crate::base64_bytes")]
    pub bytes: Vec<u8>,
}
