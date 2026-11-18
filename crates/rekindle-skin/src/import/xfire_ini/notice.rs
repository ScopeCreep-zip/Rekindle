//! The `NOTICE` beside an imported INI-era pack: provenance, licence
//! status, what every colour group became, the frame's states, every file
//! and key dropped, and every change the walk made.

use std::fmt::Write as _;

use super::super::credit::Authorship;
use super::super::notice;
use super::super::walk::Change;
use super::{Read, XFIRE_INI_FORMAT};

/// The NOTICE text, deterministic for one input.
#[must_use]
pub fn render(
    id: &str,
    read: &Read,
    source: &str,
    authorship: Authorship<'_>,
    changes: &[Change],
) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "builtin:{id}: imported by `cargo xtask import-skin --format {XFIRE_INI_FORMAT}`"
    );
    let _ = writeln!(
        s,
        "(plan E7.7.3 item 2; rekindle-skin::import::xfire_ini).\n"
    );
    let _ = writeln!(s, "Provenance");
    for (k, v) in &read.info {
        let _ = writeln!(s, "  {k}: {v}");
    }
    let _ = writeln!(s, "  source tree BLAKE3: {source}");
    let _ = writeln!(s, "\n{}", authorship.notice_text());
    let _ = writeln!(s, "Colour groups ({} defined)", read.colours.len());
    for c in &read.colours {
        let [r, g, b] = c.value;
        let _ = writeln!(
            s,
            "  {} RGBA({r},{g},{b},0) -> {}",
            c.name,
            c.target.describe()
        );
    }
    if let Some(f) = &read.frame {
        let i = f.insets;
        let mut sections: Vec<&str> = f.shape.iter().flatten().map(|(sec, _)| *sec).collect();
        sections.dedup();
        let _ = writeln!(
            s,
            "\nRegions\n  window.frame: [{}], nine-slice {}/{}/{}/{}, transparent middle",
            sections.join("] ["),
            i.top,
            i.right,
            i.bottom,
            i.left
        );
        for (state, img) in &f.images {
            let _ = writeln!(s, "    {state:?}: {}x{} px", img.width(), img.height());
        }
        for (state, why) in &f.skipped {
            let _ = writeln!(s, "    {state:?} not emitted: {why}");
        }
    }
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
            "  {}: [{}], {w}x{h} px, {:?}; states {}",
            c.region,
            read.tally.sources.get(c.region).map_or("", String::as_str),
            c.fit,
            states.join(", ")
        );
    }
    let t = &read.tally;
    for (component, state, why) in &t.dropped {
        let _ = writeln!(s, "  dropped [{component}] {state}: {why}");
    }
    let _ = writeln!(s, "\nDropped");
    for (what, n) in &read.dropped {
        let _ = writeln!(s, "  {what}: {n}");
    }
    let _ = writeln!(
        s,
        "  lines Windows reads no value from ({}):",
        read.repeated.len()
    );
    for r in &read.repeated {
        let _ = writeln!(s, "    {r}");
    }
    let _ = writeln!(s, "  files not read ({}):", read.unread.len());
    for f in &read.unread {
        let _ = writeln!(s, "    {f}");
    }
    notice::changes(
        &mut s,
        "Contrast changes (minimal Oklab-lightness walk)",
        changes,
    );
    s
}
