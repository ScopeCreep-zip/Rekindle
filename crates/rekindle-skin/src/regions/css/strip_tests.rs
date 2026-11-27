use rekindle_types::dto::skin::Insets;

use super::*;

#[test]
fn a_three_slice_strip_keeps_its_caps_aspect_at_the_chrome_height() {
    let fit = RegionFit::NineSlice(Insets {
        top: 0,
        right: 30,
        bottom: 0,
        left: 20,
    });
    assert_eq!(
        value("x.png", fit, 51).as_deref(),
        Some(
            "url(\"x.png\") 0 30 0 20 fill / 0 calc(var(--chrome-height) * 30 / 51) 0 \
                 calc(var(--chrome-height) * 20 / 51) / 0 stretch"
        )
    );
    assert_eq!(value("x.png", RegionFit::Stretch, 51), None);
    let framed = RegionFit::NineSlice(Insets {
        top: 2,
        right: 3,
        bottom: 2,
        left: 3,
    });
    assert_eq!(value("x.png", framed, 51), None);
}
