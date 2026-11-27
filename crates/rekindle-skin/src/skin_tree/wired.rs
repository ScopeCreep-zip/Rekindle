//! The role of a tile neither name table knows, from what the pack itself
//! says the tile is and does (SKC3; ruling 3 of
//! `design-skin-catalogue-2026-11-26.md`: "component with no role: extend
//! the role table from the component's class and function"). It is asked
//! only after [`super::chat_roles::role`] and [`super::roles::role`] find
//! none, so no tile they name changes, and it reads no pack's name.

use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction};
use rekindle_types::dto::skin_tree::NodeRole;

use super::roles::{classless, Facts, Role};

/// The role of a tile from its class, art, children and own `triggers`:
///
/// * a classless tile (`Class=Tile`) with no art of its own that holds
///   tiles is their container, a group (the chat title holders
///   `[UserNameBox]` `Class=Tile events=Transparent` around `UserNameText`
///   and `TypingMessage`, `[TypingMessageField]` around `TypingMessage`,
///   `[BackGround]` around its `Topbg`/`Leftbg`/… art, Carbon Fiber's
///   `TabButtonsContainer` `Class="Tile"` with its images commented out
///   around the tab buttons, a chat window's root `[Chat]` `Class=Tile`);
/// * a tile holding none whose own trigger drags the window is a drag
///   strip (`[dgbar]` `Action.lbtndown=AppDragWindow` over its bar art);
/// * a `PushButton` the pack wires a pressing trigger on, or a tile whose
///   own pressing trigger resizes the window, is a button run by that
///   trigger, as [`super::roles`] makes a grip the pack wires itself
///   (`[StateClose]` `Action.click=AppClose`, `[WndMini]`
///   `Action.click=AppMinimize`, `[Spectator]` `Action.lbtndown=
///   GroupMessage(0)`, a pack link's `AppLaunchUrl`, `[LeftResize]`
///   `Action.lbtndown=ResizeL`); a `Menu` is the same control (Xfire's
///   class for a button acting on the press: `[dgbar]`, Guild Wars UI's
///   `StatusArrow`, Mountain Dew's `[MtDewLogo]` `Class=Menu`
///   `Action.lbtndown=AppLaunchUrl(...)`, its own link, `MtDew.ini:157-168`);
/// * such a control whose press pops one of the engine's menus is that
///   menu's title, as [`super::roles`] names `FileMenu`: Xbox 360's and
///   Evolans's `[File]` `Class=Menu` `Action.lbtndown=MenuFile`
///   (`Xbox360.ini:268-271`, `EvlChat.ini:244-249`); the system menu's is
///   Xfire's X logo, served the Rekindle mark ([`super::brand`]): Xfire
///   lays its system menu on its X (the INI packs' `[TheX]`, `LOGOX`,
///   `xfire_x.gif` with `MenuSys`), as CleanX's chat `[XfireX]`
///   `Class=Menu` `Graphics.middle=graphics/xfire_x.gif`
///   `Action.lbtndown=MenuSys` (`CleanXChatSkin.ini:186-192`) and Guild
///   Wars UI's `Sysmenu` `Class="Menu"` on its `thex` image with
///   `MenuSys` (`gwcomponents.xml:609-622`);
/// * a tile holding none and wired to nothing, a picture class or a
///   control the pack leaves unwired, is decor: it shows its art, or,
///   its art undefined, lays nothing (spec A8): Wolf ET's own logo
///   `[WolfETLogo]` `Class=Menu` `MouseOver=false` with no action
///   (`Wolf ET infoskin.ini:82-86`), FEAR's `[TIGraphic]` `Class=Tile`
///   naming `infoview/topinset.gif`, which the pack does not hold
///   (`FEARInfoSkin.ini:188-191`).
#[must_use]
pub fn role(f: &Facts<'_>, triggers: &[NodeTrigger]) -> Option<Role> {
    if classless(f) && !f.has_art && !f.children.is_empty() {
        return Some(Role::Shows(NodeRole::Group));
    }
    if f.children.is_empty() && triggers.iter().any(|t| t.run == SkinAction::DragWindow) {
        return Some(Role::Shows(NodeRole::Drag));
    }
    let control = f.class.is_some_and(|c| {
        ["pushbutton", "menu"]
            .iter()
            .any(|k| c.trim().eq_ignore_ascii_case(k))
    });
    let mut pressed = triggers.iter().filter(|t| t.presses()).map(|t| &t.run);
    if control {
        match pressed.clone().find(|r| matches!(r, SkinAction::Menu(_))) {
            Some(SkinAction::Menu(EngineMenu::Sys)) => return Some(Role::Shows(NodeRole::Logo)),
            Some(_) => return Some(Role::Acts(NodeRole::MenuTitle, None)),
            None => {}
        }
    }
    let resizes = pressed.clone().any(|r| matches!(r, SkinAction::Resize(_)));
    if (control && pressed.next().is_some()) || resizes {
        return Some(Role::Acts(NodeRole::Button, None));
    }
    (f.children.is_empty() && triggers.is_empty() && (control || classless(f)))
        .then_some(Role::Shows(NodeRole::Decor))
}
