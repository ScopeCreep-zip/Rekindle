//! Event calendar input — scroll, RSVP.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::in_flight::RequestKind;
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
            let max = state.communities.events.get(community).map_or(0, |e| e.len().saturating_sub(1));
            state.ephemeral.events_selected = max;
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            let max = state.communities.events.get(community).map_or(0, |e| e.len().saturating_sub(1));
            state.ephemeral.events_selected = (state.ephemeral.events_selected + 1).min(max);
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.ephemeral.events_selected = state.ephemeral.events_selected.saturating_sub(1);
            vec![]
        }
        KeyCode::Enter => {
            if let Some(events) = state.communities.events.get(community) {
                if let Some(ev) = events.get(state.ephemeral.events_selected) {
                    let event_id = ev.get("eventId").or_else(|| ev.get("event_id"))
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if !event_id.is_empty() {
                        let (_, effect) = state.in_flight.track_request(
                            RequestKind::Send,
                            rekindle_types::daemon::DaemonRequest::Chat(
                                rekindle_types::daemon::ChatRequest::EventRsvp {
                                    community: community.to_string(),
                                    event_id,
                                    status: "going".into(),
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
