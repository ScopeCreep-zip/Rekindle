//! SKC3: the roles [`super::wired`] gives a tile the name tables lack, and
//! the name rows the census's unknown components added.

use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, PackUrl, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Edge, NodeRole, SlotKey, ViewKey};

use super::roles::{Facts, Role};
use super::{chat_roles, roles, wired};
use crate::import::tree::Node;

fn facts<'a>(class: Option<&'a str>, has_art: bool, children: &'a [String]) -> Facts<'a> {
    Facts {
        has_art,
        class,
        children,
    }
}

fn on(on: SkinEvent, run: SkinAction) -> Vec<NodeTrigger> {
    vec![NodeTrigger { on, run }]
}

#[test]
fn a_classless_container_without_art_is_a_group() {
    let kids = ["UserNameText".to_owned(), "TypingMessage".to_owned()];
    let group = Some(Role::Shows(NodeRole::Group));
    assert_eq!(wired::role(&facts(Some("Tile"), false, &kids), &[]), group);
    assert_eq!(wired::role(&facts(None, false, &kids), &[]), group);
    // Art of its own is decor ([`roles::role`]); no children is no
    // container but an unwired tile laying nothing (SKC4).
    assert_eq!(wired::role(&facts(Some("Tile"), true, &kids), &[]), None);
    let decor = Some(Role::Shows(NodeRole::Decor));
    assert_eq!(wired::role(&facts(Some("Tile"), false, &[]), &[]), decor);
    assert_eq!(wired::role(&facts(Some("Text"), false, &kids), &[]), None);
}

#[test]
fn a_childless_tile_dragging_the_window_is_a_drag_strip() {
    let drag = on(SkinEvent::LeftDown, SkinAction::DragWindow);
    assert_eq!(
        wired::role(&facts(Some("Menu"), true, &[]), &drag),
        Some(Role::Shows(NodeRole::Drag))
    );
    // Unwired, it is the pack's picture, never a drag strip (SKC4).
    let decor = Some(Role::Shows(NodeRole::Decor));
    assert_eq!(wired::role(&facts(Some("Menu"), true, &[]), &[]), decor);
}

#[test]
fn a_wired_push_button_or_grip_is_a_button_run_by_its_trigger() {
    let button = Some(Role::Acts(NodeRole::Button, None));
    let close = on(SkinEvent::Click, SkinAction::Close);
    assert_eq!(
        wired::role(&facts(Some("PushButton"), true, &[]), &close),
        button
    );
    // An unwired push button is its art (SKC4), a wired tile of another
    // class is not a button.
    assert_eq!(
        wired::role(&facts(Some("PushButton"), true, &[]), &[]),
        Some(Role::Shows(NodeRole::Decor))
    );
    assert_eq!(wired::role(&facts(Some("Text"), false, &[]), &close), None);
    let grip = on(SkinEvent::LeftDown, SkinAction::Resize(Edge::Left));
    assert_eq!(wired::role(&facts(Some("Tile"), false, &[]), &grip), button);
}

#[test]
fn the_status_arrow_and_typing_text_are_named() {
    let f = facts(Some("Menu"), true, &[]);
    let status = roles::role("StatusArrow", Some("StatusArrow"), None, &f);
    assert_eq!(status, Some(Role::Shows(NodeRole::StatusList)));
    let f = facts(Some("Text"), false, &[]);
    assert_eq!(
        chat_roles::role("TYPING", Some("TYPING"), None, &f),
        Some(Role::Shows(NodeRole::Slot(SlotKey::TypingNotice)))
    );
}

