//! Plan SK15: a target box cut short of 24 px on one side takes its 24 px
//! from the free room on its other side, or from a neighbour's share of the
//! gap that the neighbour can spare (used by `target::targets`, after the
//! cuts at the neighbours and the kept boxes, before the served stack).
//!
//! [`super::target`]'s cut splits the gap between two arts and keeps the
//! wanted box centred on the art, so an art with room on its far side
//! still came out short: SpaceCowboy's `Logo` (16 x 16, its `HelpMenu`
//! 6 px below, a free title-bar row 4 px above) was served 22 x 23 though
//! a 24 x 24 box lies clear of every other target 1 px higher; Simple-X's
//! `StatusEdit` (14 px tall) lost 1 px above to its button row with 6 px of
//! window free below it. WCAG 2.2 SC 2.5.8 measures the target, not its
//! centring, so a 24 px box beside the art meets it as well as one centred
//! on it, and the art never moves.
//!
//! - Each candidate meeting neither 24 x 24 nor the spacing exception, in
//!   order, short of 24 px on an axis, grows on that
//!   axis into free room: in the window, meeting no neighbour's target box
//!   and none of its kept boxes (the views and every served text box it
//!   is not kin to). It takes the side with more room first.
//! - Still short, it takes the rest from one neighbour alone on a side
//!   when the strip it needs meets that neighbour's target box and nothing
//!   else, never the neighbour's art, and the neighbour, grown on its far
//!   side into free room, ends no shorter on that axis than 24 px or than
//!   it was (Onyx's `Logo` 2 px over its `StatusEdit`: the status field's
//!   box gives the 1 px and takes 1 px of free room below its art).
//! - The growth stands only when the candidate ends 24 x 24 and no other
//!   candidate that met SC 2.5.8 stops meeting it; else nothing changes.
//! - Two target boxes still never overlap: every grown strip is free of
//!   every other box.

use super::target::meet;
use super::validate::{overlap, HIT};

/// The strip `d` px deep beside `g` on `axis`, after it or before it.
fn strip(g: [i32; 4], axis: usize, after: bool, d: i32) -> [i32; 4] {
    let mut s = g;
    s[axis] = if after {
        g[axis] + g[axis + 2]
    } else {
        g[axis] - d
    };
    s[axis + 2] = d;
    s
}

/// `g` grown `d` px on `axis`, after it or before it.
fn grow(mut g: [i32; 4], axis: usize, after: bool, d: i32) -> [i32; 4] {
    if !after {
        g[axis] -= d;
    }
    g[axis + 2] += d;
    g
}

fn inside(s: [i32; 4], (w, h): (i32, i32)) -> bool {
    s[0] >= 0 && s[1] >= 0 && s[0] + s[2] <= w && s[1] + s[3] <= h
}

/// Whether candidate `j` meets the spacing exception on the box `own` with
/// the boxes `area`: `spaced(j, own, area)`.
pub(super) type SpacingTest<'s> = dyn Fn(usize, [i32; 4], &[[i32; 4]]) -> bool + 's;

/// The boxes a target may not grow over, as [`settle`] reads them.
struct GrowLimits<'a> {
    /// Every candidate's art box.
    arts: &'a [[i32; 4]],
    /// The candidates each candidate keeps clear of (its neighbours).
    near: &'a dyn Fn(usize, usize) -> bool,
    /// Each candidate's kept boxes.
    walls: &'a [Vec<[i32; 4]>],
    window: (i32, i32),
}

