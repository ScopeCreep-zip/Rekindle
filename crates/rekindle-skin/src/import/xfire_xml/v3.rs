//! Xfire's Classic v3 layout (plan E7.7.3b; Wayback `media.xfire.com/
//! xfire/client/skins/xfire.zip`, 880 files): the XML era before
//! Symbiosis's `Components/` tree.
//!
//! * File names are lower case (`skin.xml`, `info.xml`, `xfirethemes.xml`)
//!   and attributes come in either case in one file (`name=`/`Name=`,
//!   `state=`/`State=`, `type=`/`Type=`, values like `Type="bottomleft"`,
//!   `<Author Type="original">`): files and attributes are matched
//!   ASCII-case-insensitively, as the Windows client did.
//! * Colours are mostly the attribute form `red= green= blue= alpha=`
//!   ([`super::colours::parse_attrs`]), under `RGBA()`'s alpha rule; their
//!   names are v3's own ([`NAMES`]).
//! * Every component is in one file, [`SOURCE`]; the chat window's frame is
//!   four of them ([`FRAME`]), read like the INI era's `[Top]`/`[Left]`/
//!   `[Right]`/`[Bottom]`: each outer edge's `middle` is the side piece, its
//!   `top`/`bottom` caps are not read, and the middle is transparent.

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::Insets;

use super::super::tree::Node;
use super::super::ImportError;
use super::colours::Target;
use super::{bitmap, descendants};

/// The file holding every v3 component.
pub const SOURCE: &str = "xfirecomponents.xml";

/// The chat window frame in [`bitmap::NINE`] order: (component, image
/// type); `None` is the transparent middle (`xfirechatwindow.xml` lays
/// these four tiles around the chat view).
pub const FRAME: [Option<(&str, &str)>; 9] = [
    Some(("ChatTitleBar", "topleft")),
    Some(("ChatTitleBar", "top")),
    Some(("ChatTitleBar", "topright")),
    Some(("ChatLeftOuterEdge", "middle")),
    None,
    Some(("ChatRightOuterEdge", "middle")),
    Some(("ChatBaseBoard", "bottomleft")),
    Some(("ChatBaseBoard", "bottom")),
    Some(("ChatBaseBoard", "bottomright")),
];

const XIG: Target = Target::Dropped("in-game overlay (E7.5 research)");
const SERVERS: Target = Target::Dropped("server browser colours");
const VOICE: Target = Target::Dropped("voice indicator: no region slot");

