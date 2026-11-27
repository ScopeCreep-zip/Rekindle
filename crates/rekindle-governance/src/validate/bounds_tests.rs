use super::*;

#[test]
fn edges() {
    assert!(slowmode_ok(0) && slowmode_ok(MAX_SLOWMODE_SECONDS));
    assert!(!slowmode_ok(MAX_SLOWMODE_SECONDS + 1));
    assert!(!timeout_ok(0) && timeout_ok(1) && timeout_ok(MAX_TIMEOUT_SECONDS));
    assert!(!timeout_ok(MAX_TIMEOUT_SECONDS + 1));
    assert!(description_ok(&"é".repeat(MAX_DESCRIPTION_LEN)));
    assert!(!description_ok(&"é".repeat(MAX_DESCRIPTION_LEN + 1)));
    assert!(description_ok("line\n\ttab"));
    assert!(!description_ok("bell\u{7}"));
    assert!(!description_ok("bidi\u{202e}"));
}
