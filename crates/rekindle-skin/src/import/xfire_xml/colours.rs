//! The Xfire `*CG` colour names and what each becomes (plan E7.7.3 item 1;
//! `.claude/plans/skins/research/d1-skeuo-winamp.md` §3.3, read against
//! the registry as built: a d1 role the registry does not hold yet is
//! [`Target::NoRole`], recorded in the `NOTICE`, never invented).
//!
//! `RGBA(r,g,b,a)`: `a` is a COLORREF-style unused byte, so a non-zero
//! alpha refuses the skin. On a text or fill role the RGB is absolute; in
//! an `<Images Color=…>` it is a signed per-channel shift baked into that
//! state's pixels.

use super::super::ImportError;

/// What a colour name becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// An absolute colour on this registry token.
    Token(&'static str),
    /// A registry role a skin cannot set (`Class::Fixed`).
    Fixed(&'static str),
    /// A signed shift on the images that name it; baked, never a token.
    Shift,
    /// The d1 role the registry does not hold yet; the default applies.
    NoRole(&'static str),
    /// Dropped with the reason.
    Dropped(&'static str),
}

const XIG: Target = Target::Dropped("in-game overlay (E7.5 research)");
const SERVERS: Target = Target::Dropped("server browser and file depth colours");

/// Every Carbon Fiber name, the 62 of `Themes.xml:7-77` and the 9 its
/// components reference undefined, and the two Symbiosis (Xfire's stock
/// skin, `Themes.xml`) adds, in byte order.
pub const NAMES: &[(&str, Target)] = &[
    ("BlackCG", Target::Dropped("XIG only")),
    ("ChatEditBGCG", Target::Token("bg.input")),
    ("ChatOtherHyperLinkCG", Target::NoRole("chat.peer.link")),
    ("ChatOtherNameCG", Target::NoRole("chat.peer.name")),
    ("ChatOtherTextCG", Target::NoRole("chat.peer.text")),
    ("ChatPersonalHyperLinkCG", Target::NoRole("chat.self.link")),
    ("ChatPersonalNameCG", Target::NoRole("chat.self.name")),
    ("ChatPersonalTextCG", Target::NoRole("chat.self.text")),
    ("ChatTitleCG", Target::Token("titlebar.text")),
    ("ChatViewBGCG", Target::Token("bg.panel")),
    ("DisabledText", Target::Token("text.disabled")),
    // Symbiosis's spelling of the same role (its components' `Disabled`
    // text states name it).
    ("DisabledTextCG", Target::Token("text.disabled")),
    ("FriendNameCG", Target::Token("text.primary")),
    ("HeaderTextCG", Target::Token("text.secondary")),
    ("InfoViewBGCG", Target::Token("bg.secondary")),
    ("InfoViewTextCG", Target::NoRole("profile.text")),
    ("LinkTextCG", Target::Token("link")),
    ("ListBGCG", Target::Token("bg.primary")),
    ("ListClanTextCG", Target::Token("text.clan")),
    (
        "ListColorInActive",
        Target::Dropped("defined, never referenced"),
    ),
    ("ListColorSelectedCG", Target::Token("accent.fill")),
    (
        "ListColorSelectedDiabledCG",
        Target::NoRole("list.selected.bg.inactive"),
    ),
    ("ListColorSelectedTextCG", Target::Token("selection.text")),
    (
        "ListFoFTextCG",
        Target::Dropped("friends of friends (E7.8 research)"),
    ),
    ("ListFriendTextCG", Target::Token("text.base")),
    ("ListOfflineTextCG", Target::Token("status.offline-text")),
    ("MenuTextCG", Target::Token("menubar.text")),
    ("ProgBarBGCG", Target::Token("progress.bg")),
    ("ProgBarFillCG", Target::Token("progress.fill")),
    ("ProgBarOutlineCG", Target::Token("progress.outline")),
    ("ProgBarTextCG", Target::Token("progress.text")),
    ("PushButtonCG", Target::Shift),
    ("PushButtonDeactivatedCG", Target::Shift),
    ("PushButtonDownCG", Target::Shift),
    ("PushButtonHoverCG", Target::Shift),
    // The group-chat room-name box; the dialog fields are TextField*CG's.
    (
        "RoomNameEditBGCG",
        Target::Dropped("room-name box (input.bg is TextFieldBGCG's)"),
    ),
    (
        "RoomNameEditTextCG",
        Target::Dropped("room-name box (input.text is TextFieldTextCG's)"),
    ),
    ("ScrollbarBGCG", Target::Token("scrollbar.track")),
    ("ScrollbarCG", Target::Shift),
    ("SideCG", Target::Shift),
    ("StatusBackgroundCG", Target::Token("bg.status")),
    ("StatusTextCG", Target::Token("text.status")),
    ("TextDepth1", SERVERS),
    ("TextDepth2", SERVERS),
    ("TextDepth3", SERVERS),
    ("TextDepth4", SERVERS),
    ("TextFavServersCG", SERVERS),
    // The dialog text field (bg.input stays ChatEditBGCG's chat edit box).
    ("TextFieldBGCG", Target::Token("input.bg")),
    ("TextFieldTextCG", Target::Token("input.text")),
    (
        "TextFieldTextHighlightCG",
        Target::Token("input.selection.bg"),
    ),
    (
        "TextFieldTextHighlightInactiveCG",
        Target::Token("input.selection.bg-inactive"),
    ),
    ("ToolTipBGCG", Target::Token("tooltip.bg")),
    ("ToolTipTextCG", Target::Token("tooltip.text")),
    ("UserNameCG", Target::NoRole("self.name")),
    ("UserTypingMessageCG", Target::NoRole("chat.typing.text")),
    ("XIGActiveWindowTitleCG", XIG),
    ("XIGBarTextCG", XIG),
    ("XIGCaretCG", XIG),
    ("XIGEditModeTitleCG", XIG),
    ("XIGFocusedColor", XIG),
    ("XIGFocusedColorCG", XIG),
    ("XIGLiveStatsTextCG", XIG),
    ("XIGLiveStatsTextDeactivatedCG", XIG),
    ("XIGMusicPlayerTrackTextCG", XIG),
    ("XIGOverlayTextCG", XIG),
    ("XIGOverlayTitleCG", XIG),
    ("XIGSystemDialogTextCG", XIG),
    ("XIGSystemDialogTitleCG", XIG),
    ("XIGWebTabActiveTextCG", XIG),
    ("XIGWebTabInActiveTextCG", XIG),
    ("XfireLogoCG", Target::Shift),
    ("XfireLogoDownCG", Target::Shift),
    ("XfireLogoHoverCG", Target::Shift),
];

impl Target {
    /// The `NOTICE` text for this target.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::Token(id) => format!("token {id}"),
            Self::Fixed(id) => format!("registry-fixed {id} (a skin cannot set it)"),
            Self::Shift => "image shift (baked)".to_owned(),
            Self::NoRole(role) => format!("not mapped: no registry role {role} yet"),
            Self::Dropped(why) => format!("dropped: {why}"),
        }
    }
}

/// The target of `name` in `table` (sorted by name); a name outside the
/// table is dropped.
#[must_use]
pub fn target_in(table: &[(&str, Target)], name: &str) -> Target {
    table
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .map_or(Target::Dropped("not an Xfire colour name"), |i| table[i].1)
}

/// The target of `name` in the XML era's [`NAMES`].
#[must_use]
pub fn target(name: &str) -> Target {
    target_in(NAMES, name)
}

/// `RGBA(r,g,b,a)` as signed channels; `a` must be 0 (or absent, as in
/// one sample value). Channels are −255..=255 (negative only as a shift).
///
/// # Errors
/// [`ImportError::NonZeroAlpha`] or [`ImportError::BadColour`].
pub fn parse_rgba(name: &str, value: &str) -> Result<[i16; 3], ImportError> {
    let bad = || ImportError::BadColour {
        name: name.to_owned(),
        value: value.to_owned(),
    };
    let inner = value
        .trim()
        .strip_prefix("RGBA(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(bad)?;
    let parts: Vec<i32> = inner
        .split(',')
        .map(|p| p.trim().parse::<i32>().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let (rgb, alpha) = match parts.as_slice() {
        [r, g, b] => ([*r, *g, *b], 0),
        [r, g, b, a] => ([*r, *g, *b], *a),
        _ => return Err(bad()),
    };
    channels(name, value, rgb, alpha)
}

/// Classic v3's attribute form, `red="…" green="…" blue="…" alpha="…"`
/// (423 of its 428 colours), under [`parse_rgba`]'s rule: `alpha` is the
/// same unused byte, 0 (405 values) or absent (18), and a non-zero one
/// refuses. `None` when the element carries none of the four attributes.
///
/// # Errors
/// [`ImportError::NonZeroAlpha`] or [`ImportError::BadColour`] (a channel
/// missing or not an integer).
pub fn parse_attrs(
    name: &str,
    attr: impl Fn(&str) -> Option<String>,
) -> Option<Result<[i16; 3], ImportError>> {
    let [r, g, b, a] = ["red", "green", "blue", "alpha"].map(&attr);
    if r.is_none() && g.is_none() && b.is_none() && a.is_none() {
        return None;
    }
    let value = format!(
        "red={} green={} blue={} alpha={}",
        r.as_deref().unwrap_or("?"),
        g.as_deref().unwrap_or("?"),
        b.as_deref().unwrap_or("?"),
        a.as_deref().unwrap_or("(absent)")
    );
    let bad = || ImportError::BadColour {
        name: name.to_owned(),
        value: value.clone(),
    };
    let int = |v: Option<&String>| v.and_then(|v| v.trim().parse::<i32>().ok()).ok_or_else(bad);
    let parsed = (|| {
        let rgb = [int(r.as_ref())?, int(g.as_ref())?, int(b.as_ref())?];
        let alpha = match &a {
            None => 0,
            Some(_) => int(a.as_ref())?,
        };
        channels(name, &value, rgb, alpha)
    })();
    Some(parsed)
}

fn channels(name: &str, value: &str, rgb: [i32; 3], alpha: i32) -> Result<[i16; 3], ImportError> {
    let bad = || ImportError::BadColour {
        name: name.to_owned(),
        value: value.to_owned(),
    };
    if alpha != 0 {
        return Err(ImportError::NonZeroAlpha {
            name: name.to_owned(),
            alpha,
        });
    }
    let mut out = [0i16; 3];
    for (o, c) in out.iter_mut().zip(rgb) {
        *o = i16::try_from(c)
            .ok()
            .filter(|v| (-255..=255).contains(v))
            .ok_or_else(bad)?;
    }
    Ok(out)
}

/// An absolute colour: every channel 0–255.
///
/// # Errors
/// [`ImportError::BadColour`] for a negative channel.
pub fn absolute(name: &str, value: &str, c: [i16; 3]) -> Result<[u8; 3], ImportError> {
    let mut out = [0u8; 3];
    for (o, v) in out.iter_mut().zip(c) {
        *o = u8::try_from(v).map_err(|_| ImportError::BadColour {
            name: name.to_owned(),
            value: value.to_owned(),
        })?;
    }
    Ok(out)
}

#[cfg(test)]
#[path = "colours_tests.rs"]
mod tests;
