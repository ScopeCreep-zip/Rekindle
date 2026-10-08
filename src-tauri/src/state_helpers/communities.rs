//! Community channel-list write helpers + governance-key collection.

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;

use crate::state::AppState;

/// Replace the entire channel list for a community.
pub fn set_community_channels(
    state: &Arc<AppState>,
    community_id: &str,
    channels: Vec<crate::state::ChannelInfo>,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels = channels;
    }
}

/// Append a single channel to a community's channel list.
pub fn push_community_channel(
    state: &Arc<AppState>,
    community_id: &str,
    channel: crate::state::ChannelInfo,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels.push(channel);
    }
}

/// Our pseudonym public key (hex) in a community, if joined and primed.
///
/// THE accessor for the `communities.read().get(id).and_then(|c|
/// c.my_pseudonym_key.clone())` pattern that was inlined across
/// adapters and runtimes — delegate here instead of re-spelling it.
pub fn my_pseudonym_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())
}

/// Collect communities with governance record keys.
pub fn communities_with_governance_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .communities
        .read()
        .values()
        .filter_map(|c| c.governance_key.as_ref().map(|k| (c.id.clone(), k.clone())))
        .collect()
}

/// Whether `channel_id` is one of the community's stage channels.
pub fn channel_is_stage(state: &Arc<AppState>, community_id: &str, channel_id: &str) -> bool {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|community| community.channels.iter().find(|ch| ch.id == channel_id))
        .is_some_and(|channel| matches!(channel.channel_type, crate::state::ChannelType::Stage))
}

/// The scope of a joined community's tasks: a child of the login scope,
/// created on first use. A closed scope while logged out, for a community
/// that is not joined, or once it was shut down (left), so work spawned
/// for it is dropped (plan C4).
pub fn community_scope(state: &AppState, community_id: &str) -> Arc<SessionScope> {
    let Some(login) = super::login_scope(state) else {
        return SessionScope::closed("community");
    };
    let mut communities = state.communities.write();
    let Some(community) = communities.get_mut(community_id) else {
        return SessionScope::closed("community");
    };
    Arc::clone(
        community
            .tasks
            .get_or_insert_with(|| login.child("community")),
    )
}

/// Spawn `fut` on the community's scope; dropped (and logged) when the
/// community has no live scope.
pub fn spawn_in_community<F>(state: &AppState, community_id: &str, name: &'static str, fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    community_scope(state, community_id).spawn_or_drop(name, fut);
}

/// Spawn a token-watching task on the community's scope; dropped (and
/// logged) when the community has no live scope.
pub fn spawn_in_community_with_token<F, Fut>(
    state: &AppState,
    community_id: &str,
    name: &'static str,
    task: F,
) where
    F: FnOnce(tokio_util::sync::CancellationToken) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    community_scope(state, community_id).spawn_with_token_or_drop(name, task);
}
