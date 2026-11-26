//! The Xfire INI era's colour-group names (DoiDD, `DoiDDThemes.ini:6-61`)
//! and what each becomes: `.claude/plans/skins/research/d1-skeuo-winamp.md`
//! §3.2 and §3.3 "DoiDD (INI-era) names not in Carbon Fiber", read against
//! the registry as built. A name with an XML-era twin maps where its twin
//! maps (`BuddyViewBG` as `ListBGCG`, `ChatViewCG` as `ChatViewBGCG`, …);
//! a d1 role the registry does not hold yet is [`Target::NoRole`].
//! The values are read by the XML era's [`parse_rgba`](super::super::xfire_xml::colours::parse_rgba).

use super::super::xfire_xml::colours::Target;

const SERVERS: Target = Target::Dropped("server browser");
const FOF: Target = Target::Dropped("friends of friends (E7.8 research)");
const XIG: Target = Target::Dropped("in-game overlay (E7.5 research)");

/// Every DoiDD name and the nine Combat (Xfire's own INI skin,
/// `CombatThemes.ini`) defines under its own names, each mapped where the
/// Combat file that uses it puts it, in byte order.
pub const NAMES: &[(&str, Target)] = &[
    ("AdAreaCG", Target::Dropped("the AdView class is dropped")),
    // Combat: the chat area's other-party name and text (`CombatChatSkin.ini:321-322`).
    ("AllServers", Target::NoRole("chat.peer.text")),
    ("AllServersText", SERVERS),
    ("BevelCG", Target::Shift),
    ("BottomBarCG", Target::Shift),
    ("BuddyViewBG", Target::Token("bg.primary")),
    ("BuddyViewFofText", FOF),
    ("BuddyViewFriendText", Target::Token("text.base")),
    (
        "BuddyViewNonFocus",
        Target::NoRole("list.selected.bg.inactive"),
    ),
    ("BuddyViewOfflineText", Target::Token("status.offline-text")),
    ("BuddyViewSelected", Target::Token("accent.fill")),
    ("BuddyViewSelectedText", Target::Token("selection.text")),
    ("ButtonCG", Target::Shift),
    // Combat: `ColorGroup=` of the chat area and edit box, as DoiDD's
    // `ColorGroup=InfoViewBG` grounds its view (`ChatViewCG`'s role).
    ("ChatBGCG", Target::Token("bg.panel")),
    ("ChatMainCG", Target::Shift),
    ("ChatMainDeactivatedCG", Target::Shift),
    ("ChatOtherCG", Target::NoRole("chat.peer.name")),
    ("ChatOtherTextCG", Target::NoRole("chat.peer.text")),
    ("ChatPersonalCG", Target::NoRole("chat.self.name")),
    ("ChatPersonalTextCG", Target::NoRole("chat.self.text")),
    ("ChatViewCG", Target::Token("bg.panel")),
    (
        "DownloadText",
        Target::Dropped("the download manager's tab label"),
    ),
    // Combat: the buddy list's `Text.color_friend` (`combat.ini:491`).
    ("Friend", Target::Token("text.base")),
    ("FriendofFriend", FOF),
    ("FriendsFavoritesText", SERVERS),
    ("HeaderColor", Target::Shift),
    ("HeaderText", Target::Token("text.secondary")),
    ("Hilight", Target::Shift),
    ("HilightDown", Target::Shift),
    ("HilightHover", Target::Shift),
    ("InGameChatTextColor", XIG),
    ("InfoBackDropCG", Target::Shift),
    ("InfoOuterCG", Target::Shift),
    ("InfoViewBG", Target::Token("bg.secondary")),
    ("InfoViewFofText", FOF),
    ("InfoViewFriendText", Target::NoRole("profile.friend-text")),
    ("InfoViewHyperlink", Target::NoRole("profile.link")),
    (
        "InfoViewOfflineText",
        Target::NoRole("profile.offline-text"),
    ),
    ("InfoViewText", Target::NoRole("profile.text")),
    ("InnerBevelBkCg", Target::Shift),
    // Combat: `Text.color_selectedtext` (`combat.ini:496`).
    ("Label", Target::Token("selection.text")),
    ("LogoCG", Target::Shift),
    ("LogoDown", Target::Shift),
    ("LogoHover", Target::Shift),
    ("MainCG", Target::Shift),
    ("MainDeActivatedCG", Target::Shift),
    ("MenuBarCG", Target::Shift),
    ("MenuDownCG", Target::Shift),
    ("MenuHoverCG", Target::Shift),
    ("MenuTextCG", Target::Token("menubar.text")),
    // Combat: the download progress bar's fill (`combat.ini:537`).
    ("ProgressFill", Target::Token("progress.fill")),
    ("ScrollbarColor", Target::Shift),
    ("ScrollbarColorBG", Target::Token("scrollbar.track")),
    // Combat: `Text.color_selected` and `color_nonfocus` (`combat.ini:494-495`).
    ("Select", Target::Token("accent.fill")),
    ("StatusBackgroundCG", Target::Token("bg.status")),
    ("StatusCG", Target::Token("text.status")),
    ("StatusWrapCG", Target::Shift),
    ("TooltipCG", Target::Token("tooltip.bg")),
    ("TooltipTextCG", Target::Token("tooltip.text")),
    ("TypingNotificationCG", Target::NoRole("chat.typing.text")),
    ("UserNameBkCG", Target::Shift),
    ("UserNameCG", Target::NoRole("self.name")),
    ("VentCG", Target::Dropped("a Ventrilo panel")),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_counts_hold() {
        assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(NAMES.len(), 63);
        let dropped = NAMES
            .iter()
            .filter(|(_, t)| matches!(t, Target::Dropped(_)))
            .count();
        assert_eq!(dropped, 9);
    }
}
