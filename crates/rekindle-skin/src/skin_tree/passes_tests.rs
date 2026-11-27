use rekindle_types::dto::skin_action::{SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Anchor, NodeArt, NodeRole, NodeSpan, ViewKey};

use super::super::tests::{acts, fixed, n};
use super::rows_by_view;

/// Lead decision 12: Alloy's `[ButtonHolder]` slots (`TabAddButton` at
/// X7 stretched, `TabChatRefreshButton` at X53; `Alloy.ini:654-700`)
/// become the Friends and Servers rows, each button at its same place.
#[test]
fn an_ini_packs_button_slots_are_one_row_per_view() {
    let b = |id: &str, a: SkinAction, v: ViewKey| {
        let mut b = acts(
            n(id, NodeRole::Button, fixed(0, 40), fixed(0, 20)),
            SkinEvent::Click,
            a,
        );
        b.shows_with = Some(v);
        b
    };
    let mut add = n(
        "TabAddButton",
        NodeRole::Group,
        NodeSpan::Stretch { start: 7, end: 0 },
        fixed(3, 20),
    );
    add.children = vec![
        b("AddButton", SkinAction::AddFriend, ViewKey::Friends),
        b("AddServerButton", SkinAction::AddServer, ViewKey::Servers),
    ];
    let mut chat = n(
        "TabChatRefreshButton",
        NodeRole::Group,
        fixed(53, 40),
        fixed(3, 20),
    );
    chat.children = vec![
        b("ChatButton", SkinAction::ChatFriend, ViewKey::Friends),
        b("RefreshButton", SkinAction::RefreshServer, ViewKey::Servers),
    ];
    let mut holder = n(
        "ButtonHolder",
        NodeRole::Group,
        NodeSpan::Fixed {
            at: Anchor::centre(0),
            size: 190,
        },
        fixed(0, 26),
    );
    holder.children = vec![add, chat];
    let mut root = vec![holder];
    rows_by_view(&mut root).unwrap();
    let bar = &root[0];
    assert_eq!(bar.role, NodeRole::ButtonBar);
    // Each row holds its views' slots as wrappers (id, span, z, art),
    // each holding that view's button at its own place in the slot.
    let rows: Vec<_> = bar
        .children
        .iter()
        .map(|r| {
            let slots: Vec<_> = r
                .children
                .iter()
                .map(|w| {
                    let kids: Vec<_> = w
                        .children
                        .iter()
                        .map(|c| (c.id.as_str(), c.x.clone(), c.y.clone()))
                        .collect();
                    (
                        w.id.as_str(),
                        w.role,
                        w.shows_with,
                        w.x.clone(),
                        w.y.clone(),
                        kids,
                    )
                })
                .collect();
            (r.role, r.shows_with, slots)
        })
        .collect();
    let add_x = NodeSpan::Stretch { start: 7, end: 0 };
    let own = (fixed(0, 40), fixed(0, 20));
    let slot = |id, v, x, b| {
        (
            id,
            NodeRole::Group,
            Some(v),
            x,
            fixed(3, 20),
            vec![(b, own.0.clone(), own.1.clone())],
        )
    };
    let (f, s) = (ViewKey::Friends, ViewKey::Servers);
    let want = vec![
        (
            NodeRole::ButtonRow(f),
            Some(f),
            vec![
                slot("TabAddButton", f, add_x.clone(), "AddButton"),
                slot("TabChatRefreshButton", f, fixed(53, 40), "ChatButton"),
            ],
        ),
        (
            NodeRole::ButtonRow(s),
            Some(s),
            vec![
                slot("TabAddButton", s, add_x, "AddServerButton"),
                slot("TabChatRefreshButton", s, fixed(53, 40), "RefreshButton"),
            ],
        ),
    ];
    assert_eq!(rows, want);
}

/// Lead decision 13: Combat's centred `ChatRefreshStopTab` (37 px art
/// over 36 px buttons) has no exact place for its button in the row,
/// so the slot stays a wrapper with its own span and art, and the
/// button keeps its place inside it.
#[test]
fn a_centred_slot_stays_a_wrapper_with_its_art() {
    let span = NodeSpan::Fixed {
        at: Anchor::centre(0),
        size: 37,
    };
    let mut slot = n("Slot", NodeRole::Group, span.clone(), fixed(0, 20));
    slot.art = Some(NodeArt::middle("slot.png".into(), 37, 20));
    let mut b = acts(
        n("ChatButton", NodeRole::Button, fixed(0, 36), fixed(0, 20)),
        SkinEvent::Click,
        SkinAction::ChatFriend,
    );
    b.shows_with = Some(ViewKey::Friends);
    slot.children = vec![b];
    let mut root = vec![slot];
    rows_by_view(&mut root).unwrap();
    let w = &root[0].children[0];
    let art = w
        .art
        .as_ref()
        .and_then(|a| a.images().next())
        .map(String::as_str);
    assert_eq!(
        (w.id.as_str(), w.x.clone(), art),
        ("Slot", span, Some("slot.png"))
    );
    assert_eq!(
        (w.children[0].id.as_str(), w.children[0].x.clone()),
        ("ChatButton", fixed(0, 36))
    );
}
