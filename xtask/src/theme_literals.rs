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
    fn finds_named_colours_as_whole_words_only() {
        assert_eq!(
            named_in("color-mix(in srgb, white 5%, transparent)"),
            vec!["white"]
        );
        assert_eq!(named_in("1px solid Black"), vec!["Black"]);
        assert_eq!(
            named_in("color-mix(in srgb, var(--color-xfire-accent) 80%, black)"),
            vec!["black"]
        );
        // Not colours: custom-property names, quoted font names, the
        // keywords without a colour, and the system colours.
        assert!(named_in("var(--color-xfire-red-tan) \"Tan Sans\", 'navy'").is_empty());
        assert!(named_in("transparent currentColor inherit initial unset").is_empty());
        assert!(named_in("1px solid CanvasText; background: Highlight").is_empty());
    }

    #[test]
    fn the_token_blocks_may_hold_literals_and_nothing_else_in_global_may() {
        let css = "@theme {\n  --color-xfire-bg: #0a0a0a;\n}\n@layer sec { :root { --sec-x: #fff; } }\n@layer base {\n  body { color: #e7e7e7; }\n}\n/* #abc in a comment */\n@layer xfire-defaults { :root[data-skin-base=\"light\"] { --color-xfire-overlay: #000000; } }\n@layer a11y {\n  @media (prefers-contrast: more) {\n    :root { --color-xfire-border: rgba(255, 255, 255, 0.55); --color-xfire-x: white; }\n  }\n}";
        let in_global = scan_css(TOKEN_FILE, css);
        assert_eq!(in_global.len(), 3, "{in_global:?}");
        assert!(in_global[0].contains(":6  #e7e7e7"), "{in_global:?}");
        // The a11y overrides are not a token block.
        assert!(in_global[1].contains(":12  rgba("), "{in_global:?}");
        assert!(in_global[2].contains(":12  white"), "{in_global:?}");
        // The same token declarations anywhere else are literals.
        assert_eq!(scan_css("src/styles/theme/00-x.css", css).len(), 6);
    }

    #[test]
    fn a_seeded_named_colour_in_a_theme_part_fails() {
        let css = "@layer components {\n  .a:hover {\n    background: color-mix(in srgb, white 5%, transparent);\n  }\n  .b { background: black; border: 1px solid transparent; }\n}";
        let hits = scan_css("src/styles/theme/99-x.css", css);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits[0].contains(":3  white  in `background`"), "{hits:?}");
        assert!(hits[1].contains(":5  black"), "{hits:?}");
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
