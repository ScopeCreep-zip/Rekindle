//! The skin editor over IPC (plan E7.7.5, Telegram's model): a draft the
//! editor holds, and the Host's report on it. Every check is the Host's
//! (`SkinValidateDraft`, `rekindle_skin::draft`); the editor renders the
//! report and sends the author's edits back.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dto::skin::{RegionFit, SkinBase, SkinContrast, SkinCredits, SkinLicense, TokenValue};

/// A skin being edited: one variant's tokens, by registry id, its credits
/// and licence, the regions the author imported, and the package it
/// started from. A token the draft does not set takes the base's default,
/// as in a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinDraft {
    /// The display name the exported manifest carries.
    pub name: String,
    /// The manifest's description (≤ 280 scalars, may be empty).
    pub description: String,
    /// Who made this skin: credited first in the exported `credits.authors`
    /// (owner 2026-11-18: everyone gets credit for their work).
    pub author: String,
    /// The licence the author grants: an SPDX expression, or the full text
    /// when it is not one (the Host decides which, as the validator reads it).
    pub license: String,
    /// What the skin came from, carried forward by "Start from the current
    /// skin": the source's credited authors and contributors, its URL, and
    /// the licence it was made under. The author adds to it, never replaces it.
    pub credits: SkinCredits,
    pub base: SkinBase,
    pub contrast: SkinContrast,
    pub tokens: BTreeMap<String, TokenValue>,
    /// The package the draft started from (`builtin:<id>` or `pkg:<hex>`):
    /// its regions, icons and sounds go into the export unless a region
    /// below replaces one.
    pub from: Option<String>,
    /// Regions the author imported, by registry region id.
    pub regions: BTreeMap<String, SkinDraftRegion>,
}

/// One imported region: a PNG or GIF the author chose, read by the Host
/// and checked by the same validator as any package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinDraftRegion {
    /// The image file, an absolute path the OS open dialog returned.
    pub path: String,
    /// Stretch, tile, or nine-slice with its insets.
    pub fit: RegionFit,
    /// The colour token under the region's text, where it carries text.
    pub text_backing: Option<String>,
}

/// A registry region the editor may import an image for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinRegionSlot {
    pub id: String,
    /// The region carries text, so it needs a `textBacking` token.
    pub carries_text: bool,
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
    /// The licence the export states, as the Host read the draft's.
    pub license: SkinLicense,
    /// The credits the export carries: the author first, then what the
    /// skin came from.
    pub credits: SkinCredits,
    /// Every registry region an image may be imported for.
    pub region_slots: Vec<SkinRegionSlot>,
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
