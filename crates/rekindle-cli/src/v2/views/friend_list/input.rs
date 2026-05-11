//! Friend list input — scroll, accept/reject requests, open DM.

use crossterm::event::{KeyCode, KeyEvent};

use super::FriendListView;
use crate::v2::tui::action::Action;

pub fn handle_update(view: &mut FriendListView, action: &Action) -> Option<Action> {
    match action {
        Action::ScrollDown(_) => {
            let max = build_visual_count(view).saturating_sub(1);
            let i = view.list_state.selected().unwrap_or(0);
            view.list_state.select(Some((i + 1).min(max)));
        }
        Action::ScrollUp(_) => {
            let i = view.list_state.selected().unwrap_or(0);
            view.list_state.select(Some(i.saturating_sub(1)));
        }
        Action::ScrollToTop => { view.list_state.select(Some(0)); }
        Action::ScrollToBottom => {
            let max = build_visual_count(view).saturating_sub(1);
            view.list_state.select(Some(max));
        }
        _ => {}
    }
    None
}

pub fn handle_focused_key(view: &mut FriendListView, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            let max = build_visual_count(view).saturating_sub(1);
            let i = view.list_state.selected().unwrap_or(0);
            view.list_state.select(Some((i + 1).min(max)));
            None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let i = view.list_state.selected().unwrap_or(0);
            view.list_state.select(Some(i.saturating_sub(1)));
            None
        }
        KeyCode::Enter => {
            let visual_idx = view.list_state.selected()?;
            let friend_idx = visual_to_friend_index(&view.friends, visual_idx)?;
            let friend = view.friends.get(friend_idx)?;
            Some(Action::ShowDmThread { peer_key: friend.public_key.clone() })
        }
        KeyCode::Char('a') => {
            let visual_idx = view.list_state.selected()?;
            let pending_idx = visual_to_pending_index(&view.friends, &view.pending_requests, visual_idx)?;
            let request = view.pending_requests.get(pending_idx)?;
            Some(Action::AcceptFriendRequest(request.public_key.clone()))
        }
        KeyCode::Char('r') => {
            let visual_idx = view.list_state.selected()?;
            let pending_idx = visual_to_pending_index(&view.friends, &view.pending_requests, visual_idx)?;
            let request = view.pending_requests.get(pending_idx)?;
            Some(Action::RejectFriendRequest(request.public_key.clone()))
        }
        _ => None,
    }
}

fn build_visual_count(view: &FriendListView) -> usize {
    let mut count = 0;
    let mut current_status: Option<&str> = None;
    for friend in &view.friends {
        if current_status != Some(friend.status.as_str()) {
            current_status = Some(friend.status.as_str());
            count += 1;
        }
        count += 1;
    }
    if !view.pending_requests.is_empty() {
        count += 2 + view.pending_requests.len();
    }
    count
}

fn visual_to_friend_index(friends: &[rekindle_types::display::FriendDisplay], visual_idx: usize) -> Option<usize> {
    let mut current_status: Option<&str> = None;
    let mut visual = 0usize;
    for (friend_count, friend) in friends.iter().enumerate() {
        if current_status != Some(friend.status.as_str()) {
            current_status = Some(friend.status.as_str());
            if visual == visual_idx { return None; }
            visual += 1;
        }
        if visual == visual_idx { return Some(friend_count); }
        visual += 1;
    }
    None
}

fn visual_to_pending_index(
    friends: &[rekindle_types::display::FriendDisplay],
    pending: &[super::PendingRequestDisplay],
    visual_idx: usize,
) -> Option<usize> {
    let mut visual = 0usize;
    let mut current_status: Option<&str> = None;
    for friend in friends {
        if current_status != Some(friend.status.as_str()) {
            current_status = Some(friend.status.as_str());
            visual += 1;
        }
        visual += 1;
    }
    if pending.is_empty() { return None; }
    visual += 2;
    for (i, _) in pending.iter().enumerate() {
        if visual == visual_idx { return Some(i); }
        visual += 1;
    }
    None
}
