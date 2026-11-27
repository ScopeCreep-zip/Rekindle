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
#[path = "presence_fmt_tests.rs"]
mod tests;
