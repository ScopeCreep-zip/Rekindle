mod poll;
pub(crate) mod registry;
mod sync;

use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;

fn current_presence_status(state: &Arc<AppState>) -> &'static str {
    match state_helpers::identity_status(state).unwrap_or(crate::state::UserStatus::Online) {
        crate::state::UserStatus::Online => "online",
        crate::state::UserStatus::Away => "away",
        crate::state::UserStatus::Busy => "busy",
        crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => "offline",
    }
}

pub use poll::{presence_poll_tick_public, start_presence_poll};
