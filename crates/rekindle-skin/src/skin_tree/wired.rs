//! The role of a tile neither name table knows, from what the pack itself
//! says the tile is and does (SKC3; ruling 3 of
//! `design-skin-catalogue-2026-11-26.md`: "component with no role: extend
//! the role table from the component's class and function"). It is asked
//! only after [`super::chat_roles::role`] and [`super::roles::role`] find
//! none, so no tile they name changes, and it reads no pack's name.

use rekindle_types::dto::skin_action::{NodeTrigger, SkinAction};
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
///   `Action.lbtndown=ResizeL`).
#[must_use]
pub fn role(f: &Facts<'_>, triggers: &[NodeTrigger]) -> Option<Role> {
    if classless(f) && !f.has_art && !f.children.is_empty() {
        return Some(Role::Shows(NodeRole::Group));
    }
    if f.children.is_empty() && triggers.iter().any(|t| t.run == SkinAction::DragWindow) {
        return Some(Role::Shows(NodeRole::Drag));
    }
    let push = f
        .class
        .is_some_and(|c| c.trim().eq_ignore_ascii_case("PushButton"));
    let pressed = triggers.iter().any(NodeTrigger::presses);
    let resizes = triggers
        .iter()
        .any(|t| t.presses() && matches!(t.run, SkinAction::Resize(_)));
    ((push && pressed) || resizes).then_some(Role::Acts(NodeRole::Button, None))
}
