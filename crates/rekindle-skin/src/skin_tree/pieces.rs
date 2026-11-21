//! How a node's piece set draws in its box (spec A7, plan SK2-P1): the one
//! geometry both the stylesheet ([`super::css`]) and the window outline
//! ([`crate::shape::silhouette`]) read, so the two cannot disagree.
//!
//! The box is cut by four insets: the corner columns' widths (the widest of
//! `topleft`/`left`/`bottomleft`, and of the right column) and the corner
//! rows' heights. Each piece then draws in one band of the box:
//! - the corners in the whole box, once, at its corner;
//! - `top`/`bottom` in the row band (between the corner columns, full
//!   height) at its top/bottom, repeated across from the band's start;
//! - `left`/`right` in the column band (between the corner rows, full
//!   width) at its left/right, repeated down from the band's top;
//! - `middle` in the middle band, repeated both ways from its top left.
//!
//! Every copy is clipped to its band, so a cap never repeats and the middle
//! starts at the start cap's end edge (Total Chaos's 8 px title repeat at
//! phase x = 74 behind its 75 px cap laid from x = -1, `skr/tools/period.py`).

use rekindle_types::dto::skin_tree::{NodePiece, NodePieces, PieceSlot};

/// The part of a node's box a piece draws in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// The whole box.
    Box,
    /// Between the corner columns, full height.
    Row,
    /// Between the corner rows, full width.
    Column,
    /// Between both.
    Middle,
}

/// How one slot draws: its band, whether it is laid from the band's far
/// edge on each axis, and whether it repeats on each axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub band: Band,
    pub end: (bool, bool),
    pub repeat: (bool, bool),
}

/// `slot`'s cell.
#[must_use]
pub fn cell(slot: PieceSlot) -> Cell {
    use PieceSlot as S;
    let band = match slot {
        S::TopLeft | S::TopRight | S::BottomLeft | S::BottomRight => Band::Box,
        S::Top | S::Bottom => Band::Row,
        S::Left | S::Right => Band::Column,
        S::Middle => Band::Middle,
    };
    let end = (
        matches!(slot, S::TopRight | S::Right | S::BottomRight),
        matches!(slot, S::BottomLeft | S::Bottom | S::BottomRight),
    );
    let repeat = match band {
        Band::Box => (false, false),
        Band::Row => (true, false),
        Band::Column => (false, true),
        Band::Middle => (true, true),
    };
    Cell { band, end, repeat }
}

/// The insets `[left, right, top, bottom]`: the corner columns' widths and
/// the corner rows' heights.
#[must_use]
pub fn insets(p: &NodePieces) -> [i32; 4] {
    use PieceSlot as S;
    let most = |slots: [S; 3], f: fn(&NodePiece) -> u16| {
        slots
            .iter()
            .filter_map(|s| p.get(s))
            .map(|x| i32::from(f(x)))
            .max()
            .unwrap_or(0)
    };
    [
        most([S::TopLeft, S::Left, S::BottomLeft], |x| x.width),
        most([S::TopRight, S::Right, S::BottomRight], |x| x.width),
        most([S::TopLeft, S::Top, S::TopRight], |x| x.height),
        most([S::BottomLeft, S::Bottom, S::BottomRight], |x| x.height),
    ]
}

/// The set's natural size (spec A6 `ToContent`): the corner columns and
/// rows plus the widest middle-column piece and the tallest middle-row one.
#[must_use]
pub fn natural(set: &NodePieces) -> (i32, i32) {
    use PieceSlot as S;
    let [left, right, top, bottom] = insets(set);
    let most = |slots: [S; 3], f: fn(&NodePiece) -> u16| {
        slots
            .iter()
            .filter_map(|s| set.get(s))
            .map(|x| i32::from(f(x)))
            .max()
            .unwrap_or(0)
    };
    (
        left + right + most([S::Top, S::Middle, S::Bottom], |x| x.width),
        top + bottom + most([S::Left, S::Middle, S::Right], |x| x.height),
    )
}

/// `band`'s rectangle `[x, y, w, h]` in a `w × h` box cut by `insets`.
#[must_use]
pub fn band_rect(
    band: Band,
    [left, right, top, bottom]: [i32; 4],
    width: i32,
    height: i32,
) -> [i32; 4] {
    match band {
        Band::Box => [0, 0, width, height],
        Band::Row => [left, 0, width - left - right, height],
        Band::Column => [0, top, width, height - top - bottom],
        Band::Middle => [left, top, width - left - right, height - top - bottom],
    }
}

/// One drawn copy of a piece: its slot, where it lands in the box
/// `[x, y, w, h]`, and the source offset of that rectangle in the piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copy {
    pub slot: PieceSlot,
    pub at: [i32; 4],
    pub src: (i32, i32),
}

/// The starts of a piece `len` px long laid in `[start, start + span)`:
/// once, from the far edge when `end`, or repeated from the start when
/// `repeat` (no slot repeats along an axis it is laid from the end of).
fn starts(start: i32, span: i32, len: i32, end: bool, repeat: bool) -> Vec<i32> {
    if span <= 0 || len <= 0 {
        return Vec::new();
    }
    if !repeat {
        return vec![if end { start + span - len } else { start }];
    }
    let mut out = Vec::new();
    let mut at = start;
    while at < start + span {
        out.push(at);
        at += len;
    }
    out
}

/// One piece laid in a box: its slot, its band `[x, y, w, h]`, and the
/// starts of its copies across and down (each copy clipped to the band).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lay {
    pub slot: PieceSlot,
    pub band: [i32; 4],
    pub xs: Vec<i32>,
    pub ys: Vec<i32>,
}

/// How each piece of `p` lays in a `w × h` box.
#[must_use]
pub fn lay(p: &NodePieces, w: i32, h: i32) -> Vec<Lay> {
    let ins = insets(p);
    p.iter()
        .map(|(&slot, piece)| {
            let c = cell(slot);
            let band = band_rect(c.band, ins, w, h);
            Lay {
                slot,
                band,
                xs: starts(
                    band[0],
                    band[2],
                    i32::from(piece.width),
                    c.end.0,
                    c.repeat.0,
                ),
                ys: starts(
                    band[1],
                    band[3],
                    i32::from(piece.height),
                    c.end.1,
                    c.repeat.1,
                ),
            }
        })
        .collect()
}

/// Every copy the set `p` draws in a `w × h` box, each clipped to its band.
#[must_use]
pub fn draw(p: &NodePieces, w: i32, h: i32) -> Vec<Copy> {
    let mut out = Vec::new();
    for l in lay(p, w, h) {
        let Some(piece) = p.get(&l.slot) else {
            continue;
        };
        let (pw, ph) = (i32::from(piece.width), i32::from(piece.height));
        let band = l.band;
        for &y in &l.ys {
            for &x in &l.xs {
                let x0 = x.max(band[0]);
                let y0 = y.max(band[1]);
                let x1 = (x + pw).min(band[0] + band[2]);
                let y1 = (y + ph).min(band[1] + band[3]);
                if x0 < x1 && y0 < y1 {
                    out.push(Copy {
                        slot: l.slot,
                        at: [x0, y0, x1 - x0, y1 - y0],
                        src: (x0 - x, y0 - y),
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "pieces_tests.rs"]
mod tests;