/// SKC4: a `Menu` is a control like a `PushButton` (Mountain Dew's
/// `[MtDewLogo]` pack link); one popping an engine menu is its title
/// (Xbox 360's `[File]` `MenuFile`); the system menu's is Xfire's X logo
/// (CleanX's chat `[XfireX]`, Guild Wars UI's `Sysmenu`, both `MenuSys`).
#[test]
fn a_wired_menu_is_a_button_a_menu_title_or_the_system_menu_logo() {
    let menu = |m| on(SkinEvent::LeftDown, SkinAction::Menu(m));
    let f = facts(Some("Menu"), true, &[]);
    assert_eq!(
        wired::role(&f, &menu(EngineMenu::File)),
        Some(Role::Acts(NodeRole::MenuTitle, None))
    );
    assert_eq!(
        wired::role(
            &facts(Some("PushButton"), true, &[]),
            &menu(EngineMenu::Tool)
        ),
        Some(Role::Acts(NodeRole::MenuTitle, None))
    );
    assert_eq!(
        wired::role(&f, &menu(EngineMenu::Sys)),
        Some(Role::Shows(NodeRole::Logo))
    );
    let link = on(
        SkinEvent::LeftDown,
        SkinAction::LaunchUrl(PackUrl {
            url: "http://www.mountaindew.com/".to_owned(),
        }),
    );
    assert_eq!(
        wired::role(&f, &link),
        Some(Role::Acts(NodeRole::Button, None))
    );
    // A text field is no control: wired or not, it is not decor.
    assert_eq!(wired::role(&facts(Some("Text"), true, &[]), &[]), None);
}

/// SKC4: an unwired tile holding none is decor, its art or (undefined,
/// spec A8) nothing: Wolf ET's `[WolfETLogo]` `Class=Menu`, FEAR's
/// `[TIGraphic]` `Class=Tile` naming a bitmap the pack lacks.
#[test]
fn an_unwired_childless_picture_or_control_is_decor() {
    let decor = Some(Role::Shows(NodeRole::Decor));
    assert_eq!(wired::role(&facts(Some("Menu"), true, &[]), &[]), decor);
    assert_eq!(wired::role(&facts(Some("Tile"), false, &[]), &[]), decor);
    // A component the pack never defines has no role (rule 1).
    assert_eq!(wired::role(&facts(None, false, &[]), &[]), None);
}

/// A component whose `<Text>` face shows `string`.
fn face(string: &str) -> Node {
    let text = Node {
        name: "Text".to_owned(),
        attrs: std::collections::BTreeMap::new(),
        text: string.to_owned(),
        children: Vec::new(),
    };
    Node {
        name: "Component".to_owned(),
        attrs: std::collections::BTreeMap::new(),
        text: String::new(),
        children: vec![text],
    }
}

/// SKC4: Guild Wars UI's `FriendsText` (`<Text>FriendsTitle</Text>` over
/// its `FriendsTab`) and StreamEngine's chat `[WindowTitle]`
/// (`Text.text=ChatTitle`) take the roles their strings name.
#[test]
fn a_text_takes_the_role_its_string_names() {
    let tab = ["FriendsTab".to_owned()];
    let text = |kids| facts(Some("Text"), false, kids);
    let role = |f: &Facts<'_>, s: &str, chat| wired::text_role(f, Some(&face(s)), chat);
    let shows = |r| Some(Role::Shows(r));
    assert_eq!(
        role(&text(&tab), "FriendsTitle", false),
        shows(NodeRole::Tab(ViewKey::Friends))
    );
    assert_eq!(
        role(&text(&tab), "ServerTitle", false),
        shows(NodeRole::Tab(ViewKey::Servers))
    );
    assert_eq!(
        role(&text(&tab), "DownloadTitle", false),
        shows(NodeRole::Tab(ViewKey::Files))
    );
    // A view's title on a text holding no tab button is no tab.
    assert_eq!(role(&text(&[]), "FriendsTitle", false), None);
    // The conversation's title, only in a chat window.
    assert_eq!(
        role(&text(&[]), " ChatTitle ", true),
        shows(NodeRole::ChatTitle)
    );
    assert_eq!(
        role(&text(&[]), "GroupChatTitle", true),
        shows(NodeRole::ChatTitle)
    );
    assert_eq!(role(&text(&[]), "ChatTitle", false), None);
    // Only a text: art showing the same string is not one.
    assert_eq!(
        role(&facts(Some("Tile"), false, &tab), "FriendsTitle", false),
        None
    );
    assert_eq!(role(&text(&[]), "%displayuser%", true), None);
}
