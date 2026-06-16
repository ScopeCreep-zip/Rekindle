//! Per-view data requirements — maps ViewKind to IPC requests needed.

use std::time::{Duration, Instant};

use rekindle_types::daemon::{ChatRequest, DaemonRequest, LifecycleRequest};
use rekindle_types::subscription_events::SubscriptionFilter;

use super::effects::Effect;
use super::state::channels::ChannelKey;
use super::state::in_flight::{InFlightState, RequestKind};
use super::state::navigation::ViewKind;
use super::state::TuiState;

/// One data requirement for a view.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum DataRequirement {
    Status,
    NetworkPeers,
    CommunityList,
    Identity,
    FriendList,
    DmInbox { limit: u32 },
    DmThread { peer_key: String, limit: u32 },
    ChannelHistory { community: String, channel: String, limit: u32 },
    CommunityInfo { community: String },
    MarkRead { context: rekindle_types::daemon::ReadContext },
    Subscribe { filters: Vec<SubscriptionFilter> },
    BanList { community: String },
    PendingMembers { community: String },
    Invites { community: String },
    Events { community: String },
    Onboarding { community: String },
}

/// Compute data requirements for a view, filtered by staleness and loaded state.
pub fn required_data(view: &ViewKind, state: &TuiState) -> Vec<DataRequirement> {
    tracing::debug!(view = ?view, "tui: computing data requirements");
    match view {
        ViewKind::Dashboard => {
            let mut reqs = vec![DataRequirement::Status];
            if stale(state.communities.list_loaded_at, 30, state.now) {
                reqs.push(DataRequirement::CommunityList);
            }
            if stale(state.friends.loaded_at, 30, state.now) {
                reqs.push(DataRequirement::FriendList);
            }
            reqs.push(DataRequirement::Identity);
            reqs.push(DataRequirement::NetworkPeers);
            // Load DM inbox at startup so threads are populated before
            // the user navigates to the DMs tab. Without this, subscription
            // events for DMs that arrive while on Dashboard have no thread
            // state to push into.
            if !state.dm.inbox_loaded || stale(state.dm.inbox_loaded_at, 30, state.now) {
                reqs.push(DataRequirement::DmInbox { limit: 50 });
            }
            reqs
        }

        ViewKind::DmInbox => {
            let mut reqs = vec![];
            if !state.dm.inbox_loaded || stale(state.dm.inbox_loaded_at, 30, state.now) {
                reqs.push(DataRequirement::DmInbox { limit: 50 });
            }
            let target_peer = state.session.dm_selected_peer.as_deref()
                .or_else(|| state.dm.threads.keys().next().map(|k| k.as_str()));
            if let Some(peer_key) = target_peer {
                if let Some(thread) = state.dm.threads.get(peer_key) {
                    if !thread.loaded && !thread.loading {
                        reqs.push(DataRequirement::DmThread { peer_key: peer_key.to_string(), limit: 50 });
                    }
                }
            }
            reqs
        }

        ViewKind::DmThread { peer_key } => {
            let mut reqs = vec![];
            match state.dm.threads.get(peer_key) {
                Some(thread) if !thread.loaded && !thread.loading => {
                    reqs.push(DataRequirement::DmThread { peer_key: peer_key.clone(), limit: 50 });
                }
                None => {
                    if !state.dm.inbox_loaded {
                        reqs.push(DataRequirement::DmInbox { limit: 50 });
                    }
                    reqs.push(DataRequirement::DmThread { peer_key: peer_key.clone(), limit: 50 });
                }
                _ => {}
            }
            reqs.push(DataRequirement::MarkRead {
                context: rekindle_types::daemon::ReadContext::Dm { peer: peer_key.clone() },
            });
            reqs
        }

        ViewKind::ChannelWatch { community, channel } => {
            let key = ChannelKey { community: community.clone(), channel: channel.clone() };
            let mut reqs = vec![];
            let needs_history = state.channels.channels.get(&key)
                .map_or(true, |ch| !ch.loaded || stale(ch.loaded_at, 60, state.now));
            if needs_history {
                reqs.push(DataRequirement::ChannelHistory {
                    community: community.clone(),
                    channel: channel.clone(),
                    limit: 50,
                });
            }
            if !state.communities.details.contains_key(community) {
                reqs.push(DataRequirement::CommunityInfo { community: community.clone() });
            }
            reqs.push(DataRequirement::Subscribe {
                filters: vec![SubscriptionFilter::community(community.clone())],
            });
            reqs.push(DataRequirement::MarkRead {
                context: rekindle_types::daemon::ReadContext::Channel {
                    community: community.clone(),
                    channel: channel.clone(),
                },
            });
            reqs
        }

        ViewKind::FriendList => {
            if !state.friends.loaded || stale(state.friends.loaded_at, 30, state.now) {
                vec![DataRequirement::FriendList]
            } else {
                vec![]
            }
        }

        ViewKind::Doctor => {
            vec![DataRequirement::Status, DataRequirement::Identity]
        }

        ViewKind::CommunityInfo { community } => {
            if !state.communities.details.contains_key(community) {
                vec![DataRequirement::CommunityInfo { community: community.clone() }]
            } else {
                vec![]
            }
        }

        ViewKind::IdentitySettings => {
            vec![DataRequirement::Status, DataRequirement::Identity]
        }

        ViewKind::Moderation { community } => {
            vec![
                DataRequirement::CommunityInfo { community: community.clone() },
                DataRequirement::BanList { community: community.clone() },
                DataRequirement::PendingMembers { community: community.clone() },
            ]
        }

        ViewKind::Invite { community } => {
            vec![DataRequirement::Invites { community: community.clone() }]
        }

        ViewKind::Events { community } => {
            vec![DataRequirement::Events { community: community.clone() }]
        }

        ViewKind::Onboarding { community } => {
            vec![DataRequirement::Onboarding { community: community.clone() }]
        }

        ViewKind::VoiceSession { .. } => vec![],
        ViewKind::FilePreview { .. } => vec![],
    }
}

