//! Root sizing is imported in canvas coordinates, independently of art and popups.

use rekindle_types::dto::skin::{Insets, WindowSizeHints};

use super::import::bad;
use crate::import::{tree::Node, ImportError};

/// Translate the root's authored canvas dimensions by the same tree
/// outsets used for its minimum. Frame outsets already inside that canvas
/// (such as StealthX's rail) are not added again.
pub(super) fn root_size_hints(
    root: Option<&Node>,
    out: Insets,
) -> Result<WindowSizeHints, ImportError> {
    let read = |key, a: u16, b: u16| {
        root.and_then(|node| node.attr_ci(key))
            .map(|value| {
                value
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|value| *value > 0)
                    .and_then(|value| value.checked_add(a)?.checked_add(b))
                    .ok_or_else(|| bad(format!("invalid root {key}: {value}")))
            })
            .transpose()
    };
    Ok(WindowSizeHints {
        default_width: read("DefaultWidth", out.left, out.right)?,
        default_height: read("DefaultHeight", out.top, out.bottom)?,
        max_width: read("MaxWidth", out.left, out.right)?,
        max_height: read("MaxHeight", out.top, out.bottom)?,
    })
}

#[cfg(test)]
#[path = "import_sizing_tests.rs"]
mod tests;
