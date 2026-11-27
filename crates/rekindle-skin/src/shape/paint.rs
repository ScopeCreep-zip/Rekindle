//! A legacy frame can repeat art also laid by its component tree. Keep
//! the authored tree placement, except for a stretched ground plane
//! already drawn beneath the frame. The node and its input remain either way.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::dto::skin::{FrameTile, Insets, TileAxis, WindowLayout};
use rekindle_types::dto::skin_tree::{Anchor, NodeArt, NodeRole, NodeSpan, PieceSlot, SkinNode};

use crate::skin_tree::pieces::{self, Band};

use super::{min_window, MAX_WINDOW_HEIGHT, MAX_WINDOW_WIDTH};

#[derive(Clone, Copy)]
struct PaintedSpan {
    start: i32,
    end: i32,
    origin: i32,
}

impl PaintedSpan {
    fn covers(self, other: Self, period: i32) -> bool {
        self.start <= other.start
            && self.end >= other.end
            && (self.origin - other.origin).rem_euclid(period) == 0
    }
}

#[derive(Clone, Copy)]
struct FrameAxis {
    axis: TileAxis,
    insets: (u16, u16),
}

impl FrameAxis {
    fn at(self, extent: i32, natural: i32) -> PaintedSpan {
        let (near, far) = (i32::from(self.insets.0), i32::from(self.insets.1));
        let (start, end) = match self.axis {
            TileAxis::Start { offset } => {
                let start = near + i32::from(offset);
                (start, start + natural)
            }
            TileAxis::End { offset } => {
                let end = extent - far - i32::from(offset);
                (end - natural, end)
            }
            TileAxis::Centre => {
                let start = near + (extent - near - far - natural).div_euclid(2);
                (start, start + natural)
            }
            TileAxis::Repeat { start, end } => {
                (near + i32::from(start), extent - far - i32::from(end))
            }
        };
        PaintedSpan {
            start,
            end,
            origin: start,
        }
    }
}

struct TreeAxis<'a> {
    axis: &'a NodeSpan,
    outsets: (u16, u16),
    band: (i32, i32),
    repeats: bool,
    at_end: bool,
}

impl TreeAxis<'_> {
    fn at(&self, extent: i32, natural: i32) -> PaintedSpan {
        let (near, far) = (i32::from(self.outsets.0), i32::from(self.outsets.1));
        let (position, size) = self.axis.place(extent - near - far);
        let start = near + position + self.band.0;
        let end = near + position + size - self.band.1;
        let origin = if self.at_end { end - natural } else { start };
        if self.repeats {
            PaintedSpan { start, end, origin }
        } else {
            PaintedSpan {
                start: start.max(origin),
                end: end.min(origin + natural),
                origin,
            }
        }
    }
}

fn independent(span: &NodeSpan) -> bool {
    matches!(
        span,
        NodeSpan::Stretch { .. }
            | NodeSpan::Fixed {
                at: Anchor::Start(_) | Anchor::End(_) | Anchor::Centre { .. },
                ..
            }
    )
}

/// A negative-depth, full background derives the frame's underlay. Keep
/// that layer only when its piece covers the complete tree background.
fn is_ground(tile: &FrameTile, node: &SkinNode, art: &NodeArt) -> bool {
    tile.overhang
        && tile.decoration == Some(0)
        && node.role == NodeRole::Decor
        && node.z < 0
        && matches!(node.x, NodeSpan::Stretch { .. })
        && matches!(node.y, NodeSpan::Stretch { .. })
        && art.normal.len() == 1
        && art.states.is_empty()
}

/// Axes are independent. Check every supported extent so a centered
/// piece, cap, or partial final repeat cannot leave unmatched paint.
fn covered(
    frame: FrameAxis,
    tree: &TreeAxis<'_>,
    min: u16,
    max: u32,
    size: u16,
    ground: bool,
) -> bool {
    let natural = i32::from(size);
    let Ok(maximum) = i32::try_from(max) else {
        return false;
    };
    let minimum = i32::from(min);
    minimum <= maximum
        && (minimum..=maximum).all(|extent| {
            let frame = frame.at(extent, natural);
            let tree = tree.at(extent, natural);
            tree.covers(frame, natural) && (!ground || frame.covers(tree, natural))
        })
}

