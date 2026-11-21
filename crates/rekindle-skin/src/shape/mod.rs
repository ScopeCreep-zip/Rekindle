//! Shaped windows (plan SK0 S1): each window kind's frame layout as the
//! skin lays it, a 1-bit mask per tile derived from its image, and the window's opaque region as
//! device-pixel rectangles.
//!
//! Xfire cut its windows with 1-bit Win32 regions built from rectangles
//! (`Xfire.exe` imports `SetWindowRgn`/`CreateRectRgn`/`CombineRgn`, not
//! `UpdateLayeredWindow`; the frame GIFs carry a transparent index), so a
//! pixel is in or out, never half (research `scratchpad/shaped/research.md`
//! §1). [`silhouette`] composes the tiles at a window size the way the
//! pack lays them (middles repeat at their source size, never stretch) and
//! merges equal scanlines into rectangles; [`check`] is the helper's
//! refusal of a layout that would let another app show through the
//! content rectangle (threat model, UI redressing) or cost too much to
//! shape.

mod build;
mod check;
pub mod css;
mod mask;
mod silhouette;
pub mod title;

#[cfg(test)]
mod tests;

pub(crate) use build::u16_of;
pub use build::{from_nine, tile, tile_path};
pub use check::{check, MAX_RECTS, MAX_WINDOW_HEIGHT, MAX_WINDOW_WIDTH};
pub use css::{frame_css, MAX_FRAME_TILES};
pub use mask::{of_layout, Mask, ALPHA_OPAQUE};
pub use silhouette::{content_area, silhouette, Area, Rect, SCALE_ONE};

/// A layout that does not compose (a mask that does not decode, or a
/// silhouette past [`MAX_RECTS`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeError {
    /// Tile `tile` has no mask, or one that is not its size.
    Mask { tile: usize },
    /// More rectangles than [`MAX_RECTS`].
    TooManyRects { count: usize },
}
