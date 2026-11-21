//! A region the pack lays in a window's frame (plan SK0-L1), beside
//! [`super::skin::WindowLayout`] (`skin.rs` is at its line ceiling).

use serde::{Deserialize, Serialize};

use super::skin::TileAxis;

/// A rectangle the pack lays in the frame rectangle (plan SK0-L1): placed
/// once on each axis (`Start`, `End` or `Centre`; never `Repeat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutRegion {
    pub x: TileAxis,
    pub y: TileAxis,
    pub width: u16,
    pub height: u16,
}
