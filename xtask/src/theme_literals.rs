//! `check-theme-literals` (plan E7.7.0): every colour under `src/styles/`
//! is a token.
//!
//! A skin replaces tokens (E7.7.1's closed registry), so a colour written
//! as a literal in a rule is a colour no skin, no high-contrast mode and no
//! validator can reach. This gate fails on a hex colour (`#rgb`, `#rgba`,
//! `#rrggbb`, `#rrggbbaa`), a colour function (`rgb(`, `rgba(`, `hsl(`,
//! `hsla(`, `hwb(`, `lab(`, `lch(`, `oklab(`, `oklch(`, `color(`) or a CSS
//! named colour (`white`, `black`, … the 148 of CSS Color 4 §6.1) in any
//! declaration value of any stylesheet under `src/styles/`, and on a hex or
//! colour function anywhere in a non-CSS file there. `transparent` and
//! `currentColor` are not literals (no colour of their own), nor are the
//! CSS-wide keywords; the system colours (`Canvas`, `CanvasText`,
//! `Highlight`, …, §6.2) are the user's forced palette, which the
//! `forced-colors` rules must name.
//!
//! The one place literals live is the token blocks of `global.css`: the
//! custom-property declarations of `@theme` (the defaults) and of the
//! `:root` rules in `@layer xfire-defaults` (the per-base values) and
//! `@layer sec` (the reserved `--sec-*` set). Every other block of
//! `global.css`, the `@layer a11y` overrides included, is scanned like any
//! rule. Selectors are not scanned (`#root` is an id, not a colour);
//! comments and quoted strings are blanked first.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::css;
use crate::walk_source_files;

const STYLES: &str = "src/styles";
const TOKEN_FILE: &str = "src/styles/global.css";
const COLOUR_FUNCTIONS: &[&str] = &[
    "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch", "color",
];
/// The layers whose `:root` rules hold token values in [`TOKEN_FILE`].
const TOKEN_LAYERS: &[&str] = &["xfire-defaults", "sec"];
/// CSS Color 4 §6.1 `<named-color>`.
const NAMED_COLOURS: &[&str] = &[
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "rebeccapurple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
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

/// The CSS named colours in a declaration value, in order. Identifiers are
/// whole words (`--color-xfire-red` is one custom-property name, not
/// `red`); quoted strings (font names, `content`) are skipped.
fn named_in(value: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in value.chars().chain(std::iter::once(' ')) {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            word.push(c);
            continue;
        }
        if NAMED_COLOURS.contains(&word.to_ascii_lowercase().as_str()) {
            found.push(std::mem::take(&mut word));
        }
        word.clear();
        if c == '"' || c == '\'' {
            quote = Some(c);
        }
    }
    found
}

/// Whether `block` of [`TOKEN_FILE`] is a token block: `@theme`, or a
/// `:root` rule in one of [`TOKEN_LAYERS`].
fn is_token_block(block: &css::Block) -> bool {
    block.prelude == "@theme"
        || (block.selectors().iter().all(|s| s.starts_with(":root"))
            && TOKEN_LAYERS.iter().any(|l| block.in_layer(l)))
}

/// Literals in one stylesheet, as `rel:line  literal  in prop`.
fn scan_css(rel: &str, src: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for block in css::blocks(src) {
        let tokens_allowed = rel == TOKEN_FILE && is_token_block(&block);
        for decl in &block.decls {
            if tokens_allowed && decl.prop.starts_with("--") {
                continue;
            }
            for lit in literals_in(&decl.value)
                .into_iter()
                .chain(named_in(&decl.value))
            {
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
        // Generated from the registry, whose numbers are the tokens; staleness is
        // `check-skin-registry`'s.
        if rel.starts_with(crate::gen_skin_registry::GENERATED_STYLES) {
            continue;
        }
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
         the `@theme` block of {TOKEN_FILE} (a per-base value in its \
         `@layer xfire-defaults` block) and use `var(--…)`, mixed with \
         `color-mix(in srgb, var(--…) N%, transparent)` for an alpha; a \
         white or black hover overlay is `--color-xfire-overlay`.",
        hits.len()
    ))
}

#[cfg(test)]
#[path = "theme_literals_tests.rs"]
mod tests;
