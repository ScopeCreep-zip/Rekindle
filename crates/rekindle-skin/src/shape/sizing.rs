//! A window's served maximum (plan SK4): Rekindle's ceiling for its kind,
//! raised to the pack's minimum and default size, lowered to the pack's
//! declared maximum. The desktop shell holds the native window to it and
//! the Host serves it on the window's shell (`SkinShell::max_width`), so
//! the native limit and the served one never disagree (ADR 0020: the
//! policy lives in a tier crate, not in a frontend).

use rekindle_types::dto::skin::{SkinWindow, WindowLayout, WindowSizeHints};

use super::min_window;

/// Rekindle's ceiling for the buddy list (`SkinWindow::Main`), the narrow
/// vertical window: at most 400 x 900 logical px, unless the pack's own
/// minimum or default asks for more.
pub const MAIN_CEILING: (u16, u16) = (400, 900);

fn ceiling(window: SkinWindow) -> Option<(u16, u16)> {
    (window == SkinWindow::Main).then_some(MAIN_CEILING)
}

/// One axis: the ceiling raised to `minimum` and the pack's `default`,
/// then lowered to the pack's `maximum`, never below `minimum`.
fn axis(
    minimum: u16,
    ceiling: Option<u16>,
    default: Option<u16>,
    maximum: Option<u16>,
) -> Option<u16> {
    let ceiling = ceiling.map(|c| c.max(minimum).max(default.unwrap_or(0)));
    match (ceiling, maximum) {
        (Some(a), Some(b)) => Some(a.min(b).max(minimum)),
        (a, b) => a.or(b).map(|limit| limit.max(minimum)),
    }
}

/// `window`'s maximum (width, height) at `minimum` under the pack's
/// `hints`; `None` on an axis with no limit.
#[must_use]
pub fn max_size(
    window: SkinWindow,
    minimum: (u16, u16),
    hints: &WindowSizeHints,
) -> (Option<u16>, Option<u16>) {
    let ceiling = ceiling(window);
    (
        axis(
            minimum.0,
            ceiling.map(|c| c.0),
            hints.default_width,
            hints.max_width,
        ),
        axis(
            minimum.1,
            ceiling.map(|c| c.1),
            hints.default_height,
            hints.max_height,
        ),
    )
}

/// `layout`'s served maximum: [`max_size`] at its [`min_window`].
#[must_use]
pub fn max_window(window: SkinWindow, layout: &WindowLayout) -> Option<(Option<u16>, Option<u16>)> {
    Some(max_size(window, min_window(layout)?, &layout.size_hints))
}

#[cfg(test)]
#[path = "sizing_tests.rs"]
mod tests;
