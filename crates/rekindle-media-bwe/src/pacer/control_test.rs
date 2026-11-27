use super::*;

#[test]
fn padding_enabled_with_active_media() {
    let estimate = Bitrate::kbps(1_000);

    let r = PacerControl::calculate(true, estimate, false);

    assert_eq!(r.padding_rate, PADDING_TARGET);
}

#[test]
fn no_padding_without_active_media() {
    let estimate = Bitrate::kbps(1_000);

    let r = PacerControl::calculate(false, estimate, false);

    assert_eq!(r.padding_rate, Bitrate::ZERO);
}

#[test]
fn overuse_suppresses_padding() {
    let estimate = Bitrate::mbps(40);

    let r = PacerControl::calculate(true, estimate, true);
    assert_eq!(r.padding_rate, Bitrate::ZERO);
}
