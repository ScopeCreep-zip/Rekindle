//! A window's frame layout (plan SK0 S1, SK0-L1): the window kinds a skin
//! frames, the tiles that lay the frame and the pack's component tree. Re-exported from [`super::skin`], whose manifest carries them.

use serde::{Deserialize, Serialize};

use super::skin::Insets;

/// A window kind whose frame a skin lays (plan SK0 S1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkinWindow {
    /// The buddy list (Xfire's `<MainSkin>`).
    Main,
    /// A conversation window (Xfire's `<ChatSkin>`).
    Chat,
}

/// How a frame tile is placed along one axis of its box (plan SK0 S1):
/// the frame rectangle, or the whole window for an overhang tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum TileAxis {
    /// Once, its near side `offset` px from the start edge.
    Start { offset: u16 },
    /// Once, its far side `offset` px before the end edge.
    End { offset: u16 },
    /// Once, centred in the box.
    Centre,
    /// Repeated at its source size (never stretched) from `start` px to
    /// `end` px before the end edge, the last copy cut.
    Repeat { start: u16, end: u16 },
}

/// One frame tile: an image of the package placed by its anchors. Its
/// 1-bit mask (opaque where alpha ≥ 128) is derived from the image itself
/// wherever the layout is checked or shaped, so the shape has one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameTile {
    pub image: String,
    pub width: u16,
    pub height: u16,
    pub x: TileAxis,
    pub y: TileAxis,
    /// Placed in the whole window (art outside the main frame), not in
    /// the frame rectangle.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overhang: bool,
    /// A pack tile above the frame band (its Xfire `Z`): it may lie over the content rectangle, draws
    /// beneath the content's interactive layer, adds its opaque pixels outside the content to the
    /// silhouette and never cuts the content. `None` for a frame piece.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration: Option<u8>,
    /// The pack's centre fill laid in the band between a side piece and the views (plan SK0-L1, lead
    /// decision): a frame tile allowed only in that band, wholly opaque, never over the content.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fill: bool,
}

/// A window's frame layout (plan SK0 S1): the window is the frame
/// rectangle grown by `outsets` (room for overhang art); the content
/// rectangle is the frame rectangle shrunk by `content` and is always
/// opaque; `min_*` is the smallest frame the tiles fit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowLayout {
    pub outsets: Insets,
    pub content: Insets,
    pub min_width: u16,
    pub min_height: u16,
    pub tiles: Vec<FrameTile>,
    /// The pack-laid title strip (plan SK1-S1); `None` keeps the app's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<super::skin_title::TitleLayout>,
    /// The pack's component tree (plan SK2); `content` is its views node's box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<super::skin_tree::SkinTree>,
}
