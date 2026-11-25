//! The served style of a worded tab clone's words (plan SK15,
//! [`super::skin_tree::NodeLabel::style`]).

use serde::{Deserialize, Serialize};

/// The style of a source tab's word, measured from its art (plan SK15,
/// [`super::skin_tree::NodeLabel::style`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LabelStyle {
    /// Font size, px.
    pub size_px: u16,
    /// CSS font weight.
    pub weight: u16,
    /// The word's colour, `#RRGGBB`.
    pub color: String,
    /// How the words sit in `text_box`, as the source word sits in its art.
    pub align: LabelAlign,
    /// `[x, y, width, height]` of the node's box, px from its top-left,
    /// that the words are laid in: from the source word's ink start across
    /// the node (start-aligned), or the node less its frame (centred). It
    /// is not filled; `NodeLabel::cover` hides the copied word.
    pub text_box: [u16; 4],
}

/// Where a word sits in its box ([`LabelStyle::align`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelAlign {
    Start,
    Centre,
}
