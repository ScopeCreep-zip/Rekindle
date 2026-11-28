//! Call event fixtures for skin tests.

use rekindle_types::subscription_events::CallParticipant;

pub(super) fn call_participants(keys: &[&str]) -> Vec<CallParticipant> {
    keys.iter()
        .map(|&key| CallParticipant {
            public_key: key.into(),
            display_name: format!("Name of {key}"),
            is_self: key == "me",
        })
        .collect()
}
