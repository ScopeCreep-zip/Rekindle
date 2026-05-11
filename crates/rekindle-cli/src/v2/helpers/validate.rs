//! Input validation for user-provided names and identifiers.

/// Characters that are invisible or can spoof visual appearance.
/// Includes zero-width characters, directional overrides, and Unicode format chars.
fn is_dangerous_unicode(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' | // zero-width space, joiners, LTR/RTL marks
        '\u{2028}'..='\u{2029}' | // line/paragraph separators
        '\u{202A}'..='\u{202E}' | // directional formatting (LRE, RLE, PDF, LRO, RLO)
        '\u{2060}'..='\u{2064}' | // word joiner, invisible times/separator/plus
        '\u{2066}'..='\u{2069}' | // directional isolates
        '\u{FEFF}'              | // BOM / zero-width no-break space
        '\u{FFF9}'..='\u{FFFB}'   // interlinear annotation anchors
    )
}

/// Validate a display name: 1-64 chars, no control/dangerous chars, trimmed.
/// Uses char::is_whitespace for Unicode-aware trimming (handles U+3000 ideographic space).
pub fn validate_display_name(name: &str) -> anyhow::Result<String> {
    let trimmed: String = name.trim_matches(|c: char| c.is_whitespace()).to_string();
    if trimmed.is_empty() {
        anyhow::bail!("display name cannot be empty");
    }
    if trimmed.len() > 64 {
        anyhow::bail!("display name too long ({} chars, max 64)", trimmed.len());
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("display name cannot contain control characters");
    }
    if trimmed.chars().any(is_dangerous_unicode) {
        anyhow::bail!("display name cannot contain invisible or directional override characters");
    }
    Ok(trimmed)
}

/// Validate a community or channel name: 1-100 chars, no control/dangerous chars, trimmed.
pub fn validate_name(name: &str, label: &str) -> anyhow::Result<String> {
    let trimmed: String = name.trim_matches(|c: char| c.is_whitespace()).to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{label} name cannot be empty");
    }
    if trimmed.len() > 100 {
        anyhow::bail!("{label} name too long ({} chars, max 100)", trimmed.len());
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("{label} name cannot contain control characters");
    }
    if trimmed.chars().any(is_dangerous_unicode) {
        anyhow::bail!("{label} name cannot contain invisible or directional override characters");
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_trims_ascii() {
        assert_eq!(validate_display_name("  alice  ").unwrap(), "alice");
    }

    #[test]
    fn display_name_trims_unicode_whitespace() {
        // U+3000 = ideographic space, U+2003 = em space
        assert_eq!(validate_display_name("\u{3000}alice\u{2003}").unwrap(), "alice");
    }

    #[test]
    fn display_name_rejects_empty() {
        assert!(validate_display_name("").is_err());
        assert!(validate_display_name("   ").is_err());
        // Unicode whitespace only
        assert!(validate_display_name("\u{3000}\u{2003}").is_err());
    }

    #[test]
    fn display_name_rejects_long() {
        assert!(validate_display_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn display_name_accepts_max() {
        assert!(validate_display_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn display_name_rejects_control() {
        assert!(validate_display_name("hello\x00world").is_err());
        assert!(validate_display_name("hello\x1bworld").is_err());
    }

    #[test]
    fn display_name_rejects_zero_width() {
        assert!(validate_display_name("alice\u{200B}bob").is_err());
    }

    #[test]
    fn display_name_rejects_rtl_override() {
        assert!(validate_display_name("alice\u{202E}bob").is_err());
    }

    #[test]
    fn display_name_accepts_unicode() {
        assert_eq!(validate_display_name("日本語").unwrap(), "日本語");
        assert_eq!(validate_display_name("🔥 fire").unwrap(), "🔥 fire");
    }

    #[test]
    fn name_rejects_empty() {
        assert!(validate_name("", "Channel").is_err());
    }

    #[test]
    fn name_rejects_long() {
        assert!(validate_name(&"a".repeat(101), "Community").is_err());
    }

    #[test]
    fn name_accepts_max() {
        assert!(validate_name(&"a".repeat(100), "Community").is_ok());
    }

    #[test]
    fn name_trims() {
        assert_eq!(validate_name("  general  ", "Channel").unwrap(), "general");
    }

    #[test]
    fn name_rejects_bom() {
        assert!(validate_name("\u{FEFF}general", "Channel").is_err());
    }
}
