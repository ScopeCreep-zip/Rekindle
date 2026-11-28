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

/// One axis's wanted size: the pack's default, else the kind's initial
/// size, at least `minimum` and at most the served `maximum`.
fn wanted(initial: f64, minimum: u16, default: Option<u16>, maximum: Option<u16>) -> f64 {
    let preferred = default.map_or(initial, f64::from).max(f64::from(minimum));
    maximum.map_or(preferred, |max| preferred.min(f64::from(max)))
}

impl Sizing {
    pub fn for_frame(frame: SkinWindow, layout: Option<&WindowLayout>) -> Self {
        let base = super::super::size(skin::kind_of(frame));
        let minimum = layout
            .and_then(rekindle_skin::shape::min_window)
            .unwrap_or(base.min);
        let hints = layout.map_or_else(WindowSizeHints::default, |layout| layout.size_hints);
        // The same maximum the Host serves on the window's shell.
        let (max_w, max_h) = rekindle_skin::shape::max_size(frame, minimum, &hints);
        Self {
            wanted: (
                wanted(base.inner.0, minimum.0, hints.default_width, max_w),
                wanted(base.inner.1, minimum.1, hints.default_height, max_h),
            ),
            minimum: (f64::from(minimum.0), f64::from(minimum.1)),
            maximum: (max_w.map(f64::from), max_h.map(f64::from)),
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