/// Every colour of v3's first theme (`xfirethemes.xml`, `default`), in
/// byte order; roles as the late names they correspond to
/// ([`super::colours::NAMES`]). A name used only as an `<Images Color=…>`
/// is a shift.
pub const NAMES: &[(&str, Target)] = &[
    ("AdAreaCG", Target::Dropped("advertising area")),
    ("AllServersText", SERVERS),
    ("BevelCG", Target::Shift),
    ("BottomBarCG", Target::Shift),
    ("BuddyViewBG", Target::Token("bg.primary")),
    ("BuddyViewClanText", Target::Token("text.clan")),
    (
        "BuddyViewFofText",
        Target::Dropped("friends of friends (E7.8 research)"),
    ),
    ("BuddyViewFriendText", Target::Token("text.base")),
    (
        "BuddyViewNonFocus",
        Target::NoRole("list.selected.bg.inactive"),
    ),
    ("BuddyViewOfflineText", Target::Token("status.offline-text")),
    ("BuddyViewSelected", Target::Token("accent.fill")),
    ("BuddyViewSelectedText", Target::Fixed("accent.fg")),
    ("ButtonCG", Target::Shift),
    ("ChatMainCG", Target::Shift),
    ("ChatMainDeactivatedCG", Target::Shift),
    ("ChatOtherCG", Target::NoRole("chat.peer.name")),
    ("ChatOtherTextCG", Target::NoRole("chat.peer.text")),
    ("ChatPersonalCG", Target::NoRole("chat.self.name")),
    ("ChatPersonalTextCG", Target::NoRole("chat.self.text")),
    ("ChatViewCG", Target::Token("bg.panel")),
    ("FriendsFavoritesText", SERVERS),
    ("HeaderColor", Target::Shift),
    ("HeaderText", Target::Token("text.secondary")),
    ("Hilight", Target::Shift),
    ("HilightDown", Target::Shift),
    ("HilightHover", Target::Shift),
    (
        "InfoBackDropCG",
        Target::Dropped("defined, never referenced"),
    ),
    ("InfoOuterCG", Target::Shift),
    ("InfoViewBG", Target::Token("bg.secondary")),
    (
        "InfoViewFofText",
        Target::Dropped("friends of friends (E7.8 research)"),
    ),
    ("InfoViewFriendText", Target::NoRole("profile.text")),
    ("InfoViewHyperlink", Target::Token("link")),
    ("InfoViewOfflineText", Target::NoRole("profile.text")),
    ("InfoViewText", Target::NoRole("profile.text")),
    ("InnerBevelBkCg", Target::Shift),
    ("LogoCG", Target::Shift),
    ("LogoCoverCG", Target::Shift),
    ("LogoCoverDown", Target::Shift),
    ("LogoCoverHover", Target::Shift),
    ("LogoDown", Target::Shift),
    ("LogoHover", Target::Shift),
    ("MainCG", Target::Shift),
    ("MainDeactivatedCG", Target::Shift),
    ("MenuBarCG", Target::Shift),
    ("MenuDownCG", Target::Shift),
    ("MenuHoverCG", Target::Shift),
    ("MenuTextCG", Target::NoRole("menubar.text")),
    ("MicOn", VOICE),
    ("ScrollbarColor", Target::Shift),
    ("ScrollbarColorBG", Target::NoRole("scrollbar.track")),
    ("SpeakerOn", VOICE),
    ("StatusBackgroundCG", Target::Token("bg.status")),
    ("StatusCG", Target::Token("text.status")),
    ("StatusWrapCG", Target::Shift),
    ("TooltipCG", Target::NoRole("tooltip.bg")),
    ("TooltipTextCG", Target::NoRole("tooltip.text")),
    ("TypingNotificationCG", Target::Shift),
    ("UserNameBkCG", Target::Shift),
    ("UserNameCG", Target::NoRole("self.name")),
    ("VentCG", Target::Shift),
    ("VoiceTalkingShift", VOICE),
    ("XIGAlertListCG", XIG),
    ("XIGFocusedColor", XIG),
    ("XIGFocusedRectColor", XIG),
    ("XIGMenuBarCG", XIG),
    ("XIGTitleBarDisabled", XIG),
    ("XIGTooltipBG", XIG),
    ("XIGTooltipText", XIG),
];

/// `(bitmap name, shift colour)` of `component`'s normal image `kind`.
fn image<'a>(
    components: &'a Node,
    component: &str,
    kind: &str,
) -> Option<(&'a str, Option<&'a str>)> {
    let c = descendants(components, "Component").into_iter().find(|c| {
        c.attr_ci("Name")
            .is_some_and(|n| n.eq_ignore_ascii_case(component))
    })?;
    c.children_named("Images")
        .filter(|i| {
            i.attr_ci("State")
                .is_none_or(|s| s.eq_ignore_ascii_case("Normal"))
        })
        .find_map(|i| {
            i.children_named("Image")
                .find(|img| {
                    img.attr_ci("Type")
                        .is_some_and(|t| t.eq_ignore_ascii_case(kind))
                })
                .map(|img| (img.text.as_str(), i.attr_ci("Color")))
        })
}

/// The chat window frame of `components` ([`SOURCE`]), each piece decoded
/// by `piece(bitmap name, shift colour)`; `None` when a piece is not named.
///
/// # Errors
/// What `piece` returns, or pieces that do not compose.
pub fn frame(
    components: &Node,
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
) -> Result<Option<(RgbaImage, Insets)>, ImportError> {
    let mut pieces: Vec<RgbaImage> = Vec::with_capacity(9);
    for slot in FRAME {
        let Some((component, kind)) = slot else {
            pieces.push(RgbaImage::new(1, 1));
            continue;
        };
        let Some((name, colour)) = image(components, component, kind) else {
            return Ok(None);
        };
        pieces.push(piece(name, colour)?);
    }
    let pieces: [RgbaImage; 9] = pieces.try_into().map_err(|_| ImportError::MissingFile {
        file: SOURCE.to_owned(),
    })?;
    bitmap::compose(SOURCE, &pieces).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted() {
        assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(NAMES.len(), 68);
    }
}
