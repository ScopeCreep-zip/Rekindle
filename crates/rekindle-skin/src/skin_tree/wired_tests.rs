//! SKC3: the roles [`super::wired`] gives a tile the name tables lack, and
//! the name rows the census's unknown components added.

use rekindle_types::dto::skin_action::{NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Edge, NodeRole, SlotKey};

use super::roles::{Facts, Role};
use super::{chat_roles, roles, wired};

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
    // Art of its own is decor ([`roles::role`]); no children is no container.
    assert_eq!(wired::role(&facts(Some("Tile"), true, &kids), &[]), None);
    assert_eq!(wired::role(&facts(Some("Tile"), false, &[]), &[]), None);
    assert_eq!(wired::role(&facts(Some("Text"), false, &kids), &[]), None);
}

#[test]
fn a_childless_tile_dragging_the_window_is_a_drag_strip() {
    let drag = on(SkinEvent::LeftDown, SkinAction::DragWindow);
    assert_eq!(
        wired::role(&facts(Some("Menu"), true, &[]), &drag),
        Some(Role::Shows(NodeRole::Drag))
    );
    assert_eq!(wired::role(&facts(Some("Menu"), true, &[]), &[]), None);
}

#[test]
fn a_wired_push_button_or_grip_is_a_button_run_by_its_trigger() {
    let button = Some(Role::Acts(NodeRole::Button, None));
    let close = on(SkinEvent::Click, SkinAction::Close);
    assert_eq!(
        wired::role(&facts(Some("PushButton"), true, &[]), &close),
        button
    );
    // An unwired push button, or a wired tile of another class, is not one.
    assert_eq!(
        wired::role(&facts(Some("PushButton"), true, &[]), &[]),
        None
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
