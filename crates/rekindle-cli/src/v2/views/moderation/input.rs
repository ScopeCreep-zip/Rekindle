//! Moderation input — tab switch, scroll, kick/ban/timeout, approve/reject, unban.

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
            let max = current_list_len(state, community).saturating_sub(1);
            state.ephemeral.moderation_selected = max;
            vec![]
        }
        KeyCode::Tab => {
            state.ephemeral.moderation_tab = (state.ephemeral.moderation_tab + 1) % 3;
            state.ephemeral.moderation_selected = 0;
            vec![]
        }
        KeyCode::Char('1') => { state.ephemeral.moderation_tab = 0; state.ephemeral.moderation_selected = 0; vec![] }
        KeyCode::Char('2') => { state.ephemeral.moderation_tab = 1; state.ephemeral.moderation_selected = 0; vec![] }
        KeyCode::Char('3') => { state.ephemeral.moderation_tab = 2; state.ephemeral.moderation_selected = 0; vec![] }

        KeyCode::Char('j') | KeyCode::Down => {
            let max = current_list_len(state, community).saturating_sub(1);
            state.ephemeral.moderation_selected = (state.ephemeral.moderation_selected + 1).min(max);
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.ephemeral.moderation_selected = state.ephemeral.moderation_selected.saturating_sub(1);
            vec![]
        }

        _ => match state.ephemeral.moderation_tab {
            0 => handle_members_key(key.code, state, community),
            1 => handle_bans_key(key.code, state, community),
            2 => handle_queue_key(key.code, state, community),
            _ => vec![],
        },
    }
}

fn current_list_len(state: &TuiState, community: &str) -> usize {
    match state.ephemeral.moderation_tab {
        0 => state.communities.members.get(community).map_or(0, |m| m.len()),
        1 => state.communities.bans.get(community).map_or(0, |b| b.len()),
        2 => state.communities.pending_members.get(community).map_or(0, |p| p.len()),
        _ => 0,
    }
}

fn handle_members_key(code: KeyCode, state: &mut TuiState, community: &str) -> Vec<Effect> {
    let selected = state.ephemeral.moderation_selected;
    let member = state.communities.members.get(community)
        .and_then(|m| m.get(selected));

    match code {
        KeyCode::Char('K') => {
            if let Some(m) = member {
                state.nav.confirm.show(
                    format!("Kick {}?", m.member.display_name),
                    "They will be removed but can rejoin.",
                    PendingConfirmAction::KickMember {
                        community: community.to_string(),
                        pseudonym: m.member.pseudonym_key.clone(),
                    },
                );
                state.nav.overlay = Some(OverlayState::Confirm);
            }
            vec![]
        }
        KeyCode::Char('b') => {
            if let Some(m) = member {
                state.nav.confirm.show(
                    format!("Ban {}?", m.member.display_name),
                    "They will be permanently banned.",
                    PendingConfirmAction::BanMember {
                        community: community.to_string(),
                        pseudonym: m.member.pseudonym_key.clone(),
                    },
                );
                state.nav.overlay = Some(OverlayState::Confirm);
            }
            vec![]
        }
        KeyCode::Char('t') => {
            if let Some(m) = member {
                state.nav.confirm.show(
                    format!("Timeout {} for 5 minutes?", m.member.display_name),
                    "They will be unable to send messages.",
                    PendingConfirmAction::TimeoutMember {
                        community: community.to_string(),
                        pseudonym: m.member.pseudonym_key.clone(),
                        duration_secs: 300,
                    },
                );
                state.nav.overlay = Some(OverlayState::Confirm);
            }
            vec![]
        }
        _ => vec![],
    }
}

fn handle_bans_key(code: KeyCode, state: &mut TuiState, community: &str) -> Vec<Effect> {
    match code {
        KeyCode::Char('u') => {
            let selected = state.ephemeral.moderation_selected;
            if let Some(bans) = state.communities.bans.get(community) {
                if let Some(ban) = bans.get(selected) {
                    let pseudo = ban.get("pseudonymKey").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if !pseudo.is_empty() {
                        let (_, effect) = state.in_flight.track_request(
                            RequestKind::Send,
                            rekindle_types::daemon::DaemonRequest::Chat(
                                rekindle_types::daemon::ChatRequest::Unban {
                                    community: community.to_string(),
                                    target_pseudonym: pseudo,
                                },
                            ),
                            state.now,
                        );
                        return vec![effect];
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}

fn handle_queue_key(code: KeyCode, state: &mut TuiState, community: &str) -> Vec<Effect> {
    let selected = state.ephemeral.moderation_selected;
    let entry = state.communities.pending_members.get(community)
        .and_then(|p| p.get(selected));

    match code {
        KeyCode::Char('a') => {
            if let Some(entry) = entry {
                let pseudo = entry.get("requesterPseudonymHex")
                    .or_else(|| entry.get("requester_pseudonym_hex"))
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = entry.get("displayName")
                    .or_else(|| entry.get("display_name"))
                    .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                if !pseudo.is_empty() {
                    state.nav.confirm.show(
                        format!("Approve {name}?"),
                        "They will gain full community access.",
                        PendingConfirmAction::ApproveMember {
                            community: community.to_string(),
                            pseudonym: pseudo,
                        },
                    );
                    state.nav.overlay = Some(OverlayState::Confirm);
                }
            }
            vec![]
        }
        KeyCode::Char('r') => {
            if let Some(entry) = entry {
                let pseudo = entry.get("requesterPseudonymHex")
                    .or_else(|| entry.get("requester_pseudonym_hex"))
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = entry.get("displayName")
                    .or_else(|| entry.get("display_name"))
                    .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                if !pseudo.is_empty() {
                    state.nav.confirm.show(
                        format!("Reject {name}?"),
                        "Their join request will be removed.",
                        PendingConfirmAction::RejectMember {
                            community: community.to_string(),
                            pseudonym: pseudo,
                        },
                    );
                    state.nav.overlay = Some(OverlayState::Confirm);
                }
            }
            vec![]
        }
        _ => vec![],
    }
}
