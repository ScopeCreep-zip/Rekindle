//! A window's title strip as the skin pack lays it (plan SK1-S1, lead
//! decision (1)): the pack's logo, its window-name field and its
//! minimise / maximise / close buttons, each at the pack's own pixel
//! coordinates in the window, with no title text and no glyphs over the
//! pack's art. Xfire laid these as separate tiles (Symbiosis
//! `MainWindow.xml:47-58`: `XfireLogo` and three 18×16 `PushButton`s at
//! right offsets 49/25/5 in `InfoBar`); research `sk1r/research.md` §1.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::skin::{RegionState, TileAxis};

/// The role of one placed title part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TitleRole {
    /// The pack's logo art (decorative; part of the drag strip).
    Logo,
    /// The window's name field: the window's name in the pack's font.
    Name,
    /// The minimise button.
    Minimize,
    /// The maximise button.
    Maximize,
    /// The close button.
    Close,
}

impl TitleRole {
    /// Whether the part is a window button.
    #[must_use]
    pub const fn is_button(self) -> bool {
        matches!(self, Self::Minimize | Self::Maximize | Self::Close)
    }

    /// The `.xfire-titlebar-<key>` class suffix and manifest key.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::Name => "name",
            Self::Minimize => "minimize",
            Self::Maximize => "maximize",
            Self::Close => "close",
        }
    }
}

/// The name field's face as the pack declares it (Xfire's `<Text Font=
/// "Arial" Size="11" Bold="true">`, CSS px).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TitleText {
    pub font: String,
    pub size: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
}

/// One placed part, in window coordinates: `x` is `Start`, `End` or (a
/// name field that stretches) `Repeat { start, end }`; `y` from the top.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TitlePart {
    pub role: TitleRole,
    pub x: TileAxis,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// The part's normal package image, drawn at its own size (the logo
    /// and the buttons; the name field has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// A button's hover and down images (Xfire's `Hover` / `Down` sets).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub states: BTreeMap<RegionState, String>,
    /// The name field's face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TitleText>,
}

/// The title strip: its bottom edge in window px (the strip is the drag
/// region above the content) and its parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TitleLayout {
    pub bottom: u16,
    pub parts: Vec<TitlePart>,
}
