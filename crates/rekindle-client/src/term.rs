//! Terminal capabilities shared by the text frontends.

use std::io::IsTerminal;

/// Whether Unicode glyphs can be drawn rather than their ASCII stand-ins:
/// not on `TERM=dumb`; yes with a UTF-8 `LANG`, or on any interactive
/// terminal (modern terminals render Unicode without `LANG`).
#[must_use]
pub fn use_unicode() -> bool {
    if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
        return false;
    }
    let lang = std::env::var("LANG").unwrap_or_default();
    lang.contains("UTF") || lang.contains("utf") || std::io::stdout().is_terminal()
}
