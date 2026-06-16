//! Voice session input — mute, deafen, leave.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use rekindle_types::daemon::{ChatRequest, DaemonRequest};

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::confirm::PendingConfirmAction;
use crate::v2::tui::state::in_flight::RequestKind;
use crate::v2::tui::state::navigation::{OverlayState, ViewKindTag};
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    community: &str,
    channel: &str,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match event {
        TerminalEvent::Key(key) => {
            match key.code {
                KeyCode::Char('m') => {
                    if let Some(ref mut session) = state.voice.active_session {
                        session.muted = !session.muted;
                        let (_, effect) = state.in_flight.track_request(
                            RequestKind::Send,
                            DaemonRequest::Chat(ChatRequest::VoiceMute { muted: session.muted }),
                            state.now,
                        );
                        return vec![effect];
                    }
                    vec![]
                }
                KeyCode::Char('d') => {
                    if let Some(ref mut session) = state.voice.active_session {
                        session.deafened = !session.deafened;
                        let (_, effect) = state.in_flight.track_request(
                            RequestKind::Send,
                            DaemonRequest::Chat(ChatRequest::VoiceDeafen { deafened: session.deafened }),
                            state.now,
                        );
                        return vec![effect];
                    }
                    vec![]
                }
                KeyCode::Char('q') => {
                    state.nav.confirm.show(
                        format!("Leave voice #{channel} in {community}?"),
                        "You will be disconnected.",
                        PendingConfirmAction::LeaveVoice,
                    );
                    state.nav.overlay = Some(OverlayState::Confirm);
                    vec![]
                }
                _ => vec![],
            }
        }
        TerminalEvent::Mouse(mouse) => {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                if let Some(targets) = caches.click_targets.get(&ViewKindTag::VoiceSession) {
                    for (&focus_id, rect) in targets {
                        if mouse.column >= rect.x && mouse.column < rect.x + rect.width
                            && mouse.row >= rect.y && mouse.row < rect.y + rect.height
                        {
                            state.nav.focus_ring.set(focus_id);
                            return vec![];
                        }
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}
