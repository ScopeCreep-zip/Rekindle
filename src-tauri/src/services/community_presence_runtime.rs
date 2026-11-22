//! The presence sharing consent's Tauri-runtime orchestration, lifted from
//! `commands/community/presence.rs`. Our community status rides our beat
//! (`M`/1) and our shared game our card (`M`/0), both written by the Host
//! (plan E3.4 S2, S6).

use crate::state::SharedState;

/// Read the local presence sharing consent so the settings panel can bind
/// its toggles to the current choice. Defaults (default-deny) when the
/// community isn't loaded.
pub fn get_presence_policy_inner(
    state: &SharedState,
    community_id: &str,
) -> rekindle_types::presence::PresenceSharingPolicy {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|c| c.presence_policy.clone())
        .unwrap_or_default()
}
