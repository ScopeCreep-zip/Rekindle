//! Timer event processing.

use crate::v2::tui::effects::Effect;
use crate::v2::tui::state::TuiState;

pub fn process_tick(state: &mut TuiState) -> Vec<Effect> {
    state.ephemeral.spinner.tick();
    vec![]
}
