//! The `NOTICE` beside an imported Mumble or TeamSpeak 3 pack:
//! provenance, licence status, the files the palette came from, what every
//! colour and icon became, every reference the SVG resolver refused, every
//! dropped item with its reason, and every change the walk made.

use std::fmt::Write as _;

use super::super::notice::{changes, hex};
use super::super::walk::Change;
use super::{Front, Read};

/// The NOTICE text, deterministic for one input.
#[must_use]
pub fn render(id: &str, read: &Read, source: &str, made: &[Change]) -> String {
    let (command, what) = match read.front {
        Front::Mumble => ("mumble", "Mumble theme"),
        Front::TeamSpeak3 => ("teamspeak3", "TeamSpeak 3 style"),
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{id}: imported by `cargo xtask import-skin --format {command}`\n\
         (plan E7.7.3 item 5; rekindle-skin::import::qt; source.format {}).\n\n\
         Provenance\n  {what} tree BLAKE3: {source}\n\n\
         Licence\n  The {what}'s palette and images are third-party work.\n  \
         Rekindle's MIT OR Apache-2.0 licence does not cover this pack.\n",
        read.front.format()
    );
    let _ = writeln!(s, "Read ({})", read.files.len());
    for f in &read.files {
        let _ = writeln!(s, "  {f}");
    }
    let _ = writeln!(s, "\nColours ({} mapped)", read.colours.len());
    for c in &read.colours {
        let _ = writeln!(s, "  {} {} -> token {}", c.source, hex(c.value), c.token);
    }
    let _ = writeln!(
        s,
        "\nStatus icons ({} mapped, re-encoded PNG)",
        read.icons.len()
    );
    for (slot, (path, _, file)) in &read.icons {
        let _ = writeln!(s, "  {file} -> {slot} ({path})");
    }
    let images: Vec<_> = read
        .regions
        .iter()
        .flat_map(|(slot, images)| images.iter().map(move |(state, i)| (slot, state, i)))
        .collect();
    let _ = writeln!(
        s,
        "\nBackgrounds ({} mapped, tiled, re-encoded PNG)",
        images.len()
    );
    for (slot, state, (path, _, file)) in images {
        let _ = writeln!(s, "  {file} -> {slot} {} ({path})", state.key());
    }
    let _ = writeln!(
        s,
        "\nSVG references refused ({}; never read)",
        read.refused.len()
    );
    for (file, href) in &read.refused {
        let _ = writeln!(s, "  {file}: {href}");
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
