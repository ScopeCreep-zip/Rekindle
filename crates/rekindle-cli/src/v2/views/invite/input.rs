//! Invite input — create, revoke, scroll with selection.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::confirm::PendingConfirmAction;
use crate::v2::tui::state::in_flight::RequestKind;
use crate::v2::tui::state::navigation::OverlayState;
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
            let max = state.communities.invites.get(community).map_or(0, |v| v.len().saturating_sub(1));
            state.ephemeral.invite_selected = max;
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max = state.communities.invites.get(community).map_or(0, |v| v.len().saturating_sub(1));
            state.ephemeral.invite_selected = (state.ephemeral.invite_selected + 1).min(max);
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.ephemeral.invite_selected = state.ephemeral.invite_selected.saturating_sub(1);
            vec![]
        }
        KeyCode::Char('n') => {
            let (_, effect) = state.in_flight.track_request(
                RequestKind::Send,
                rekindle_types::daemon::DaemonRequest::Chat(
                    rekindle_types::daemon::ChatRequest::InviteCreate {
                        community: community.to_string(),
                        max_uses: 10,
                        expires_seconds: Some(604800),
                    },
                ),
                state.now,
            );
            vec![effect]
        }
        KeyCode::Char('r') => {
            let selected = state.ephemeral.invite_selected;
            if let Some(invites) = state.communities.invites.get(community) {
                if let Some(inv) = invites.get(selected) {
                    let code = inv.get("code").or_else(|| inv.get("invite_code"))
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if !code.is_empty() {
                        state.nav.confirm.show(
                            format!("Revoke invite {code}?"),
                            "The invite code will no longer work.",
                            PendingConfirmAction::RevokeInvite {
                                community: community.to_string(),
                                invite_code: code,
                            },
                        );
                        state.nav.overlay = Some(OverlayState::Confirm);
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}
