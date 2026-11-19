//! The skin editor over IPC (plan E7.7.5, Telegram's model): a draft the
//! editor holds, and the Host's report on it. Every check is the Host's
//! (`SkinValidateDraft`, `rekindle_skin::draft`); the editor renders the
//! report and sends the author's edits back.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dto::skin::{SkinBase, SkinContrast, TokenValue};

/// A skin being edited: one variant's tokens, by registry id. A token the
/// draft does not set takes the base's default, as in a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinDraft {
    /// The display name the exported manifest carries.
    pub name: String,
    pub base: SkinBase,
    pub contrast: SkinContrast,
    pub tokens: BTreeMap<String, TokenValue>,
}

/// The Host's report on a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinDraftReport {
    /// Every themeable registry key, resolved on the draft's base.
    pub tokens: Vec<SkinDraftToken>,
    /// Every declared contrast pair, measured on composited colours.
    pub pairs: Vec<SkinPairCheck>,
    /// Every other refusal the validator would give (base mismatch, a
    /// translucent ground, D1–D3, S1, the manifest's text rules).
    pub failures: Vec<String>,
    /// The draft's `@layer skin` token stylesheet, for live apply to the
    /// editing window only.
    pub tokens_css: String,
    /// No pair fails and nothing else is refused: export is allowed.
    pub exportable: bool,
}

/// One themeable registry key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinDraftToken {
    /// The registry id (`bg.panel`).
    pub id: String,
    /// The CSS custom property the app reads.
    pub css: String,
    /// The resolved value on the draft's base.
    pub value: TokenValue,
    /// The draft sets it (otherwise the default or a derivation applies).
    pub set: bool,
}

/// One declared contrast pair: WCAG 2.x is the gate, APCA is advisory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinPairCheck {
    /// The rule that declares it (`1.4.3`, `1.4.11`, `D4`, `S2`, …).
    pub rule: String,
    pub fg: String,
    pub ground: String,
    /// The WCAG ratio, hundredths, floored.
    pub ratio_x100: u32,
    pub required_x100: u32,
    pub pass: bool,
    /// The APCA-W3 0.0.98G Lc, tenths (negative for light text on dark).
    pub apca_lc_x10: i32,
    /// For a failing pair: the smallest Oklab-lightness move of the token
    /// the skin may set that passes it, applied only by the author's click.
    pub suggestion: Option<SkinSuggestion>,
}

/// A passing colour for a failing pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinSuggestion {
    /// The token to set.
    pub token: String,
    pub rgb: [u8; 3],
    /// The pair's ratio with it, hundredths.
    pub ratio_x100: u32,
}