/// Convert requirements to effects. Skips if the same RequestKind is already in flight.
pub fn requirements_to_effects(
    reqs: Vec<DataRequirement>,
    in_flight: &mut InFlightState,
    now: Instant,
) -> Vec<Effect> {
    let mut effects = vec![];
    for req in reqs {
        let kind = requirement_to_kind(&req);
        if in_flight.has_request_of_kind(&kind) {
            tracing::debug!(kind = ?kind, "tui: data requirement SKIPPED (already in flight)");
            continue;
        }
        let request = requirement_to_request(req);
        match request {
            RequirementEffect::Ipc(daemon_req) => {
                tracing::info!(kind = ?kind, "tui: data requirement → IPC request");
                let (_, effect) = in_flight.track_request(kind, daemon_req, now);
                effects.push(effect);
            }
            RequirementEffect::Subscribe(filters) => {
                tracing::debug!(filter_count = filters.len(), "tui: data requirement → Subscribe");
                effects.push(Effect::IpcSubscribe { filters });
            }
        }
    }
    effects
}

fn stale(loaded_at: Option<Instant>, max_age_secs: u64, now: Instant) -> bool {
    loaded_at.map_or(true, |t| now.duration_since(t) > Duration::from_secs(max_age_secs))
}

fn requirement_to_kind(req: &DataRequirement) -> RequestKind {
    match req {
        DataRequirement::Status => RequestKind::Status,
        DataRequirement::NetworkPeers => RequestKind::NetworkPeers,
        DataRequirement::CommunityList => RequestKind::CommunityList,
        DataRequirement::Identity => RequestKind::Identity,
        DataRequirement::FriendList => RequestKind::FriendList,
        DataRequirement::DmInbox { .. } => RequestKind::DmInbox,
        DataRequirement::DmThread { peer_key, .. } => RequestKind::DmThread { peer_key: peer_key.clone() },
        DataRequirement::ChannelHistory { community, channel, .. } => {
            RequestKind::ChannelHistory { community: community.clone(), channel: channel.clone() }
        }
        DataRequirement::CommunityInfo { community } => RequestKind::CommunityInfo { community: community.clone() },
        DataRequirement::MarkRead { .. } => RequestKind::MarkRead,
        DataRequirement::Subscribe { .. } => RequestKind::Subscribe,
        DataRequirement::BanList { community } => RequestKind::BanList { community: community.clone() },
        DataRequirement::PendingMembers { community } => RequestKind::PendingMembers { community: community.clone() },
        DataRequirement::Invites { community } => RequestKind::Invites { community: community.clone() },
        DataRequirement::Events { community } => RequestKind::Events { community: community.clone() },
        DataRequirement::Onboarding { community } => RequestKind::Onboarding { community: community.clone() },
    }
}

enum RequirementEffect {
    Ipc(DaemonRequest),
    Subscribe(Vec<SubscriptionFilter>),
}

fn requirement_to_request(req: DataRequirement) -> RequirementEffect {
    match req {
        DataRequirement::Status => RequirementEffect::Ipc(DaemonRequest::Lifecycle(LifecycleRequest::Status)),
        DataRequirement::NetworkPeers => RequirementEffect::Ipc(DaemonRequest::Lifecycle(LifecycleRequest::NetworkPeers)),
        DataRequirement::CommunityList => RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::CommunityList)),
        DataRequirement::Identity => RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::IdentityShow)),
        DataRequirement::FriendList => RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::FriendList)),
        DataRequirement::DmInbox { limit } => RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::DmInbox { limit })),
        DataRequirement::DmThread { peer_key, limit } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::DmThread { peer_key, limit }))
        }
        DataRequirement::ChannelHistory { community, channel, limit } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::ChannelHistory { community, channel, limit }))
        }
        DataRequirement::CommunityInfo { community } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::CommunityInfo { governance_key: community }))
        }
        DataRequirement::MarkRead { context } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::MarkRead { context }))
        }
        DataRequirement::Subscribe { filters } => RequirementEffect::Subscribe(filters),
        DataRequirement::BanList { community } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::BanList { community }))
        }
        DataRequirement::PendingMembers { community } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::CommunityPendingMembers { governance_key: community }))
        }
        DataRequirement::Invites { community } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::InviteList { community }))
        }
        DataRequirement::Events { community } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::EventList { community }))
        }
        DataRequirement::Onboarding { community } => {
            RequirementEffect::Ipc(DaemonRequest::Chat(ChatRequest::OnboardingConfigGet { community }))
        }
    }
}
