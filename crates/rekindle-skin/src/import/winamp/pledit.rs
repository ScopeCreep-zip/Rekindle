//! `PLEDIT.TXT`: the `[Text]` section's playlist colours (d1 §2.4; keys
//! as Webamp reads them, `skinParserUtils.ts` `getPlaylistStyle`). Keys are
//! case-insensitive; a value is `#RRGGBB` or `RRGGBB`. Only `[Text]` is read.
//! `Font=` names a face: it becomes `font.family` only when the bundled
//! allow-list (`registry::fonts`) carries it, and is dropped otherwise.

use super::sprites::Target;
use crate::registry::{FontFace, FONTS};

/// The `[Text]` key naming the playlist face.
pub const FONT_KEY: &str = "font";
/// The token an allowed face becomes.
pub const FONT_TOKEN: &str = "font.family";

/// The keys and what each becomes (PLEDIT owns the list tokens).
pub const KEYS: [(&str, Target); 6] = [
    ("normal", Target::Tokens(&["text.base"])),
    ("current", Target::NoRole("list.selected.text")),
    ("normalbg", Target::NoRole("list.bg")),
    ("selectedbg", Target::NoRole("list.selected.bg")),
    ("mbfg", Target::Dropped("minibrowser")),
    ("mbbg", Target::Dropped("minibrowser")),
];

/// The `Font=` line and the allowed face it names, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    pub value: String,
    pub face: Option<&'static FontFace>,
}

/// `value` against the allow-list, ignoring ASCII case (Winamp writes
/// `Arial`, `Tahoma`, `verdana`): the allowed face, by its canonical name.
#[must_use]
pub fn font(value: &str) -> Font {
    Font {
        value: value.to_owned(),
        face: FONTS.iter().find(|f| f.name.eq_ignore_ascii_case(value)),
    }
}

/// One `[Text]` line: the key as written and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub key: String,
    pub value: String,
}

/// The `[Text]` section's `key=value` lines, in order.
#[must_use]
pub fn lines(bytes: &[u8]) -> Vec<Line> {
    let text: String = bytes.iter().map(|&b| char::from(b)).collect();
    let mut in_text = false;
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_text = section.trim().eq_ignore_ascii_case("text");
            continue;
        }
        if !in_text {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push(Line {
                key: k.trim().to_owned(),
                value: v.trim().to_owned(),
            });
        }
    }
    out
}

/// `#RRGGBB` or `RRGGBB` as RGB.
#[must_use]
pub fn rgb(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([ch(0)?, ch(2)?, ch(4)?])
}

/// What `key` becomes, case-insensitively.
#[must_use]
pub fn target(key: &str) -> Option<Target> {
    KEYS.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, t)| *t)
}
