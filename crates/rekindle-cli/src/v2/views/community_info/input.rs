//! Community info input — j/k scroll, Enter opens channel.

use crossterm::event::{KeyCode, KeyEvent};

use super::CommunityInfoView;
use crate::v2::tui::action::Action;

pub fn handle_update(view: &mut CommunityInfoView, action: &Action) -> Option<Action> {
    match action {
        Action::Refresh => {
            view.loading = true;
            return Some(Action::ShowCommunityInfo { community: view.community.clone() });
        }
        Action::ScrollDown(_) => {
            if let Some(ref detail) = view.detail {
                if !detail.channels.is_empty() {
                    view.selected_channel = (view.selected_channel + 1).min(detail.channels.len() - 1);
                }
            }
        }
        Action::ScrollUp(_) => {
            view.selected_channel = view.selected_channel.saturating_sub(1);
        }
        Action::Select => {
            if let Some(ref detail) = view.detail {
                if let Some(ch) = detail.channels.get(view.selected_channel) {
                    return Some(Action::ShowChannel {
                        community: view.community.clone(), channel: ch.name.clone(),
                    });
                }
            }
        }
        _ => {}
    }
    None
}

pub fn handle_focused_key(view: &mut CommunityInfoView, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(ref detail) = view.detail {
                if !detail.channels.is_empty() {
                    view.selected_channel = (view.selected_channel + 1).min(detail.channels.len() - 1);
                }
            }
            None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            view.selected_channel = view.selected_channel.saturating_sub(1);
            None
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            view.detail.as_ref().and_then(|detail| {
                detail.channels.get(view.selected_channel).map(|ch| {
                    Action::ShowChannel { community: view.community.clone(), channel: ch.name.clone() }
                })
            })
        }
        _ => None,
    }
}
