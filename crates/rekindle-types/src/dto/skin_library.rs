//! The skin library over IPC (plan E7.7.2): `SkinList`'s answer and the
//! entry `SkinImport` answers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dto::skin::SkinWindow;

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
    /// How many controls the skin serves under 24 CSS px with no spacing
    /// exception (WCAG 2.2 SC 2.5.8, `TargetRule::Short`), counted from
    /// the package as served.
    pub short_targets: u32,
    /// `short_targets == 0`: every control meets SC 2.5.8.
    pub conforming: bool,
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

/// Why a window could not apply a skin (`SkinApplyFailed`): that window
/// keeps the skin it shows, and the Host decides the rest (it reverts a
/// skin on trial).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum SkinApplyFailure {
    /// An image the stylesheet draws did not decode; `path` is its
    /// `skin://` href (one of `SkinApplied::images`).
    ImageDecode { path: String },
    /// The stylesheet did not load; `detail` is what the window saw.
    SheetError { detail: String },
    /// The native canvas cannot fit the skin's minimum logical dimensions.
    WindowTooSmall { width: u16, height: u16 },
    /// The native clock expired before the window could present this skin.
    LoadTimeout { message: String },
}

impl SkinApplyFailure {
    /// The skin-load error the Host and desktop show on deadline.
    pub const LOAD_TIMEOUT_MESSAGE: &'static str =
        "Rekindle could not load the skin for this window in time.";

    /// The skin-load error a window shows when an image or its stylesheet
    /// did not load.
    pub const LOAD_FAILED_MESSAGE: &'static str =
        "Rekindle could not load the skin for this window.";

    #[must_use]
    pub fn load_timeout() -> Self {
        Self::LoadTimeout {
            message: Self::LOAD_TIMEOUT_MESSAGE.to_owned(),
        }
    }

    /// What the window tells the user about this failure: the Host's words,
    /// which the window shows as served (thin frontend).
    #[must_use]
    pub fn notice(&self) -> String {
        match self {
            Self::ImageDecode { .. } | Self::SheetError { .. } => {
                Self::LOAD_FAILED_MESSAGE.to_owned()
            }
            Self::WindowTooSmall { width, height } => {
                format!("This skin needs a window at least {width} × {height} pixels.")
            }
            Self::LoadTimeout { message } => message.clone(),
        }
    }
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
    /// Shared region art to decode before showing the sheet.
    pub images: Vec<String>,
    /// Frame and title art to decode only for the corresponding window kind.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub frame_images: BTreeMap<SkinWindow, Vec<String>>,
    /// The skin's resolved colours for the TUI's roles (plan E7.7.2 "the
    /// TUI applies the palette subset from the same DTO"), mapped by the
    /// generator's TUI table, so a terminal never parses CSS.
    pub palette: Vec<SkinPaletteColour>,
    /// The outline of each window kind the skin lays as a component tree
    /// (plan SK2-S2): the webview nests one `div[data-skin-node]` per node
    /// and binds each role; a window kind absent here keeps its own layout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shells: Vec<crate::dto::skin_tree::SkinShell>,
    /// The brand glyph the shell draws inline in the title strip's logo
    /// box (design `br`): the first pack strip logo's, Main before Chat,
    /// by the box rule (`skin_tree::brand::glyph`); absent when no strip
    /// lays a logo. The stylesheet only places that box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_logo: Option<crate::dto::skin_tree::NodeGlyph>,
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

/// One frame tile's or tree node art's mask (plan SK0 S3, SK2-S1r): rows of `ceil(width / 8)`
/// bytes, most significant bit first, a set bit an opaque pixel; and its
/// faint pixels, (x, y, alpha) with alpha under `rekindle_skin::shape::ALPHA_OPAQUE`,
/// sorted by row then column, which the shaper composites with the other
/// layers laid on them (`rekindle_skin::shape::silhouette`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinTileMask {
    pub width: u32,
    pub height: u32,
    pub bits: Vec<u8>,
    pub faint: Vec<(u32, u32, u8)>,
}

/// One window kind's frame layout and its masks: its tiles', in tile order,
/// then the normal art's of each tree node in the outline (plan SK2-S1r,
/// `rekindle_skin::shape::outline_nodes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinWindowFrame {
    pub window: crate::dto::skin::SkinWindow,
    pub layout: crate::dto::skin::WindowLayout,
    pub masks: Vec<SkinTileMask>,
}

/// `SkinFrames`' answer: the shown skin's window frames; a window kind
/// absent here keeps its whole rectangle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinFrames {
    /// The skin the frames are of.
    pub id: String,
    /// The same content-addressed stylesheet URI as SkinApplied.
    pub tokens_href: String,
    pub windows: Vec<SkinWindowFrame>,
}
