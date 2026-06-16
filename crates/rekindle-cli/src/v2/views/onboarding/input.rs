//! Onboarding wizard input — step navigation.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    community: &str,
    _caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else { return vec![]; };

    let step_count = state.communities.onboarding.get(community)
        .and_then(|(config, _)| config.as_ref())
        .map_or(0, |c| c.steps.len());

    match key.code {
        KeyCode::Char('n') | KeyCode::Enter | KeyCode::Right => {
            if step_count > 0 && state.ephemeral.onboarding_step + 1 < step_count {
                state.ephemeral.onboarding_step += 1;
            }
            vec![]
        }
        KeyCode::Char('p') | KeyCode::Backspace | KeyCode::Left => {
            state.ephemeral.onboarding_step = state.ephemeral.onboarding_step.saturating_sub(1);
            vec![]
        }
        _ => vec![],
    }
}
