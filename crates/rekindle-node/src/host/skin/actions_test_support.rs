//! Fixtures shared by the skin action tests (`actions_tests`,
//! `actions_routes_tests`).

use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::SkinRun;

use super::actions::{map, Mapped, Selected};
use super::reads::Reads;

/// The friend the fixtures select.
pub(super) const SELECTED_FRIEND_KEY: &str = "bobkey";

/// A selection holding only [`SELECTED_FRIEND_KEY`].
pub(super) fn friend_selected() -> Selected<'static> {
    Selected {
        friend: Some(SELECTED_FRIEND_KEY),
        ..Selected::default()
    }
}

/// The shell run `action` maps to with the friend selected; panics otherwise.
pub(super) fn run(action: &A) -> SkinRun {
    match map(action, friend_selected(), &Reads::default()) {
        Ok(Mapped::Run(r)) => r,
        other => panic!("{action:?}: {other:?}"),
    }
}
