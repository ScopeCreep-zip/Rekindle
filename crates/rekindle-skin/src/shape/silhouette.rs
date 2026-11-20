//! The window's opaque region at a size (plan SK0 S1 `silhouette`).

use std::collections::HashMap;

use rekindle_types::dto::skin::{TileAxis, WindowLayout};

use super::{Mask, ShapeError, MAX_RECTS};

/// The device-pixel ratio of 1.0, in thousandths (`scale` is per mille so
/// the result is integer arithmetic and the same on every platform).
pub const SCALE_ONE: u32 = 1000;

/// A rectangle of the window, device pixels from its top left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A box on the window, logical px: (x, y, width, height).
pub(super) type Area = (i64, i64, i64, i64);

/// A tile's copies across and down: (position, length) each.
type Axes = (Vec<(i64, i64)>, Vec<(i64, i64)>);

/// The copies of a tile along one axis of its box: (position, length).
pub(super) fn copies(axis: TileAxis, start: i64, len: i64, tile: i64) -> Vec<(i64, i64)> {
    match axis {
        TileAxis::Start { offset } => vec![(start + i64::from(offset), tile)],
        TileAxis::End { offset } => vec![(start + len - i64::from(offset) - tile, tile)],
        TileAxis::Centre => vec![(start + (len - tile).div_euclid(2), tile)],
        TileAxis::Repeat { start: a, end: b } => {
            let (mut at, stop) = (start + i64::from(a), start + len - i64::from(b));
            let mut out = Vec::new();
            while tile > 0 && at < stop {
                out.push((at, tile.min(stop - at)));
                at += tile;
            }
            out
        }
    }
}

/// The frame rectangle and the content rectangle of a `width × height`
/// window, logical px.
pub(super) fn boxes(layout: &WindowLayout, width: u32, height: u32) -> (Area, Area) {
    let (o, c) = (layout.outsets, layout.content);
    let frame = (
        i64::from(o.left),
        i64::from(o.top),
        i64::from(width) - i64::from(o.left) - i64::from(o.right),
        i64::from(height) - i64::from(o.top) - i64::from(o.bottom),
    );
    let content = (
        frame.0 + i64::from(c.left),
        frame.1 + i64::from(c.top),
        frame.2 - i64::from(c.left) - i64::from(c.right),
        frame.3 - i64::from(c.top) - i64::from(c.bottom),
    );
    (frame, content)
}

/// Every placed copy of every tile: (tile index, x, y, width, height).
pub(super) fn placed(layout: &WindowLayout, width: u32, height: u32) -> Vec<(usize, Area)> {
    let (frame, _) = boxes(layout, width, height);
    let window = (0, 0, i64::from(width), i64::from(height));
    let mut out = Vec::new();
    for (i, t) in layout.tiles.iter().enumerate() {
        let b = if t.overhang { window } else { frame };
        let xs = copies(t.x, b.0, b.2, i64::from(t.width));
        let ys = copies(t.y, b.1, b.3, i64::from(t.height));
        for &(y, h) in &ys {
            for &(x, w) in &xs {
                out.push((i, (x, y, w, h)));
            }
        }
    }
    out
}

fn fits(layout: &WindowLayout, masks: &[Mask]) -> Result<(), ShapeError> {
    if masks.len() != layout.tiles.len() {
        return Err(ShapeError::Mask {
            tile: masks.len().min(layout.tiles.len()),
        });
    }
    for (tile, (t, m)) in layout.tiles.iter().zip(masks).enumerate() {
        if (m.width(), m.height()) != (u32::from(t.width), u32::from(t.height)) {
            return Err(ShapeError::Mask { tile });
        }
    }
    Ok(())
}

