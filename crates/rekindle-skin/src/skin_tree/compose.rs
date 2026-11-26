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
/// keeps it.
pub fn community(m: &mut SkinManifest) {
    if m.windows.contains_key(&SkinWindow::Community) {
        return;
    }
    let Some(layout) = source(m).and_then(|w| m.windows.get(&w)).cloned() else {
        return;
    };
    m.windows.insert(SkinWindow::Community, layout);
}
