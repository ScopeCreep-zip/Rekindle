use rekindle_types::dto::skin::{SkinWindow, WindowSizeHints};

use super::{axis, max_size, MAIN_CEILING};

#[test]
fn the_ceiling_is_raised_to_the_minimum_and_default_and_lowered_to_the_pack_maximum() {
    // Symbiosis: minimum 368 under the 400 ceiling.
    assert_eq!(axis(368, Some(400), None, None), Some(400));
    // Total Chaos: minimum 470 over the ceiling, so the window is fixed.
    assert_eq!(axis(470, Some(400), None, None), Some(470));
    // A pack default above the ceiling raises it.
    assert_eq!(axis(300, Some(400), Some(440), None), Some(440));
    // The pack's own maximum lowers it, never below the minimum.
    assert_eq!(axis(300, Some(400), Some(440), Some(430)), Some(430));
    assert_eq!(axis(500, None, None, Some(450)), Some(500));
    // No ceiling and no pack maximum: unlimited.
    assert_eq!(axis(380, None, None, None), None);
}

#[test]
fn only_the_buddy_list_has_rekindles_ceiling() {
    let hints = WindowSizeHints::default();
    assert_eq!(
        max_size(SkinWindow::Main, (368, 650), &hints),
        (Some(MAIN_CEILING.0), Some(MAIN_CEILING.1))
    );
    assert_eq!(max_size(SkinWindow::Chat, (384, 400), &hints), (None, None));
}
