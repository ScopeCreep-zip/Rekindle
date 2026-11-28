//! The `NOTICE` beside an imported Pidgin pack: provenance, licence
//! status, every theme read, what every colour, icon and sound became,
//! every dropped item with its reason, and every change the walk made.

use std::fmt::Write as _;

use super::super::notice::{changes, hex};
use super::super::walk::Change;
use super::{Read, PIDGIN_FORMAT};

/// The NOTICE text, deterministic for one input.
#[must_use]
pub fn render(id: &str, read: &Read, source: &str, made: &[Change]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{id}: imported by `cargo xtask import-skin --format pidgin`\n\
         (plan E7.7.3 item 4; rekindle-skin::import::pidgin; source.format {PIDGIN_FORMAT}).\n\n\
         Provenance\n  theme tree BLAKE3: {source}\n\n\
         Licence\n  The theme's palette, icons and sounds are third-party work.\n  \
         Rekindle's MIT OR Apache-2.0 licence does not cover this pack.\n"
    );
    let _ = writeln!(s, "Themes ({} read)", read.themes.len());
    for (kind, name) in &read.themes {
        let _ = writeln!(s, "  purple/{kind}/theme.xml name=\"{name}\"");
    }
    let _ = writeln!(s, "\nColours ({} mapped)", read.colours.len());
    for c in &read.colours {
        let _ = writeln!(s, "  {} {} -> token {}", c.source, hex(c.value), c.token);
    }
    for (title, entries) in [("Status icons", &read.icons), ("Sounds", &read.sounds)] {
        let _ = writeln!(s, "\n{title} ({} mapped, re-encoded)", entries.len());
        for (slot, (path, _, item)) in entries {
            let _ = writeln!(s, "  {item} -> {slot} ({path})");
        }
    }
    let _ = writeln!(s, "\nDropped ({})", read.dropped.len());
    for d in &read.dropped {
        let _ = writeln!(s, "  {} -> {}", d.item, d.why);
    }
    changes(
        &mut s,
        "Contrast changes (minimal Oklab-lightness walk)",
        made,
    );
    s
}
