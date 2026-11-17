//! `check-icon-glyphs` (plan E7.12): every `ICON_*` constant in
//! `src/icons.ts` names a codepoint that the bundled Nerd Font maps, and
//! the glyph there is the one its comment names.
//!
//! Each constant is one line, `export const ICON_X = "\u{F0000}"; // nf-md-name`
//! (anything after the glyph name is free text). The comment's
//! `nf-<prefix>-<name>` is the Nerd Fonts class; the font's glyph name is
//! `<prefix>-<name>` with `-` in the name written `_` (the font's `post`
//! table, e.g. `md-chat_plus`). A constant whose comment names another
//! glyph shows the wrong picture: 18 did (2026-11-17), among them a pail
//! for "camera off" and a fisheye for "attach".

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::woff2::Woff2Tables;

const ICONS: &str = "src/icons.ts";
const FONT: &str = "src/assets/fonts/SymbolsNerdFontMono-Regular.woff2";

#[derive(Debug, PartialEq, Eq)]
struct IconLine {
    name: String,
    codepoint: u32,
    glyph: String,
}

/// Parses one `export const ICON_…` line; any other shape is an error, so a
/// constant cannot escape the check by being written differently.
fn parse_icon_line(line: &str) -> Result<IconLine> {
    let rest = line
        .strip_prefix("export const ")
        .ok_or_else(|| anyhow!("not an export"))?;
    let (name, rest) = rest
        .split_once(" = \"\\u{")
        .ok_or_else(|| anyhow!("value is not \"\\u{{…}}\""))?;
    let (hex, rest) = rest
        .split_once("}\";")
        .ok_or_else(|| anyhow!("value is not one codepoint"))?;
    let codepoint = u32::from_str_radix(hex, 16).with_context(|| format!("codepoint `{hex}`"))?;
    let comment = rest
        .trim_start()
        .strip_prefix("// nf-")
        .ok_or_else(|| anyhow!("no `// nf-…` comment"))?;
    let class: String = comment
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let (prefix, glyph) = class
        .split_once('-')
        .ok_or_else(|| anyhow!("comment `nf-{class}` has no prefix"))?;
    if glyph.is_empty() {
        bail!("comment `nf-{class}` names no glyph");
    }
    Ok(IconLine {
        name: name.to_owned(),
        codepoint,
        glyph: format!("{prefix}-{}", glyph.replace('-', "_")),
    })
}

pub fn check_icon_glyphs(root: &Path) -> Result<()> {
    let font = std::fs::read(root.join(FONT)).with_context(|| FONT.to_owned())?;
    let tables = Woff2Tables::parse(&font).with_context(|| FONT.to_owned())?;
    let cmap = tables.cmap12()?;
    let names = tables.post_names()?;
    let src = std::fs::read_to_string(root.join(ICONS)).with_context(|| ICONS.to_owned())?;
    let mut faults = Vec::new();
    let mut seen = 0usize;
    for (n, line) in src.lines().enumerate() {
        if !line.starts_with("export const ") {
            continue;
        }
        let at = format!("{ICONS}:{}", n + 1);
        let icon = match parse_icon_line(line) {
            Ok(icon) => icon,
            Err(e) => {
                faults.push(format!("{at}: {e:#}"));
                continue;
            }
        };
        seen += 1;
        let Some(&gid) = cmap.get(&icon.codepoint) else {
            faults.push(format!(
                "{at}: {} U+{:X} is not in the font",
                icon.name, icon.codepoint
            ));
            continue;
        };
        let got = names.get(usize::from(gid)).cloned().flatten();
        if got.as_deref() != Some(icon.glyph.as_str()) {
            let want_cp = cmap
                .iter()
                .filter(|(_, g)| {
                    names.get(usize::from(**g)).and_then(Option::as_deref)
                        == Some(icon.glyph.as_str())
                })
                .map(|(cp, _)| format!("U+{cp:X}"))
                .min()
                .unwrap_or_else(|| "not in the font".to_owned());
            faults.push(format!(
                "{at}: {} U+{:X} is `{}`, its comment names `{}` ({want_cp})",
                icon.name,
                icon.codepoint,
                got.as_deref().unwrap_or("<standard name>"),
                icon.glyph
            ));
        }
    }
    if seen == 0 {
        bail!("{ICONS} declares no icon constants");
    }
    if faults.is_empty() {
        println!("    {seen} icons match {FONT}");
        return Ok(());
    }
    bail!(
        "{} icon(s) show another glyph:\n      {}",
        faults.len(),
        faults.join("\n      ")
    )
}

#[cfg(test)]
mod tests {
    use super::parse_icon_line;

    #[test]
    fn parses_the_declared_shape() {
        let l = parse_icon_line(
            r#"export const ICON_X = "\u{F03E2}";  // nf-md-paperclip (attach file)"#,
        )
        .ok();
        assert_eq!(
            l.map(|l| (l.name, l.codepoint, l.glyph)),
            Some(("ICON_X".into(), 0xF03E2, "md-paperclip".into()))
        );
        let dash =
            parse_icon_line(r#"export const ICON_Y = "\u{F0415}"; // nf-md-plus-circle — x"#).ok();
        assert_eq!(dash.map(|l| l.glyph), Some("md-plus_circle".into()));
    }

    #[test]
    fn refuses_other_shapes() {
        for bad in [
            r#"export const ICON_X = "\u{F03E2}";"#,
            r#"export const ICON_X = "\u{F03E2}"; // paperclip"#,
            r#"export const ICON_X = "\uF03E2"; // nf-md-paperclip"#,
            r"export const ICON_X = ICON_Y; // nf-md-paperclip",
        ] {
            assert!(parse_icon_line(bad).is_err(), "{bad}");
        }
    }
}
