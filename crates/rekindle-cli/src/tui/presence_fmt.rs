//! Presence status formatting shared by the peer list and friend list.
//!
//! Both widgets render the same five statuses with the same glyphs,
//! labels and sort order, and each carried its own copy: identical
//! behaviour written two ways (nested `if`/`else` versus inline
//! ternaries), plus a byte-identical capitalize helper under two names
//! (`capitalize_status` / `capitalize_first`). Two widgets showing the
//! same peer in different styles is exactly what a second copy invites.

/// Presence indicator: returns `(glyph, text_label)`.
///
/// Always provides both — colour is applied by the caller via theme
/// tokens. `unicode` selects the glyph set for terminals without good
/// Unicode support.
pub(crate) fn presence_indicator(status: &str, unicode: bool) -> (&'static str, &'static str) {
    match status {
        "online" => (if unicode { "●" } else { "o" }, "[ONLINE]"),
        "away" => (if unicode { "◐" } else { "~" }, "[AWAY]"),
        "busy" => (if unicode { "●" } else { "-" }, "[BUSY]"),
        "offline" => (if unicode { "○" } else { "." }, "[OFFLINE]"),
        _ => (if unicode { "◌" } else { "?" }, "[?]"),
    }
}

/// Sort rank for presence status — lower sorts first.
pub(crate) fn presence_rank(status: &str) -> u8 {
    match status {
        "online" => 0,
        "away" => 1,
        "busy" => 2,
        "offline" => 3,
        _ => 4,
    }
}

/// Capitalize the first letter of a status string.
pub(crate) fn capitalize_status(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_orders_online_first_unknown_last() {
        let mut all = ["offline", "online", "mystery", "busy", "away"];
        all.sort_by_key(|s| presence_rank(s));
        assert_eq!(all, ["online", "away", "busy", "offline", "mystery"]);
    }

    /// Both glyph sets must cover every status, and the label must not
    /// depend on the glyph set — the two widgets previously derived
    /// these separately.
    #[test]
    fn indicator_label_is_independent_of_glyph_set() {
        for status in ["online", "away", "busy", "offline", "???"] {
            let (uni, uni_label) = presence_indicator(status, true);
            let (ascii, ascii_label) = presence_indicator(status, false);
            assert_eq!(uni_label, ascii_label, "label differs for {status}");
            assert!(!uni.is_empty() && !ascii.is_empty());
        }
    }

    #[test]
    fn capitalize_handles_empty_and_unicode() {
        assert_eq!(capitalize_status(""), "");
        assert_eq!(capitalize_status("online"), "Online");
        assert_eq!(capitalize_status("état"), "État");
    }
}
