//! The Qt palette both front-ends share (plan E7.7.3 item 5): the
//! `QPalette` colour roles and the registry token each fills, and the two
//! ways a theme writes them.
//!
//! * Mumble: an `ApplicationPalette { qproperty-<role>: <colour>; … }`
//!   block at the head of the style's QSS (`skins/raw/a/mumble-theme/
//!   Dark.qss:18-44`; `a-legacy-formats.md` §5.2). Role names are
//!   lowercase, a disabled shade is `<role>_disabled`.
//! * TeamSpeak 3: `QPalette::<Role> = <colour>;` and `CustomColor::<Name>
//!   = <colour>;` lines in the stylesheet's header comment (`skins/raw/a/
//!   ts3-improved-theme/styles/improved.qss:6-12`, `improved_darkmode.qss:
//!   7-8`). Role names are CamelCase.
//!
//! Only the palette is read. QSS selectors are Qt widget types, and QSS
//! is never applied (its `url()` reads any local file, Qt's own security
//! note, `b-modern-theme-systems.md` §6.4): every other rule is dropped.
//! Colours are `#rgb`/`#rrggbb` only ([`rgb`]); a name, `transparent` or
//! `#aarrggbb` is dropped and named, never guessed.

pub use crate::import::pidgin::map::{rgb, Target};

use Target::{Dropped, Mapped};

const DISABLED: Target = Dropped("no disabled-state roles");
const BEVEL: Target = Dropped("a 3D bevel shade; Rekindle has no bevel roles");
const TOOLTIP: Target = Dropped("no tooltip role");
const BUTTON: Target = Dropped("no button fill or label role");

/// `QPalette` colour roles, lowercased, and the token each fills.
pub const QPALETTE_ROLES: [(&str, Target); 20] = [
    ("window", Mapped("bg.panel")),
    ("windowtext", Mapped("text.primary")),
    ("base", Mapped("bg.input")),
    ("alternatebase", Mapped("bg.secondary")),
    ("text", Mapped("text.base")),
    ("highlight", Mapped("accent.fill")),
    (
        "highlightedtext",
        Dropped("accent.fg is fixed by the registry"),
    ),
    ("link", Mapped("link")),
    ("linkvisited", Dropped("no visited-link role")),
    ("brighttext", Dropped("no bright-text role")),
    ("placeholdertext", Dropped("no placeholder role")),
    ("tooltipbase", TOOLTIP),
    ("tooltiptext", TOOLTIP),
    ("button", BUTTON),
    ("buttontext", BUTTON),
    ("mid", Mapped("border")),
    ("light", BEVEL),
    ("midlight", BEVEL),
    ("dark", BEVEL),
    ("shadow", BEVEL),
];

/// The token a role name fills (any case; `_disabled` shades dropped).
#[must_use]
pub fn role(name: &str) -> Target {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with("_disabled") {
        return DISABLED;
    }
    QPALETTE_ROLES
        .iter()
        .find(|(r, _)| *r == lower)
        .map_or(Dropped("not a QPalette role"), |(_, t)| *t)
}

/// `text` with every `/* … */` comment removed (an unclosed one runs to
/// the end).
#[must_use]
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        rest = rest[at + 2..]
            .find("*/")
            .map_or("", |end| &rest[at + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// Mumble: every `qproperty-<role>: <value>` of the first
/// `ApplicationPalette { … }` block, and every other declaration in it.
/// `None` when the stylesheet has no such block.
#[must_use]
pub fn application_palette(qss: &str) -> Option<Vec<(String, String)>> {
    let text = strip_comments(qss);
    let at = text.find("ApplicationPalette")?;
    let body = &text[at..];
    let open = body.find('{')?;
    let close = body[open..].find('}').map_or(body.len(), |c| open + c);
    let decls = body[open + 1..close]
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    Some(decls)
}

/// TeamSpeak 3: every `<prefix><Name> = <value>;` in `qss` (the header
/// comment's overrides), in order. A `<prefix>` not followed by an
/// identifier, `=` and a one-line value up to `;` is prose and skipped.
#[must_use]
pub fn overrides<'a>(qss: &'a str, prefix: &str) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    let mut rest = qss;
    while let Some(at) = rest.find(prefix) {
        rest = &rest[at + prefix.len()..];
        let ident_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(ident_len);
        let Some(value) = after.trim_start_matches([' ', '\t']).strip_prefix('=') else {
            continue;
        };
        let Some(end) = value.find([';', '\n']) else {
            continue;
        };
        if name.is_empty() || !value[end..].starts_with(';') {
            continue;
        }
        out.push((name, value[..end].trim()));
    }
    out
}
