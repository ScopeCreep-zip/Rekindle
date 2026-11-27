//! Project device pixels into the GTK root's logical coordinates. The
//! input region and the visible rectangle are derived from the same union.

use std::collections::BTreeMap;

use super::geometry::{to_logical, LogicalRect, Silhouette, MAX_RECTS};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct Margins {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Margins {
    pub fn canvas(self, visible: (i32, i32)) -> (i32, i32) {
        (
            visible.0 + self.left + self.right,
            visible.1 + self.top + self.bottom,
        )
    }

    pub fn visible(self, canvas: (i32, i32)) -> (i32, i32) {
        (
            canvas.0 - self.left - self.right,
            canvas.1 - self.top - self.bottom,
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct LogicalShape {
    pub rects: Vec<LogicalRect>,
    pub bounds: LogicalRect,
    pub margins: Margins,
}

/// Non-overlapping horizontal runs for the active vertical band.
fn runs(edges: &BTreeMap<i32, i32>) -> Vec<(i32, i32)> {
    let mut depth = 0;
    let mut start = 0;
    let mut runs = Vec::new();
    for (&x, &delta) in edges {
        if depth == 0 {
            start = x;
        }
        depth += delta;
        if depth == 0 {
            runs.push((start, x - start));
        }
    }
    runs
}

fn bands(events: BTreeMap<i32, Vec<(i32, i32)>>) -> Option<Vec<LogicalRect>> {
    let mut active = BTreeMap::new();
    let mut prior = Vec::new();
    let mut start = 0;
    let mut rects = Vec::new();
    for (y, changes) in events {
        for (x, change) in changes {
            let count = active.entry(x).or_insert(0);
            *count += change;
            if *count == 0 {
                active.remove(&x);
            }
        }
        let band = runs(&active);
        if band == prior {
            continue;
        }
        if rects.len() + prior.len() > MAX_RECTS {
            return None;
        }
        rects.extend(prior.into_iter().map(|(x, w)| (x, start, w, y - start)));
        prior = band;
        start = y;
    }
    Some(rects)
}

impl LogicalShape {
    pub fn project(silhouette: &Silhouette, canvas: (i32, i32), scale: i32) -> Option<Self> {
        if canvas.0 <= 0 || canvas.1 <= 0 || scale <= 0 {
            return None;
        }
        let mut events = BTreeMap::<i32, Vec<(i32, i32)>>::new();
        let (mut left, mut top, mut right, mut bottom) = (canvas.0, canvas.1, 0, 0);
        for (x, y, w, h) in to_logical(silhouette, scale) {
            let x1 = (x + w).min(canvas.0);
            let y1 = (y + h).min(canvas.1);
            let x0 = x.max(0);
            let y0 = y.max(0);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            events.entry(y0).or_default().extend([(x0, 1), (x1, -1)]);
            events.entry(y1).or_default().extend([(x0, -1), (x1, 1)]);
            (left, top) = (left.min(x0), top.min(y0));
            (right, bottom) = (right.max(x1), bottom.max(y1));
        }
        if events.is_empty() {
            return None;
        }
        Some(Self {
            rects: bands(events)?,
            bounds: (left, top, right - left, bottom - top),
            margins: Margins {
                left,
                top,
                right: canvas.0 - right,
                bottom: canvas.1 - bottom,
            },
        })
    }
}

#[cfg(test)]
#[path = "logical_tests.rs"]
mod tests;
