use super::*;

#[test]
fn the_table_is_sorted_and_disjoint() {
    for w in CF_RANGES.windows(2) {
        assert!(w[0].0 <= w[0].1 && w[0].1 < w[1].0, "{w:?}");
    }
}

#[test]
fn bidi_controls_and_invisibles_are_hidden_formats() {
    for c in [
        '\u{202A}',
        '\u{202B}',
        '\u{202C}',
        '\u{202D}',
        '\u{202E}',
        '\u{2066}',
        '\u{2067}',
        '\u{2068}',
        '\u{2069}',
        '\u{200E}',
        '\u{200F}',
        '\u{061C}',
        '\u{200B}',
        '\u{2060}',
        '\u{FEFF}',
        '\u{00AD}',
        '\u{E0041}',
    ] {
        assert!(is_hidden_format(c), "U+{:04X}", u32::from(c));
    }
}

#[test]
fn joiners_and_ordinary_text_are_allowed() {
    assert!(is_format('\u{200D}'));
    assert!(!is_hidden_format('\u{200D}'));
    assert!(!is_hidden_format('\u{200C}'));
    for c in "Ann ann 日本 👍 ❤\u{FE0F} é\n".chars() {
        assert!(!is_hidden_format(c), "{c:?}");
    }
}
