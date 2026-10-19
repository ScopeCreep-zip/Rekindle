//! `check-theme-literals` (plan E7.7.0): every colour under `src/styles/`
//! is a token.
//!
//! A skin replaces tokens (E7.7.1's closed registry), so a colour written
//! as a literal in a rule is a colour no skin, no high-contrast mode and no
//! validator can reach. This gate fails on a hex colour (`#rgb`, `#rgba`,
//! `#rrggbb`, `#rrggbbaa`) or a colour function (`rgb(`, `rgba(`, `hsl(`,
//! `hsla(`, `hwb(`, `lab(`, `lch(`, `oklab(`, `oklch(`, `color(`) in any
//! declaration value of any stylesheet under `src/styles/`, and anywhere in
//! a non-CSS file there. The one place literals live is the token block of
//! `global.css`: its custom-property declarations (`--name: #…;`), which are
//! the defaults, the per-base values and the reserved `--sec-*` set.
//! Selectors are not scanned (`#root` is an id, not a colour); comments
//! are blanked first.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::css;
use crate::walk_source_files;

const STYLES: &str = "src/styles";
const TOKEN_FILE: &str = "src/styles/global.css";
const COLOUR_FUNCTIONS: &[&str] = &[
    "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch", "color",
];

/// The colour literals in `text`, in order.
pub(crate) fn literals_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    let mut found = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' {
            let digits = bytes[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            let after = bytes.get(i + 1 + digits).copied();
            if matches!(digits, 3 | 4 | 6 | 8) && !after.is_some_and(ident) {
                found.push(text[i..=i + digits].to_string());
            }
            continue;
        }
        if i > 0 && ident(bytes[i - 1]) {
            continue;
        }
        for f in COLOUR_FUNCTIONS {
            let end = i + f.len();
            if bytes.len() > end && bytes[end] == b'(' && text[i..end].eq_ignore_ascii_case(f) {
                found.push(format!("{}(", &text[i..end]));
            }
        }
    }
    found
}

/// Literals in one stylesheet, as `rel:line  literal  in prop`.
fn scan_css(rel: &str, src: &str) -> Vec<String> {
    let tokens_allowed = rel == TOKEN_FILE;
    let mut hits = Vec::new();
    for block in css::blocks(src) {
        for decl in &block.decls {
            if tokens_allowed && decl.prop.starts_with("--") {
                continue;
            }
            for lit in literals_in(&decl.value) {
                hits.push(format!("{rel}:{}  {lit}  in `{}`", decl.line, decl.prop));
            }
        }
    }
    hits
}

/// Literals in a non-CSS file under `src/styles` (the region map).
fn scan_other(rel: &str, src: &str) -> Vec<String> {
    src.lines()
        .enumerate()
        .flat_map(|(i, line)| {
            literals_in(line)
                .into_iter()
                .map(move |lit| format!("{rel}:{}  {lit}", i + 1))
        })
        .collect()
}

pub(crate) fn check_theme_literals(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for path in walk_source_files(&root.join(STYLES), &["css", "toml", "json"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("css"))
        {
            hits.extend(scan_css(&rel, &src));
        } else {
            hits.extend(scan_other(&rel, &src));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} colour literal(s) under {STYLES}/. Name the colour as a token in \
         the `@theme` block of {TOKEN_FILE} and use `var(--…)`, mixed with \
         `color-mix(in srgb, var(--…) N%, transparent)` for an alpha.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_hex_and_colour_functions() {
        assert_eq!(literals_in("1px solid #fff"), vec!["#fff"]);
        assert_eq!(
            literals_in("linear-gradient(to right, #0f1a3a, #1a2a5eCC)"),
            vec!["#0f1a3a", "#1a2a5eCC"]
        );
        assert_eq!(literals_in("0 1px rgb(0 0 0 / 0.3)"), vec!["rgb("]);
        assert_eq!(literals_in("RGBA(0, 0, 0, .5)"), vec!["RGBA("]);
        assert_eq!(literals_in("oklch(0.6 0.1 30)"), vec!["oklch("]);
        assert!(
            literals_in("color-mix(in srgb, var(--color-xfire-accent) 30%, transparent)")
                .is_empty()
        );
        assert!(literals_in("var(--color-xfire-bg-dark)").is_empty());
        // Not colours: a five-digit run, an id-like word, a fragment.
        assert!(literals_in("#12345 url(#grad) #main-content").is_empty());
    }

    #[test]
    fn a_seeded_literal_in_a_theme_part_fails() {
        let css = "@layer components {\n  .a {\n    color: var(--color-xfire-text);\n    background: rgba(0, 0, 0, 0.6);\n  }\n}";
        let hits = scan_css("src/styles/theme/99-x.css", css);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].starts_with("src/styles/theme/99-x.css:4  rgba("),
            "{hits:?}"
        );
    }

    #[test]
    fn the_token_block_may_hold_literals_and_nothing_else_in_global_may() {
        let css = "@theme {\n  --color-xfire-bg: #0a0a0a;\n}\n@layer sec { :root { --sec-x: #fff; } }\n@layer base {\n  body { color: #e7e7e7; }\n}\n/* #abc in a comment */";
        let in_global = scan_css(TOKEN_FILE, css);
        assert_eq!(in_global.len(), 1, "{in_global:?}");
        assert!(in_global[0].contains(":6  #e7e7e7"), "{in_global:?}");
        // The same token declarations anywhere else are literals.
        assert_eq!(scan_css("src/styles/theme/00-x.css", css).len(), 3);
    }

    #[test]
    fn non_css_files_are_scanned_whole() {
        assert_eq!(
            scan_other("src/styles/regions.toml", "'.a' = { on = [\"#fff\"] }").len(),
            1
        );
        assert!(scan_other(
            "src/styles/regions.toml",
            "'.a' = { on = [\"--color-xfire-bg\"] }"
        )
        .is_empty());
    }
}
