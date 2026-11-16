//! The `NOTICE` beside an imported pack: provenance, licence status, what
//! every colour name became, what was dropped, and every change the walk
//! made with its original and new colour and measured ratios.

use std::fmt::Write as _;

use super::super::notice;
use super::super::walk::Change;
use super::{Read, XFIRE_XML_FORMAT};

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
    let licence = if read.is_xfire_own() {
        "The skin art and palette are Xfire, Inc.'s own work (Info.xml's original\n  \
         author). Xfire is defunct; the Rekindle owner ruled on 2026-10-15 that\n  \
         Xfire's own assets may be used, and replaces them with the same command."
    } else {
        "The skin art and palette are third-party work published without a licence."
    };
    let _ = writeln!(
        s,
        "\nLicence\n  {licence}\n  Rekindle's MIT LICENSE does not cover this pack.\n"
    );
    let _ = writeln!(s, "Colours ({} defined)", read.colours.len());
    for c in &read.colours {
        let [r, g, b] = c.value;
        let _ = writeln!(
            s,
            "  {} RGBA({r},{g},{b},0) -> {}",
            c.name,
            c.target.describe()
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
    controls(&mut s, read);
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

/// The painted control slots and the state image sets kept and dropped.
fn controls(s: &mut String, read: &Read) {
    if read.frame.is_none() && !read.controls.is_empty() {
        let _ = writeln!(s, "\nRegions");
    }
    for c in &read.controls {
        let states: Vec<&str> = c.images.keys().map(|k| k.key()).collect();
        let (w, h) = c
            .images
            .values()
            .next()
            .map_or((0, 0), rekindle_image::RgbaImage::dimensions);
        let _ = writeln!(
            s,
            "  {}: {} from {}, {w}x{h} px, {:?}; states {}",
            c.region,
            if c.region.starts_with("titlebar") {
                "the title button"
            } else {
                "the scrollbar"
            },
            super::controls::SOURCE,
            c.fit,
            states.join(", ")
        );
    }
    let t = &read.tally;
    let _ = writeln!(
        s,
        "\nState images (Hover -> hover, Down -> down, Disabled -> disabled)\n  kept {} of {} images",
        t.images.0, t.images.1
    );
    for (state, (kept, of)) in &t.sets {
        let _ = writeln!(s, "  {state} sets: kept {kept} of {of}");
    }
    for (component, state, why) in &t.dropped {
        let _ = writeln!(s, "  dropped {component} {state}: {why}");
    }
}
