//! The size request and coherent limits for a framed window. A resize or
//! scale notification samples the current allocation without reissuing it.

use rekindle_types::dto::skin::{SkinWindow, WindowLayout, WindowSizeHints};
use tauri::{LogicalUnit, WindowSizeConstraints};

use super::skin;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sizing {
    pub wanted: (f64, f64),
    pub minimum: (f64, f64),
    pub maximum: (Option<f64>, Option<f64>),
}

fn axis(
    initial: f64,
    minimum: f64,
    cap: Option<f64>,
    default: Option<u16>,
    maximum: Option<u16>,
) -> (f64, Option<f64>) {
    let preferred = default.map_or(initial, f64::from).max(minimum);
    let cap = cap.map(|cap| cap.max(minimum).max(default.map_or(0.0, f64::from)));
    let supplied = maximum.map(f64::from);
    let max = match (cap, supplied) {
        (Some(a), Some(b)) => Some(a.min(b).max(minimum)),
        (a, b) => a.or(b).map(|limit| limit.max(minimum)),
    };
    (max.map_or(preferred, |max| preferred.min(max)), max)
}

impl Sizing {
    pub fn for_frame(frame: SkinWindow, layout: Option<&WindowLayout>) -> Self {
        let base = super::super::size(skin::kind_of(frame));
        let minimum = layout
            .and_then(rekindle_skin::shape::min_window)
            .map_or(base.min, |(w, h)| (f64::from(w), f64::from(h)));
        let hints = layout.map_or_else(WindowSizeHints::default, |layout| layout.size_hints);
        let cap = (frame == SkinWindow::Main).then_some((400.0, 900.0));
        let (width, max_w) = axis(
            base.inner.0,
            minimum.0,
            cap.map(|c| c.0),
            hints.default_width,
            hints.max_width,
        );
        let (height, max_h) = axis(
            base.inner.1,
            minimum.1,
            cap.map(|c| c.1),
            hints.default_height,
            hints.max_height,
        );
        Self {
            wanted: (width, height),
            minimum,
            maximum: (max_w, max_h),
        }
    }

    pub fn constraints(self) -> WindowSizeConstraints {
        let unit = |n| LogicalUnit::new(n).into();
        WindowSizeConstraints {
            min_width: Some(unit(self.minimum.0)),
            min_height: Some(unit(self.minimum.1)),
            max_width: self.maximum.0.map(unit),
            max_height: self.maximum.1.map(unit),
        }
    }

    pub fn fits(self, size: (f64, f64)) -> bool {
        size.0 >= self.minimum.0 && size.1 >= self.minimum.1
    }
}

#[cfg(test)]
#[path = "sizing_tests.rs"]
mod tests;
