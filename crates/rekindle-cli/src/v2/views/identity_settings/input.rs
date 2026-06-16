//! Identity settings input — copy key to clipboard.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    _caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else { return vec![]; };

    match key.code {
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('y') => {
            if let Some(ref id) = state.ephemeral.identity {
                if !id.public_key.is_empty() {
                    return vec![Effect::SetClipboard(id.public_key.clone())];
                }
            }
            vec![]
        }
        _ => vec![],
    }
}
