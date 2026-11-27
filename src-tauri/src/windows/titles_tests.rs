use super::*;

#[test]
fn title_text_strips_controls_and_bidi() {
    assert_eq!(title_text("  Alice\u{7}  "), "Alice");
    assert_eq!(title_text("evil\u{202E}gpj.exe"), "evilgpj.exe");
    assert_eq!(title_text("a\u{2066}b\u{200F}c\nd"), "abcd");
    assert_eq!(title_text(&"é".repeat(100)).chars().count(), MAX_TITLE_NAME);
    assert_eq!(title_text("\u{202E}\u{0}"), "");
}

#[test]
fn short_key_is_prefix() {
    assert_eq!(short_key("3f1a9c0b7e2d4f6a"), "3f1a9c0b…");
}
