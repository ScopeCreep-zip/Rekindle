use super::*;

#[test]
fn prefix_cuts_on_characters() {
    assert_eq!(prefix("abcdef", 3), "abc");
    assert_eq!(prefix("ab", 3), "ab");
    assert_eq!(prefix("héllo wörld", 5), "héllo");
    assert_eq!(prefix("🌍🌍🌍", 2), "🌍🌍");
    assert_eq!(prefix("", 4), "");
}

#[test]
fn abbreviate_keeps_head_and_tail() {
    assert_eq!(abbreviate("0123456789abcdef", 4, 2), "0123…ef");
    assert_eq!(abbreviate("short", 4, 2), "short");
    assert_eq!(abbreviate("ééééééééé", 2, 2), "éé…éé");
}

#[test]
fn sanitize_strips_ansi() {
    assert_eq!(sanitize_for_display("hello\x1b[31mworld"), "helloworld");
}

#[test]
fn sanitize_strips_null() {
    assert_eq!(sanitize_for_display("hello\x00world"), "helloworld");
}

#[test]
fn sanitize_preserves_newline() {
    assert_eq!(sanitize_for_display("a\nb"), "a\nb");
}

#[test]
fn sanitize_preserves_unicode() {
    assert_eq!(sanitize_for_display("hello 🌍"), "hello 🌍");
}

// Escape-sequence coverage moved from the CLI with the function's users.
#[test]
fn sanitize_strips_basic_csi_sequence() {
    // CSI color: ESC [ 31 m → stripped entirely
    assert_eq!(sanitize_for_display("hello\x1b[31mworld"), "helloworld");
}

#[test]
fn sanitize_strips_sgr_reset() {
    // ESC [ 0 m (reset) → stripped
    assert_eq!(sanitize_for_display("a\x1b[0mb"), "ab");
}

#[test]
fn sanitize_strips_cursor_movement() {
    // ESC [ 10 A (cursor up 10) → stripped
    assert_eq!(sanitize_for_display("before\x1b[10A_after"), "before_after");
}

#[test]
fn sanitize_strips_erase_display() {
    // ESC [ 2 J (clear screen) → stripped
    assert_eq!(sanitize_for_display("safe\x1b[2Jtext"), "safetext");
}

#[test]
fn sanitize_strips_osc_title_injection() {
    // OSC title set: ESC ] 0 ; evil BEL → stripped
    assert_eq!(
        sanitize_for_display("before\x1b]0;evil title\x07after"),
        "beforeafter"
    );
}

#[test]
fn sanitize_strips_osc_with_st_terminator() {
    // OSC terminated by ESC \ instead of BEL
    assert_eq!(sanitize_for_display("a\x1b]0;payload\x1b\\b"), "ab");
}

#[test]
fn sanitize_strips_nested_escape() {
    // Nested: ESC [ ESC [ 31m → both ESC sequences consumed
    assert_eq!(sanitize_for_display("x\x1b[\x1b[31my"), "xy");
}

#[test]
fn sanitize_strips_incomplete_csi() {
    // Incomplete CSI: ESC [ with no final byte → ESC consumed, [ left
    // The [ is a printable char so it stays. The CSI parser stops
    // when it hits end-of-input without a final byte.
    let result = sanitize_for_display("end\x1b[");
    // ESC is consumed. '[' is not a parameter/intermediate byte range
    // (0x20-0x3F), nor a final byte (0x40-0x7E) — actually '[' is 0x5B
    // which IS in the final byte range. So the parser consumes '[' as
    // the final byte. Result: "end"
    assert_eq!(result, "end");
}

#[test]
fn sanitize_strips_null_bytes() {
    assert_eq!(sanitize_for_display("hello\x00world"), "helloworld");
}

#[test]
fn sanitize_strips_bell() {
    assert_eq!(sanitize_for_display("ding\x07dong"), "dingdong");
}

#[test]
fn sanitize_strips_backspace() {
    // BS (0x08) can overwrite previous chars on some terminals
    assert_eq!(sanitize_for_display("abc\x08def"), "abcdef");
}

#[test]
fn sanitize_keeps_embedded_newline() {
    assert_eq!(sanitize_for_display("line1\nline2"), "line1\nline2");
}

#[test]
fn sanitize_preserves_tab() {
    assert_eq!(sanitize_for_display("col1\tcol2"), "col1\tcol2");
}

#[test]
fn sanitize_keeps_multibyte_text() {
    assert_eq!(sanitize_for_display("hello 🌍 世界"), "hello 🌍 世界");
}

#[test]
fn sanitize_strips_multiple_sequences() {
    // Multiple CSI sequences in one string
    assert_eq!(
        sanitize_for_display("\x1b[1m\x1b[31mbold red\x1b[0m normal"),
        "bold red normal"
    );
}

#[test]
fn sanitize_empty_string() {
    assert_eq!(sanitize_for_display(""), "");
}

#[test]
fn sanitize_only_escape_sequence() {
    assert_eq!(sanitize_for_display("\x1b[31m"), "");
}

// ── Sanitization: Unicode adversarial ──────────────────────────

#[test]
fn sanitize_preserves_rtl_override() {
    // U+202E RIGHT-TO-LEFT OVERRIDE is not a C0 control char,
    // it's a Unicode formatting character. Our sanitizer strips
    // C0 controls (0x00-0x1F except \n\t) and ANSI escapes.
    // RTL override is U+202E which is not in C0 range.
    // This is intentional — full Unicode normalization is a
    // separate concern from terminal escape injection.
    let input = "hello\u{202E}dlrow";
    let result = sanitize_for_display(input);
    assert!(result.contains('\u{202E}'));
}

#[test]
fn sanitize_preserves_zero_width_joiner() {
    // ZWJ (U+200D) is not a control char — it's used in emoji sequences
    let input = "👨\u{200D}👩\u{200D}👧";
    let result = sanitize_for_display(input);
    assert_eq!(result, input);
}

// ── Display name validation ────────────────────────────────────
