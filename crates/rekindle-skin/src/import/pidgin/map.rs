//! What each Pidgin theme item becomes (plan E7.7.3 item 4).
//!
//! The vocabulary is Pidgin 2.x's, read from its loaders' behaviour (no
//! code copied; `skins/research/a-legacy-formats.md` §4.6):
//! * buddy list (`purple/blist/theme.xml`, root `type="pidgin buddy list"`):
//!   `<blist color>`, `<groups><expanded|collapsed font color background>`,
//!   `<buddys><background color>` and eight `<buddys>` text styles, each
//!   `font` and `color` (`gtkblist-theme-loader.c:52-180`);
//! * status icons (`purple/status-icon/theme.xml`): `<icon id file>`
//!   (`gtkicon-theme-loader.c:43-77`);
//! * sounds (`purple/sound/theme.xml`, root `type="sound"`): `<event name
//!   file>` (`sound-theme-loader.c:44-77`), event names from
//!   `gtksound.c:59-71`.
//!
//! Icons fill the registry's [`IconSlot`]s and sounds its [`SoundCue`]s
//! (`crate::registry`), typed so this table cannot name an id the
//! manifest validator would refuse.

use crate::registry::{IconSlot, SoundCue};

/// What one Pidgin item became: a registry token (`&str`), icon slot or
/// sound cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<T = &'static str> {
    /// The registry entry it fills.
    Mapped(T),
    /// Dropped, and why.
    Dropped(&'static str),
}

use Target::{Dropped, Mapped};

impl<T> Target<T> {
    /// The mapped entry through `f`; a drop stays a drop.
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Target<U> {
        match self {
            Mapped(t) => Mapped(f(t)),
            Dropped(why) => Dropped(why),
        }
    }
}

const NO_GROUP: Target = Dropped("no group-header role yet");

/// Buddy-list colours: the element path under the root, its attribute,
/// and the registry token.
pub const COLOURS: [(&str, &str, Target); 14] = [
    ("blist", "color", Mapped("bg.panel")),
    ("groups/expanded", "color", NO_GROUP),
    ("groups/expanded", "background", NO_GROUP),
    ("groups/collapsed", "color", NO_GROUP),
    ("groups/collapsed", "background", NO_GROUP),
    (
        "buddys/background",
        "color",
        Dropped("no contact-row fill role"),
    ),
    ("buddys/contact_text", "color", Mapped("text.primary")),
    ("buddys/online_text", "color", Mapped("status.online-text")),
    ("buddys/away_text", "color", Mapped("status.away-text")),
    (
        "buddys/offline_text",
        "color",
        Mapped("status.offline-text"),
    ),
    (
        "buddys/idle_text",
        "color",
        Dropped("Rekindle has no idle status"),
    ),
    (
        "buddys/message_text",
        "color",
        Dropped("no unread-row text role"),
    ),
    (
        "buddys/message_nick_said_text",
        "color",
        Dropped("no nick-said row text role"),
    ),
    ("buddys/status_text", "color", Mapped("text.status")),
];

/// Status icon ids (Pidgin's status names) and the slot each fills.
pub const ICONS: [(&str, Target<IconSlot>); 9] = [
    ("available", Mapped(IconSlot::Online)),
    ("away", Mapped(IconSlot::Away)),
    ("busy", Mapped(IconSlot::Busy)),
    ("offline", Mapped(IconSlot::Offline)),
    (
        "extended-away",
        Dropped("Rekindle has no extended-away status"),
    ),
    ("invisible", Mapped(IconSlot::Invisible)),
    ("chat", Dropped("Rekindle has no chat status")),
    ("log-in", Dropped("not a status")),
    ("log-out", Dropped("not a status")),
];

/// Sound events and the cue each plays. One file per cue: `message` takes
/// `im_recv`, so the other receive events are dropped, not merged.
pub const SOUNDS: [(&str, Target<SoundCue>); 12] = [
    ("login", Mapped(SoundCue::SignOn)),
    ("logout", Mapped(SoundCue::SignOff)),
    ("im_recv", Mapped(SoundCue::Message)),
    ("nick_said", Mapped(SoundCue::Mention)),
    (
        "first_im_recv",
        Dropped("one file per cue: message takes im_recv"),
    ),
    (
        "chat_msg_recv",
        Dropped("one file per cue: message takes im_recv"),
    ),
    ("send_im", Dropped("no Rekindle cue for a sent message")),
    (
        "send_chat_msg",
        Dropped("no Rekindle cue for a sent message"),
    ),
    ("join_chat", Dropped("no Rekindle cue for a chat join")),
    ("left_chat", Dropped("no Rekindle cue for a chat leave")),
    ("pounce_default", Dropped("Rekindle has no buddy pounce")),
    ("got_attention", Dropped("Rekindle has no attention buzz")),
];

/// The target of `key` in `table`, or dropped as unknown.
#[must_use]
pub fn lookup<T: Copy>(table: &[(&str, Target<T>)], key: &str) -> Target<T> {
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(Dropped("not a Pidgin id we know"), |(_, t)| *t)
}

/// The registry id a sound event (`sound`) or status icon `key` fills.
#[must_use]
pub fn entry_target(sound: bool, key: &str) -> Target {
    if sound {
        lookup(&SOUNDS, key).map(SoundCue::id)
    } else {
        lookup(&ICONS, key).map(IconSlot::id)
    }
}

/// `#rgb` or `#rrggbb`. Pidgin's `gdk_rgba_parse` also takes colour names
/// and `rgb()`/`rgba()`; those are dropped and named, never guessed.
#[must_use]
pub fn rgb(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize, n: usize| u8::from_str_radix(&hex[i..i + n], 16).ok();
    match hex.len() {
        3 => Some([digit(0, 1)? * 17, digit(1, 1)? * 17, digit(2, 1)? * 17]),
        6 => Some([digit(0, 2)?, digit(2, 2)?, digit(4, 2)?]),
        _ => None,
    }
}

/// A `file=` that is one plain relative path: no `..`, root, drive or
/// backslash (CVE-2014-3697 was a drive-letter path in a theme tarball).
#[must_use]
pub fn plain_path(file: &str) -> bool {
    !file.is_empty()
        && !file.starts_with('/')
        && !file.contains(['\\', ':'])
        && file
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