struct WindowGeometry {
    frame: Insets,
    tree: Insets,
    minimum: (u16, u16),
}

impl WindowGeometry {
    fn covered(
        &self,
        tile: &FrameTile,
        node: &SkinNode,
        art: &NodeArt,
        slot: PieceSlot,
        ground: bool,
    ) -> bool {
        let cell = pieces::cell(slot);
        let [left, right, top, bottom] = pieces::insets(&art.normal);
        let frame = if tile.overhang {
            Insets {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            }
        } else {
            self.frame
        };
        let x = TreeAxis {
            axis: &node.x,
            outsets: (self.tree.left, self.tree.right),
            band: if matches!(cell.band, Band::Row | Band::Middle) {
                (left, right)
            } else {
                (0, 0)
            },
            repeats: cell.repeat.0,
            at_end: cell.end.0,
        };
        let y = TreeAxis {
            axis: &node.y,
            outsets: (self.tree.top, self.tree.bottom),
            band: if matches!(cell.band, Band::Column | Band::Middle) {
                (top, bottom)
            } else {
                (0, 0)
            },
            repeats: cell.repeat.1,
            at_end: cell.end.1,
        };
        covered(
            FrameAxis {
                axis: tile.x,
                insets: (frame.left, frame.right),
            },
            &x,
            self.minimum.0,
            MAX_WINDOW_WIDTH,
            tile.width,
            ground,
        ) && covered(
            FrameAxis {
                axis: tile.y,
                insets: (frame.top, frame.bottom),
            },
            &y,
            self.minimum.1,
            MAX_WINDOW_HEIGHT,
            tile.height,
            ground,
        )
    }
}

/// Find an unconditional root painting the same bytes over the frame
/// tile, with a matching repeat phase.
fn duplicate(
    tile: &FrameTile,
    roots: &[SkinNode],
    images: &BTreeMap<String, Vec<u8>>,
    geometry: &WindowGeometry,
) -> Option<(usize, bool)> {
    let frame_image = images.get(&tile.image)?;
    roots.iter().enumerate().find_map(|(index, node)| {
        if node.shows_with.is_some()
            || node.fit.is_some()
            || matches!(node.role, NodeRole::View(_))
            || !independent(&node.x)
            || !independent(&node.y)
        {
            return None;
        }
        let art = node.art.as_ref()?;
        let ground = is_ground(tile, node, art);
        art.normal
            .iter()
            .find(|(slot, piece)| {
                tile.width == piece.width
                    && tile.height == piece.height
                    && images
                        .get(&piece.image)
                        .is_some_and(|bytes| bytes == frame_image)
                    && geometry.covered(tile, node, art, **slot, ground)
            })
            .map(|_| (index, ground))
    })
}

/// Paint each repeated legacy image in one layer after fitting the tree.
/// Keep the original geometry, triggers, and package image bytes.
pub(crate) fn deduplicate_legacy_frame(
    layout: &mut WindowLayout,
    images: &BTreeMap<String, Vec<u8>>,
) {
    let Some(minimum) = min_window(layout) else {
        return;
    };
    let Some(tree) = &mut layout.tree else {
        return;
    };
    let geometry = WindowGeometry {
        frame: layout.outsets,
        tree: tree.outsets,
        minimum,
    };
    let mut grounds = BTreeSet::new();
    layout.tiles.retain(|tile| {
        if tile.width == 0 || tile.height == 0 {
            return true;
        }
        match duplicate(tile, &tree.root, images, &geometry) {
            None => true,
            Some((index, true)) => grounds.insert(index),
            Some((_, false)) => false,
        }
    });
    for index in grounds {
        tree.root[index].art = None;
    }
}
