//! Community info input — scroll channels, navigate, moderation/invite/events shortcuts.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::confirm::PendingConfirmAction;
use crate::v2::tui::state::navigation::{OverlayState, ViewKind};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    community: &str,
    _caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else { return vec![]; };

    match key.code {
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('G') => {
            if let Some(detail) = state.communities.details.get(community) {
                if !detail.channels.is_empty() {
                    state.communities.selected_channel.insert(
                        community.to_string(), detail.channels.len() - 1,
                    );
                }
            }
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(detail) = state.communities.details.get(community) {
                if !detail.channels.is_empty() {
                    let current = state.communities.selected_channel
                        .get(community).copied().unwrap_or(0);
                    let new = (current + 1).min(detail.channels.len() - 1);
                    state.communities.selected_channel.insert(community.to_string(), new);
                }
            }
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let current = state.communities.selected_channel
                .get(community).copied().unwrap_or(0);
            state.communities.selected_channel.insert(community.to_string(), current.saturating_sub(1));
            vec![]
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            let selected = state.communities.selected_channel
                .get(community).copied().unwrap_or(0);
            if let Some(detail) = state.communities.details.get(community) {
                if let Some(ch) = detail.channels.get(selected) {
                    return vec![Effect::Navigate(ViewKind::ChannelWatch {
                        community: community.to_string(),
                        channel: ch.name.clone(),
                    })];
                }
            }
            vec![]
        }
        KeyCode::Char('m') => {
            vec![Effect::Navigate(ViewKind::Moderation { community: community.to_string() })]
        }
        KeyCode::Char('i') => {
            vec![Effect::Navigate(ViewKind::Invite { community: community.to_string() })]
        }
        KeyCode::Char('e') => {
            vec![Effect::Navigate(ViewKind::Events { community: community.to_string() })]
        }
        KeyCode::Char('L') => {
            let name = state.communities.name_for(community);
            state.nav.confirm.show(
                format!("Leave {name}?"),
                "You will lose access to all channels.",
                PendingConfirmAction::LeaveCommunity {
                    community: community.to_string(),
                },
            );
            state.nav.overlay = Some(OverlayState::Confirm);
            vec![]
        }
        _ => vec![],
    }
}
