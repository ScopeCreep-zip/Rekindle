//! The text rules for strings a skin shows the user: a scalar bound, NFC,
//! and no control or bidi formatting characters (a bidi override in a name
//! reorders what the user reads: Trojan Source, CVE-2021-42574).

use rekindle_types::dto::skin::SkinRule;
use unicode_normalization::is_nfc;

/// What a field allows beyond the common rules.
#[derive(Clone, Copy)]
pub(super) struct TextRule {
    pub max_scalars: usize,
    pub may_be_empty: bool,
    pub line_feeds: bool,
}

/// The Unicode bidi formatting characters (UAX #9 §2): ALM, LRM, RLM, the
/// embeddings and overrides, and the isolates.
pub(crate) fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

pub(super) fn check(field: &str, s: &str, rule: TextRule) -> Result<(), SkinRule> {
    if !rule.may_be_empty && s.trim().is_empty() {
        return Err(SkinRule::EmptyText {
            field: field.to_owned(),
        });
    }
    let scalars = s.chars().count();
    if scalars > rule.max_scalars {
        return Err(SkinRule::TextTooLong {
            field: field.to_owned(),
            scalars: scalars as u64,
            max: rule.max_scalars as u64,
        });
    }
    if !is_nfc(s) {
        return Err(SkinRule::NotNfc {
            field: field.to_owned(),
        });
    }
    let forbidden =
        |c: char| (c.is_control() && !(rule.line_feeds && c == '\n')) || is_bidi_control(c);
    if let Some(c) = s.chars().find(|&c| forbidden(c)) {
        return Err(SkinRule::ForbiddenCharacter {
            field: field.to_owned(),
            codepoint: u32::from(c),
        });
    }
    Ok(())
}
