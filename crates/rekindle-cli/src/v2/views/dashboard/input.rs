//! Dashboard keyboard input — 2x2 grid navigation, quick shortcuts.

use crossterm::event::{KeyCode, KeyEvent};

use super::state::DashboardView;
use crate::v2::tui::action::Action;
use crate::v2::tui::focus::FocusId;

pub fn handle_update(view: &mut DashboardView, action: &Action) -> Option<Action> {
    match action {
        Action::ScrollDown(_) => {
            match view.focus.current() {
                FocusId::DashIdentity => view.focus.set(FocusId::ChannelTree),
                FocusId::DashNode => view.focus.set(FocusId::FriendList),
                _ => {}
            }
        }
        Action::ScrollUp(_) => {
            match view.focus.current() {
                FocusId::ChannelTree => view.focus.set(FocusId::DashIdentity),
                FocusId::FriendList => view.focus.set(FocusId::DashNode),
                _ => {}
            }
        }
        Action::FocusNext => view.focus.next(),
        Action::FocusPrev => view.focus.prev(),
        Action::Back => {
            match view.focus.current() {
                FocusId::DashNode => view.focus.set(FocusId::DashIdentity),
                FocusId::FriendList => view.focus.set(FocusId::ChannelTree),
                _ => {}
            }
        }
        Action::Select => {
            return match view.focus.current() {
                FocusId::DashIdentity => Some(Action::ShowIdentitySettings),
                FocusId::DashNode => Some(Action::ShowDoctor),
                FocusId::ChannelTree => view.communities.first().map(|c| Action::ShowCommunityInfo { community: c.governance_key.clone() }),
                FocusId::FriendList => Some(Action::ShowFriendList),
                _ => None,
            };
        }
        _ => {}
    }
    None
}

pub fn handle_focused_key(view: &mut DashboardView, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('d') => Some(Action::ShowDmInbox),
        KeyCode::Char('f') => Some(Action::ShowFriendList),
        KeyCode::Char('c') => view.communities.first().map(|c| Action::ShowCommunityInfo { community: c.governance_key.clone() }),
        KeyCode::Char('h') => {
            match view.focus.current() {
                FocusId::DashNode => view.focus.set(FocusId::DashIdentity),
                FocusId::FriendList => view.focus.set(FocusId::ChannelTree),
                _ => {}
            }
            None
        }
        KeyCode::Char('l') => {
            match view.focus.current() {
                FocusId::DashIdentity => view.focus.set(FocusId::DashNode),
                FocusId::ChannelTree => view.focus.set(FocusId::FriendList),
                _ => {}
            }
            None
        }
        KeyCode::Enter => {
            match view.focus.current() {
                FocusId::DashIdentity => Some(Action::ShowIdentitySettings),
                FocusId::DashNode => Some(Action::ShowDoctor),
                FocusId::ChannelTree => {
                    view.communities.first().map(|c| Action::ShowCommunityInfo { community: c.governance_key.clone() })
                        .or(Some(Action::ShowCommunityInfo { community: String::new() }))
                }
                FocusId::FriendList => Some(Action::ShowFriendList),
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn handle_click(view: &mut DashboardView, column: u16, row: u16) -> Option<Action> {
    let focus_ids = [FocusId::DashIdentity, FocusId::DashNode, FocusId::ChannelTree, FocusId::FriendList];
    for (rect, &id) in view.panel_rects.iter().zip(&focus_ids) {
        if column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height {
            view.focus.set(id);
            return match id {
                FocusId::DashIdentity => Some(Action::ShowIdentitySettings),
                FocusId::DashNode => Some(Action::ShowDoctor),
                FocusId::ChannelTree => view.communities.first().map(|c| Action::ShowCommunityInfo { community: c.governance_key.clone() }),
                FocusId::FriendList => Some(Action::ShowFriendList),
                _ => None,
            };
        }
    }
    None
}
