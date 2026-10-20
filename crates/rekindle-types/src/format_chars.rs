//! Invisible format characters (Unicode general category `Cf`) that text a
//! peer or a user supplies may not carry (plan audit 2026-10-20, G6).
//!
//! `Cf` holds the bidirectional overrides, embeddings and isolates
//! (`U+202A..U+202E`, `U+2066..U+2069`), the directional marks, zero-width
//! spaces, the invisible operators, the byte-order mark and the tag
//! characters. Each renders as nothing, or reorders what follows it, so a
//! name or message holding one can show text that differs from its bytes
//! (Trojan Source, CVE-2021-42574; UTS #39 §5 restriction of default
//! ignorables). A spoofable display field is one of the threat model's
//! refused paths (`docs/security/threat-model.md`, Scope).
//!
//! Two `Cf` characters stay allowed because ordinary text needs them: the
//! zero-width non-joiner and joiner (`U+200C`, `U+200D`), which Persian,
//! the Indic scripts and every emoji ZWJ sequence use (UAX #31 §2.3 allows
//! both in identifiers for the same reason).
//!
//! The table is `Cf` from Unicode 15.1 `DerivedGeneralCategory.txt`; std
//! exposes `Cc` (`char::is_control`) but not `Cf`.

/// Every `Cf` range, inclusive, in code-point order.
const CF_RANGES: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Zero-width non-joiner and joiner: `Cf`, but needed by real text.
const JOINERS: [char; 2] = ['\u{200C}', '\u{200D}'];

/// Whether `c` is a `Cf` format character.
#[must_use]
pub fn is_format(c: char) -> bool {
    let cp = u32::from(c);
    CF_RANGES
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Whether `c` is a format character text may not carry: any `Cf` but the
/// two joiners.
#[must_use]
pub fn is_hidden_format(c: char) -> bool {
    is_format(c) && !JOINERS.contains(&c)
}

#[cfg(test)]
mod tests {
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
}
