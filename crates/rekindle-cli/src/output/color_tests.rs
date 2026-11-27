use super::*;

#[test]
fn no_color_flag_disables() {
    let support = ColorSupport::detect(true);
    assert!(!support.is_enabled());
    assert!(!support.has_true_color());
    assert!(!support.has_256_colors());
}
