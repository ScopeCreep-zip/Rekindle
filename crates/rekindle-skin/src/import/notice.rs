//! What every importer's `NOTICE` shares: colours as hex, ratios as
//! `n.nn:1`, and the walk's changes, each with its original and new colour
//! and measured ratios.

use std::fmt::Write as _;

use super::walk::Change;

/// `#rrggbb`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// A ratio ×100 as `n.nn:1`.
pub fn ratio(x100: u32) -> String {
    format!("{}.{:02}:1", x100 / 100, x100 % 100)
}

/// One line per change, under `heading`.
pub fn changes(s: &mut String, heading: &str, changes: &[Change]) {
    let _ = writeln!(s, "\n{heading} ({} made)", changes.len());
    for c in changes {
        let after = c.after_x100.map_or_else(|| "passes".to_owned(), ratio);
        let _ = writeln!(
            s,
            "  {}: {} -> {} ({} {} on {}: {} -> {}, required {})",
            c.token,
            hex(c.original),
            hex(c.new),
            c.rule,
            c.fg,
            c.ground,
            ratio(c.before_x100),
            after,
            ratio(c.required_x100)
        );
    }
}
