//! Internal event processing — navigation, toasts, quit.

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::InternalEvent;
use crate::v2::tui::state::TuiState;

pub fn process_internal(event: &InternalEvent, state: &mut TuiState) -> Vec<Effect> {
    match event {
        InternalEvent::Toast { message, level } => {
            state.ephemeral.toasts.push(message.clone(), *level, state.now);
            vec![]
        }
    }
}
