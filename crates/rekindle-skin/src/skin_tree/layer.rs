//! A tile's draw layer from its component (spec A9): `DrawOnTopmost` and
//! `DrawOnBottommost` lift it out of its siblings' `Z` order.

use crate::import::tree::Node;

/// How far a component's `DrawOnTopmost`/`DrawOnBottommost` (spec A9;
/// `Xfire.exe` UTF-16 strings at 0x2b75a4, 0x2b75c0) lifts its tile out of
/// its siblings' `Z` order: past every pack `Z` (they run -4..100), so
/// Classic's `TheX` (`xfirecomponents.xml:147`, `DrawOnTopmost="true"`)
/// draws above the siblings laid after it.
const TOPMOST: i16 = 1000;

/// The layer a tile's `component` puts it in (spec A9): [`TOPMOST`] above
/// its siblings, its negation below them, else 0.
pub(super) fn layer_of(component: Option<&Node>) -> i16 {
    let on = |k: &str| {
        component
            .and_then(|c| c.attr_ci(k))
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    };
    if on("DrawOnTopmost") {
        TOPMOST
    } else if on("DrawOnBottommost") {
        -TOPMOST
    } else {
        0
    }
}
