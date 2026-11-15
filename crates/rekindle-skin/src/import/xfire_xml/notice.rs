//! The `NOTICE` beside an imported pack: provenance, licence status, what
//! every colour name became, what was dropped, and every change the walk
//! made with its original and new colour and measured ratios.

use std::fmt::Write as _;

use super::super::notice;
use super::super::walk::Change;
use super::colours::Target;
use super::{Read, XFIRE_XML_FORMAT};

fn target(t: Target) -> String {
    match t {
        Target::Token(id) => format!("token {id}"),
        Target::Fixed(id) => format!("registry-fixed {id} (a skin cannot set it)"),
        Target::Shift => "image shift (baked)".to_owned(),
        Target::NoRole(role) => format!("not mapped: no registry role {role} yet"),
        Target::Dropped(why) => format!("dropped: {why}"),
    }
}

/// The NOTICE text, deterministic for one input.
#[must_use]
pub fn render(id: &str, read: &Read, source: &str, changes: &[Change]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "builtin:{id}: imported by `cargo xtask import-skin --format {XFIRE_XML_FORMAT}`"
    );
    let _ = writeln!(
        s,
        "(plan E7.7.3 item 1; rekindle-skin::import::xfire_xml).\n"
    );
    let _ = writeln!(s, "Provenance");
    for (k, v) in &read.info {
        let _ = writeln!(s, "  {k}: {v}");
    }
    let _ = writeln!(s, "  source tree BLAKE3: {source}");
    let _ = writeln!(
        s,
        "\nLicence\n  The skin art and palette are third-party work published without a licence.\n  \
         Rekindle's MIT LICENSE does not cover this pack.\n"
    );
    let _ = writeln!(s, "Colours ({} defined)", read.colours.len());
    for c in &read.colours {
        let [r, g, b] = c.value;
        let _ = writeln!(
            s,
            "  {} RGBA({r},{g},{b},0) -> {}",
            c.name,
            target(c.target)
        );
    }
    if let Some((img, i)) = &read.frame {
        let _ = writeln!(
            s,
            "\nRegions\n  window.frame: the nine-piece frame of {}, {}x{} px, nine-slice {}/{}/{}/{}",
            super::FRAME_SOURCE,
            img.width(),
            img.height(),
            i.top,
            i.right,
            i.bottom,
            i.left
        );
    }
    let _ = writeln!(s, "\nDropped");
    for (what, n) in &read.dropped {
        let _ = writeln!(s, "  {what}: {n}");
    }
    let _ = writeln!(
        s,
        "  sounds, strings, popups, other bitmaps and the in-game overlay (XIG) are not read."
    );
    notice::changes(
        &mut s,
        "Contrast changes (minimal Oklab-lightness walk)",
        changes,
    );
    s
}
