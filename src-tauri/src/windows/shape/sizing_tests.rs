use super::*;

#[test]
fn size_policy_keeps_legacy_defaults_and_honours_per_axis_pack_limits() {
    assert_eq!(
        axis(320.0, 340.0, Some(400.0), None, None),
        (340.0, Some(400.0))
    );
    assert_eq!(
        axis(320.0, 400.0, Some(400.0), None, None),
        (400.0, Some(400.0))
    );
    assert_eq!(
        axis(650.0, 555.0, Some(900.0), Some(555), None),
        (555.0, Some(900.0))
    );
    assert_eq!(
        axis(320.0, 470.0, Some(400.0), None, None),
        (470.0, Some(470.0))
    );
    assert_eq!(
        axis(320.0, 300.0, Some(400.0), Some(440), Some(430)),
        (430.0, Some(430.0))
    );
    assert_eq!(
        axis(650.0, 500.0, None, None, Some(600)),
        (600.0, Some(600.0))
    );
}
