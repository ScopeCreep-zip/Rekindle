//! The `NOTICE` beside an imported Winamp pack: provenance, licence
//! status, what every colour became, every mapped sprite and whether its
//! region is a registry slot yet, the import transforms, the entries never
//! read, and every change the walk made.

use std::fmt::Write as _;

use super::super::notice::{changes, hex};
use super::super::walk::Change;
use super::pledit::{self, FONT_TOKEN};
use super::sprites::Target;
use super::{Read, WINAMP_WSZ_FORMAT};
use crate::regions::slot_of;
use crate::registry::FONTS;

fn target(t: Target) -> String {
    match t {
        Target::Tokens(ids) => format!("token {}", ids.join(", ")),
        Target::NoRole(role) => format!("not mapped: no registry role {role} yet"),
        Target::Dropped(why) => format!("dropped: {why}"),
    }
}

/// The NOTICE text, deterministic for one input.
#[must_use]
pub fn render(id: &str, read: &Read, source: &str, made: &[Change]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{id}: imported by `cargo xtask import-skin --format {WINAMP_WSZ_FORMAT}`\n\
         (plan E7.7.3 item 3; rekindle-skin::import::winamp).\n\n\
         Provenance\n  archive BLAKE3: {source}\n\n\
         Licence\n  The skin art and palette are third-party work.\n  \
         Rekindle's MIT LICENSE does not cover this pack.\n"
    );
    let _ = writeln!(s, "Colours ({} read)", read.colours.len());
    for c in &read.colours {
        let _ = writeln!(s, "  {} {} -> {}", c.source, hex(c.value), target(c.target));
    }
    for line in &read.unknown {
        let _ = writeln!(
            s,
            "  PLEDIT.TXT {line} -> dropped: not a colour key we read"
        );
    }
    if let Some(f) = &read.font {
        let _ = writeln!(s, "  {}", font_line(f));
    }
    let _ = writeln!(s, "\nSprites (39 of Webamp's 228 map; d1 §2.3)");
    for (sprite, present) in &read.sprites {
        let state = match (present, slot_of(sprite.region).is_some()) {
            (false, _) => format!(
                "absent ({} not in the archive)",
                sprite.sheet.to_uppercase()
            ),
            (true, true) => format!("region {}", sprite.region),
            (true, false) => format!(
                "decoded; not mapped: no registry region {} yet",
                sprite.region
            ),
        };
        let _ = writeln!(s, "  {} -> {state}", sprite.name);
    }
    if read.frame.is_some() {
        let _ = writeln!(
            s,
            "\nImport transforms\n  \
             window.frame, titlebar: _SELECTED sprites are `normal`, plain ones `windowInactive`.\n  \
             window.frame: GEN_TOP_RIGHT's baked close cell (right 2, top 3, 9x9) repainted with \
             GEN_TOP_LEFT_RIGHT_FILL;\n  the 125 px bottom bars cut to their outer 25 px; \
             GEN_MIDDLE_*_BOTTOM stacked on them; the middle is GENEX's window background.\n  \
             titlebar.start, titlebar.end: the dominant colour of GEN_TOP_CENTER_FILL_SELECTED \
             (the titlebar's textBacking)."
        );
    }
    let _ = writeln!(s, "\nNever read past their names ({})", read.ignored.len());
    for name in &read.ignored {
        let _ = writeln!(s, "  {name}");
    }
    changes(
        &mut s,
        "Contrast changes (minimal Oklab-lightness walk)",
        made,
    );
    s
}

/// `Font=`: the token it became, or dropped with the allow-list named.
fn font_line(f: &pledit::Font) -> String {
    match f.face {
        Some(face) => format!(
            "PLEDIT.TXT Font={} -> token {FONT_TOKEN} ({})",
            f.value, face.name
        ),
        None => format!(
            "PLEDIT.TXT Font={} -> dropped: not on the bundled font allow-list ({})",
            f.value,
            FONTS.iter().map(|f| f.name).collect::<Vec<_>>().join(", ")
        ),
    }
}