impl GrowLimits<'_> {
    /// Whether the strip `s` beside candidate `i` is free, `except` one
    /// neighbour's box.
    fn free(&self, area: &[[i32; 4]], i: usize, s: [i32; 4], except: Option<usize>) -> bool {
        inside(s, self.window)
            && !self.walls[i].iter().any(|w| overlap(s, *w))
            && (0..area.len())
                .filter(|&j| Some(j) != except && (self.near)(i, j))
                .all(|j| !overlap(s, area[j]))
    }

    /// How many px, up to `need`, candidate `i` can grow on `axis`, after
    /// it or before it, into free room.
    fn depth(&self, area: &[[i32; 4]], i: usize, axis: usize, after: bool, need: i32) -> i32 {
        (1..=need)
            .take_while(|&d| self.free(area, i, strip(area[i], axis, after, d), None))
            .last()
            .unwrap_or(0)
    }

    /// Grows candidate `i` on `axis` into free room toward 24 px.
    fn regrow(&self, area: &mut [[i32; 4]], i: usize, axis: usize) {
        let need = HIT - area[i][axis + 2];
        if need <= 0 {
            return;
        }
        let (a, b) = (
            self.depth(area, i, axis, true, need),
            self.depth(area, i, axis, false, need),
        );
        let first = a >= b;
        let d = self.depth(area, i, axis, first, need);
        area[i] = grow(area[i], axis, first, d);
        let rest = HIT - area[i][axis + 2];
        let d = self.depth(area, i, axis, !first, rest.max(0));
        area[i] = grow(area[i], axis, !first, d);
    }

    /// Whether neighbour `j` can give candidate `i` the strip `band` on
    /// `axis`: the strip is free but for `j`'s box, and meets `j`'s art only
    /// where `j`'s art is wide enough to keep 24 px clear of it and the
    /// strip lies on its edge (plan E7.7 TAB1: Symbiosis's `StatusChanger`
    /// abuts its 322 px `Status` text, 5 px from the window's edge once the
    /// strip is given up; the text's last pixel column is its to press). The
    /// text box `j` draws is `j`'s own and no wall to `i` there.
    fn spares(&self, area: &[[i32; 4]], i: usize, j: usize, band: [i32; 4], axis: usize) -> bool {
        let art = self.arts[j];
        let over = meet(band, art);
        let edge = |hit: [i32; 4]| {
            hit[axis] == art[axis] || hit[axis] + hit[axis + 2] == art[axis] + art[axis + 2]
        };
        if over.is_some_and(|hit| art[axis + 2] - hit[axis + 2] < 2 * HIT || !edge(hit)) {
            return false;
        }
        let walls = |wall: &[i32; 4]| over.is_none() || *wall != art;
        inside(band, self.window)
            && !self.walls[i]
                .iter()
                .filter(|wall| walls(wall))
                .any(|wall| overlap(band, *wall))
            && (0..area.len())
                .filter(|&other| other != j && (self.near)(i, other))
                .all(|other| !overlap(band, area[other]))
    }

    /// Takes what candidate `i` still lacks on `axis` from one neighbour's
    /// box on a side (see the module doc).
    fn borrow(&self, area: &mut [[i32; 4]], i: usize, axis: usize) {
        let need = HIT - area[i][axis + 2];
        if need <= 0 {
            return;
        }
        for after in [true, false] {
            let band = strip(area[i], axis, after, need);
            let owners: Vec<usize> = (0..area.len())
                .filter(|&j| (self.near)(i, j) && overlap(band, area[j]))
                .collect();
            let [j] = owners[..] else { continue };
            if !self.spares(area, i, j, band, axis) {
                continue;
            }
            let was = area[j][axis + 2];
            let mut trial = area.to_vec();
            let old = trial[j];
            trial[j] = if after {
                let start = band[axis] + band[axis + 2];
                let mut cut = old;
                cut[axis] = start.max(old[axis]);
                cut[axis + 2] = old[axis] + old[axis + 2] - cut[axis];
                cut
            } else {
                let mut cut = old;
                cut[axis + 2] = band[axis].min(old[axis] + old[axis + 2]) - old[axis];
                cut
            };
            trial[i] = grow(trial[i], axis, after, need);
            let lack = (HIT - trial[j][axis + 2]).max(0);
            let depth = self.depth(&trial, j, axis, after, lack);
            trial[j] = grow(trial[j], axis, after, depth);
            if trial[j][axis + 2] >= was.min(HIT) {
                area.copy_from_slice(&trial);
                return;
            }
        }
    }
}

/// Grows every candidate's target box that meets neither 24 x 24 nor the
/// spacing exception (a candidate already meeting SC 2.5.8 keeps its box),
/// in candidate order (see the module doc). The candidates' art boxes are
/// `arts`; `near` names the neighbours each keeps clear of; candidate `i`
/// keeps clear of each box of `kept` (id, box, views) whose id `keeps(i,
/// id)` accepts; `spaced(j, own, area)` is the spacing exception for
/// candidate `j` on the box `own` ([`SpacingTest`]). The growth stands only when the
/// candidate ends 24 x 24 and every candidate that met SC 2.5.8 still does;
/// else every box stays as it was.
pub(super) fn settle(
    area: &mut [[i32; 4]],
    arts: &[[i32; 4]],
    near: &dyn Fn(usize, usize) -> bool,
    kept: &[(&str, [i32; 4], bool)],
    keeps: &dyn Fn(usize, &str) -> bool,
    window: (i32, i32),
    spaced: &SpacingTest<'_>,
) {
    let walls: Vec<Vec<[i32; 4]>> = (0..arts.len())
        .map(|i| {
            kept.iter()
                .filter(|(id, ..)| keeps(i, id))
                .map(|(_, o, _)| *o)
                .collect()
        })
        .collect();
    let limits = GrowLimits {
        arts,
        near,
        walls: &walls,
        window,
    };
    // Whether candidate `j` meets SC 2.5.8 with the boxes `a`: 24 x 24, or
    // the spacing exception on its box or on its art.
    let meets = |j: usize, a: &[[i32; 4]]| {
        (a[j][2] >= HIT && a[j][3] >= HIT) || spaced(j, a[j], a) || spaced(j, arts[j], a)
    };
    for i in 0..area.len() {
        if meets(i, area) {
            continue;
        }
        let was = area.to_vec();
        for axis in [0, 1] {
            limits.regrow(area, i, axis);
            limits.borrow(area, i, axis);
        }
        let whole = area[i][2] >= HIT && area[i][3] >= HIT;
        if !whole || (0..area.len()).any(|j| j != i && meets(j, &was) && !meets(j, area)) {
            area.copy_from_slice(&was);
        }
    }
}