/// Sorted, merged `[start, end)` runs.
fn merge(mut runs: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    runs.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(runs.len());
    for (a, b) in runs {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn device(v: i64, scale: u32) -> u32 {
    u32::try_from(v.max(0) * i64::from(scale) / i64::from(SCALE_ONE)).unwrap_or(u32::MAX)
}

/// The opaque region of a `width × height` (logical px) window laid by
/// `layout` with `masks` (one per tile, [`super::of_layout`]), as rectangles in device pixels at `scale` (per mille, see
/// [`SCALE_ONE`]): the content rectangle, whole, and every tile's opaque
/// pixels where the pack lays it, clipped to the window. Equal consecutive
/// scanlines merge into one rectangle; the rectangles are disjoint and
/// sorted top to bottom, left to right, so the result is deterministic.
///
/// # Errors
/// A mask that is not its tile's size, or more than [`MAX_RECTS`] rectangles.
pub fn silhouette(
    layout: &WindowLayout,
    masks: &[Mask],
    width: u32,
    height: u32,
    scale: u32,
) -> Result<Vec<Rect>, ShapeError> {
    fits(layout, masks)?;
    let runs_of: Vec<Vec<Vec<(u32, u32)>>> = masks
        .iter()
        .map(|m| (0..m.height()).map(|y| m.runs(y)).collect())
        .collect();
    let (frame, content) = boxes(layout, width, height);
    let window = (0, 0, i64::from(width), i64::from(height));
    let axes: Vec<Axes> = layout
        .tiles
        .iter()
        .map(|t| {
            let b = if t.overhang { window } else { frame };
            (
                copies(t.x, b.0, b.2, i64::from(t.width)),
                copies(t.y, b.1, b.3, i64::from(t.height)),
            )
        })
        .collect();
    let w = i64::from(width);
    let mut cache: HashMap<Vec<(usize, usize)>, Vec<(i64, i64)>> = HashMap::new();
    let mut logical: Vec<(i64, i64, i64, i64)> = Vec::new();
    let mut open: Vec<(i64, i64)> = Vec::new();
    let mut open_from = 0i64;
    for y in 0..=i64::from(height) {
        let row = if y == i64::from(height) {
            Vec::new()
        } else {
            // Each tile's source row under scanline `y`, if a copy covers it.
            let key: Vec<(usize, usize)> = axes
                .iter()
                .enumerate()
                .filter_map(|(i, (_, ys))| {
                    let k = ys.partition_point(|c| c.0 + c.1 <= y);
                    let c = ys.get(k).filter(|c| c.0 <= y)?;
                    Some((i, usize::try_from(y - c.0).ok()?))
                })
                .collect();
            let in_content = y >= content.1 && y < content.1 + content.3 && content.2 > 0;
            let mut row = cache
                .entry(key)
                .or_insert_with_key(|key| {
                    let mut runs = Vec::new();
                    for &(i, src) in key {
                        let tile_runs = runs_of[i].get(src).map_or(&[][..], Vec::as_slice);
                        for &(x, cw) in &axes[i].0 {
                            for &(a, b) in tile_runs {
                                let (a, b) = (i64::from(a), i64::from(b).min(cw));
                                if a < b {
                                    runs.push(((x + a).max(0), (x + b).min(w)));
                                }
                            }
                        }
                    }
                    runs.retain(|r| r.0 < r.1);
                    merge(runs)
                })
                .clone();
            if in_content {
                row.push((content.0.max(0), (content.0 + content.2).min(w)));
            }
            merge(row)
        };
        if row != open {
            logical.extend(
                open.iter()
                    .map(|&(a, b)| (a, open_from, b - a, y - open_from)),
            );
            if logical.len() > MAX_RECTS {
                return Err(ShapeError::TooManyRects {
                    count: logical.len(),
                });
            }
            open = row;
            open_from = y;
        }
    }
    let mut out: Vec<Rect> = logical
        .into_iter()
        .filter_map(|(x, y, rw, rh)| {
            let (x0, y0) = (device(x, scale), device(y, scale));
            let (x1, y1) = (device(x + rw, scale), device(y + rh, scale));
            (x1 > x0 && y1 > y0).then_some(Rect {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
            })
        })
        .collect();
    out.sort_unstable_by_key(|r| (r.y, r.x));
    Ok(out)
}
