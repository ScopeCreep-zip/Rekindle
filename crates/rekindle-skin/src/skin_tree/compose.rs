//! The Community window's layout (plan SK17-6, `SkinWindow::Community`).
//! Xfire never drew a Community window, so the Host composes it from the
//! pack's own windows instead of drawing a generic one: the pack's room
//! window (`<GroupChatSkin>`), else its conversation window, else its buddy
//! list. No importer writes a room layout yet (SK17-2/-3 bring the
//! `GroupChatWindow` frame), so today every built-in's Community window
//! wears its chat window's frame, title strip, window buttons, borders and
//! menus: the same frame its group-chat window wears (the group chat is a
//! chat window, `windows::open_chat_window`). The shell places its
//! community panes in that tree's slots, as the group chat places its own
//! controls.
//!
//! The composition is a Host placement, like the Communities view in the
//! main tree ([`super::place::communities`]): it is never written into a
//! package, and every read of a package composes again.

use rekindle_types::dto::skin::{SkinManifest, SkinWindow};
use rekindle_types::dto::skin_tree::{Anchor, NodeRole, NodeSpan, SkinNode, SlotKey};

/// The windows the Community window is composed from, in order.
pub const SOURCES: [SkinWindow; 2] = [SkinWindow::Chat, SkinWindow::Main];

/// The window `m`'s Community layout is composed from: the first of
/// [`SOURCES`] it lays, `None` when it lays none.
#[must_use]
pub fn source(m: &SkinManifest) -> Option<SkinWindow> {
    SOURCES.into_iter().find(|w| m.windows.contains_key(w))
}

/// Give `m` its Community window: the chosen source's served layout, whole
/// (frame tiles, title strip, outsets, content box, minimum, tree and its
/// popups), so the Community window and the source window draw the same
/// nodes from the same art. A pack that lays its own Community window
/// keeps it. Composed from a buddy list, the buddy list's own nodes are
/// left out ([`strip_buddy_list`]).
pub fn community(m: &mut SkinManifest) {
    if m.windows.contains_key(&SkinWindow::Community) {
        return;
    }
    let Some(from) = source(m) else {
        return;
    };
    let Some(mut layout) = m.windows.get(&from).cloned() else {
        return;
    };
    if from == SkinWindow::Main {
        if let Some(tree) = layout.tree.as_mut() {
            strip_buddy_list(&mut tree.root);
        }
    }
    m.windows.insert(SkinWindow::Community, layout);
}

/// The roles only a buddy list lays (plan SK17-6, SKW2): the ad slot
/// (`AdArea`, Total Chaos's `[AdBar]`), the view tabs and dial, the views
/// and their button rows (Friends/Servers/Files/Communities), the name and
/// status fields, the InfoView and the list's column header. A Community
/// window keeps what every window of the pack wears: the frame, the title
/// strip and its window buttons, the resize grips, the menus and the
/// pack's own decor and links.
#[must_use]
pub fn buddy_list_only(role: NodeRole) -> bool {
    matches!(
        role,
        NodeRole::AdArea
            | NodeRole::TabStrip
            | NodeRole::Tab(_)
            | NodeRole::Knob(_)
            | NodeRole::Views
            | NodeRole::View(_)
            | NodeRole::ButtonRow(_)
            | NodeRole::NameField
            | NodeRole::StatusText
            | NodeRole::StatusList
            | NodeRole::InfoView
            | NodeRole::ColumnHeader
    )
}

/// Leaves a buddy list's own nodes out of `nodes`. The group holding the
/// views is the window's content area: its children (the views, the tab
/// strip, the expand button) become the community content ([`content`])
/// filling it. Every other buddy-list-only node keeps its box, empty, so
/// the siblings laid after it (`Anchor::After`) stay where the pack put
/// them.
pub fn strip_buddy_list(nodes: &mut [SkinNode]) {
    for n in nodes {
        if let Some(views) = n.children.iter().find(|c| c.role == NodeRole::Views) {
            n.children = content(views);
        } else if n.role == NodeRole::Views {
            let children = content(n);
            empty(n);
            n.children = children;
        } else if buddy_list_only(n.role) {
            empty(n);
        } else {
            strip_buddy_list(&mut n.children);
        }
    }
}

/// The composer's height in the content area: the chat composer every
/// chat-laid built-in serves (`SlotKey::ChatEdit`, 50 px in Symbiosis and
/// xSLIMFIRE, `community_tests::the_community_content_slot_reaches_the_composer`).
pub const COMPOSER_HEIGHT: u16 = 50;

/// The community content in place of the buddy list's views, filling its
/// parent as a chat window's content does: the conversation slot
/// (`SlotKey::ChatArea`, where the group chat draws its messages) above
/// the composer (`SlotKey::ChatEdit`), so the composer is drawn inside the
/// pack's content area like every chat-laid pack's.
fn content(views: &SkinNode) -> Vec<SkinNode> {
    vec![
        slot(
            views.id.clone(),
            SlotKey::ChatArea,
            NodeSpan::Stretch {
                start: 0,
                end: COMPOSER_HEIGHT.cast_signed(),
            },
            views.z,
        ),
        slot(
            format!("{}ChatEdit", views.id),
            SlotKey::ChatEdit,
            NodeSpan::Fixed {
                at: Anchor::End(0),
                size: COMPOSER_HEIGHT,
            },
            views.z,
        ),
    ]
}

fn slot(id: String, key: SlotKey, y: NodeSpan, z: i16) -> SkinNode {
    SkinNode {
        id,
        role: NodeRole::Slot(key),
        x: NodeSpan::Stretch { start: 0, end: 0 },
        y,
        z,
        art: None,
        text: None,
        label: None,
        inset: None,
        target: None,
        fit: None,
        shows_with: None,
        control_group: None,
        triggers: Vec::new(),
        children: Vec::new(),
        disabled_by: None,
    }
}

fn empty(n: &mut SkinNode) {
    n.role = NodeRole::Group;
    n.art = None;
    n.text = None;
    n.label = None;
    n.inset = None;
    n.target = None;
    n.fit = None;
    n.shows_with = None;
    n.control_group = None;
    n.triggers.clear();
    n.children.clear();
}
