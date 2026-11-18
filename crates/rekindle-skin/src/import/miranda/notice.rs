//! The `NOTICE` beside an imported Miranda NG pack: provenance, licence
//! status, what every colour became (unmapped ones named), the regions,
//! everything dropped, and every change the walk made.

use std::fmt::Write as _;

use super::super::notice;
use super::super::walk::Change;
use super::{Read, MIRANDA_FORMAT};

/// The NOTICE text, deterministic for one input.
#[must_use]
pub fn render(id: &str, read: &Read, source: &str, changes: &[Change]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "builtin:{id}: imported by `cargo xtask import-skin --format {MIRANDA_FORMAT}`\n\
         (plan E7.7.3 item 6; rekindle-skin::import::miranda).\n"
    );
    let _ = writeln!(s, "Provenance\n  engine: {:?}", read.engine);
    let _ = writeln!(s, "  skin file: {}", read.skin_file);
    for (k, v) in &read.info {
        let _ = writeln!(s, "  {k}: {v}");
    }
    let _ = writeln!(s, "  source tree BLAKE3: {source}");
    let _ = writeln!(
        s,
        "\nLicence\n  The skin art and palette are third-party work under their author's terms.\n  \
         Rekindle's MIT LICENSE does not cover this pack.\n"
    );
    let _ = writeln!(s, "Colours ({} defined)", read.colours.len());
    for c in &read.colours {
        let to = c.token.map_or_else(
            || "unmapped (no registry role)".to_owned(),
            |t| format!("token {t}"),
        );
        let _ = writeln!(s, "  {} {}={} -> {to}", c.item, c.key, c.raw);
    }
    let _ = writeln!(s, "\nRegions ({})", read.regions.len());
    for (slot, r) in &read.regions {
        let i = r.insets;
        let _ = writeln!(
            s,
            "  {slot}: nine-slice {}/{}/{}/{} (top/right/bottom/left)",
            i.top, i.right, i.bottom, i.left
        );
        for (state, img) in &r.images {
            let src = r.sources.get(state).map_or("", String::as_str);
            let _ = writeln!(
                s,
                "    {state:?}: {}x{} px from {src}",
                img.width(),
                img.height()
            );
        }
    }
    let _ = writeln!(s, "\nDropped ({})", read.dropped.len());
    for d in &read.dropped {
        let _ = writeln!(s, "  {d}");
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
