//! In-flight request tracking — pending sends, request metadata, credit limits.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use rekindle_types::daemon::DaemonRequest;

#[derive(Clone, Debug)]
pub struct RequestMeta {
    pub kind: RequestKind,
    pub sent_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RequestKind {
    DmInbox,
    DmThread { peer_key: String },
    ChannelHistory { community: String, channel: String },
    CommunityInfo { community: String },
    CommunityList,
    FriendList,
    Status,
    NetworkPeers,
    Identity,
    Invites { community: String },
    Events { community: String },
    Onboarding { community: String },
    ThreadMessages { community: String, thread_id: String },
    Pins { community: String },
    BanList { community: String },
    PendingMembers { community: String },
    Send,
    MarkRead,
    Subscribe,
    Typing,
    Other,
}

#[derive(Clone, Debug)]
pub enum PendingSend {
    ChannelMessage {
        community: String,
        channel: String,
        body: String,
        reply_to: Option<u64>,
    },
    Dm {
        peer_key: String,
        body: String,
    },
}

/// Per-request-type credit counters. Single-threaded — plain u32, no atomics.
#[derive(Debug)]
pub struct InFlightLimits {
    dm_inbox: u32,
    dm_thread: u32,
    channel_history: u32,
    status: u32,
    network_peers: u32,
    community_list: u32,
    community_info: u32,
    identity: u32,
    friend_list: u32,
    send: u32,
    invites: u32,
    events: u32,
    onboarding: u32,
    thread_messages: u32,
    pins: u32,
    ban_list: u32,
    pending_members: u32,
}

impl InFlightLimits {
    pub fn new() -> Self {
        Self {
            dm_inbox: 0,
            dm_thread: 0,
            channel_history: 0,
            status: 0,
            network_peers: 0,
            community_list: 0,
            community_info: 0,
            identity: 0,
            friend_list: 0,
            send: 0,
            invites: 0,
            events: 0,
            onboarding: 0,
            thread_messages: 0,
            pins: 0,
            ban_list: 0,
            pending_members: 0,
        }
    }

    #[must_use = "credit check result must not be ignored"]
    pub fn try_reserve(&mut self, kind: &RequestKind) -> bool {
        match self.counter_and_ceiling(kind) {
            Some((counter, ceiling)) => {
                if *counter >= ceiling {
                    return false;
                }
                *counter += 1;
                true
            }
            None => true,
        }
    }

    pub fn release(&mut self, kind: &RequestKind) {
        if let Some((counter, _)) = self.counter_and_ceiling(kind) {
            *counter = counter.saturating_sub(1);
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Returns `None` for uncapped fire-and-forget kinds.
    fn counter_and_ceiling(&mut self, kind: &RequestKind) -> Option<(&mut u32, u32)> {
        match kind {
            RequestKind::DmInbox => Some((&mut self.dm_inbox, 1)),
            RequestKind::DmThread { .. } => Some((&mut self.dm_thread, 2)),
            RequestKind::ChannelHistory { .. } => Some((&mut self.channel_history, 1)),
            RequestKind::Status => Some((&mut self.status, 1)),
            RequestKind::NetworkPeers => Some((&mut self.network_peers, 1)),
            RequestKind::CommunityList => Some((&mut self.community_list, 1)),
            RequestKind::CommunityInfo { .. } => Some((&mut self.community_info, 1)),
            RequestKind::Identity => Some((&mut self.identity, 1)),
            RequestKind::FriendList => Some((&mut self.friend_list, 1)),
            RequestKind::Send => Some((&mut self.send, 4)),
            RequestKind::Invites { .. } => Some((&mut self.invites, 1)),
            RequestKind::Events { .. } => Some((&mut self.events, 1)),
            RequestKind::Onboarding { .. } => Some((&mut self.onboarding, 1)),
            RequestKind::ThreadMessages { .. } => Some((&mut self.thread_messages, 1)),
            RequestKind::Pins { .. } => Some((&mut self.pins, 1)),
            RequestKind::BanList { .. } => Some((&mut self.ban_list, 1)),
            RequestKind::PendingMembers { .. } => Some((&mut self.pending_members, 1)),
            RequestKind::MarkRead | RequestKind::Subscribe
            | RequestKind::Typing | RequestKind::Other => None,
        }
    }
}

impl Default for InFlightLimits {
    fn default() -> Self {
        Self::new()
    }
}

/// In-flight request tracking. `requests` and `pending_sends` are private —
/// access through methods to preserve the invariant that request_ids are unique
/// and pending_sends keys are a subset of requests keys for Send kinds.
#[derive(Debug)]
pub struct InFlightState {
    next_request_id: u64,
    requests: HashMap<u64, RequestMeta>,
    pending_sends: HashMap<u64, PendingSend>,
    credits: InFlightLimits,
    /// Requests deferred because their credit type was at ceiling.
    /// Drained when credit becomes available.
    deferred: VecDeque<(u64, DaemonRequest)>,
}

impl InFlightState {
    pub fn new() -> Self {
        Self {
            next_request_id: 1,
            requests: HashMap::new(),
            pending_sends: HashMap::new(),
            credits: InFlightLimits::new(),
            deferred: VecDeque::new(),
        }
    }

    /// Internal-only ID allocator. Not pub — callers use track_send or track_request.
    fn next_id(&mut self) -> u64 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    #[must_use = "credit check result must not be ignored"]
    pub fn try_reserve(&mut self, kind: &RequestKind) -> bool {
        self.credits.try_reserve(kind)
    }

    pub fn release_credit(&mut self, kind: &RequestKind) {
        self.credits.release(kind);
    }

    /// Atomic *tracking* for send requests: allocates request_id, inserts
    /// RequestMeta with kind=Send, and inserts PendingSend — all in one call.
    ///
    /// Does NOT reserve credit. Credit reservation happens in `execute_effects`
    /// (machine.rs) when the returned `Effect::IpcRequest` is processed. This
    /// separation exists because `execute_effects` handles deferral — if credit
    /// is at ceiling, the effect is pushed to the deferred queue instead of
    /// being sent immediately.
    ///
    /// The lifecycle: track_send creates tracking → caller returns Effect →
    /// execute_effects reserves credit or defers → spawn_ipc_request sends →
    /// response arrives → process_command_result removes tracking + releases credit.
    pub fn track_send(
        &mut self,
        pending: PendingSend,
        request: DaemonRequest,
        now: Instant,
    ) -> (u64, super::super::effects::Effect) {
        let id = self.next_id();
        self.requests.insert(id, RequestMeta {
            kind: RequestKind::Send,
            sent_at: now,
        });
        self.pending_sends.insert(id, pending);
        (id, super::super::effects::Effect::IpcRequest {
            request_id: id,
            request,
        })
    }

    /// Atomic *tracking* for non-send requests: allocates request_id,
    /// inserts RequestMeta, returns the Effect.
    ///
    /// Does NOT reserve credit — same lifecycle as `track_send`.
    /// See `track_send` doc for the full credit lifecycle explanation.
    pub fn track_request(
        &mut self,
        kind: RequestKind,
        request: DaemonRequest,
        now: Instant,
    ) -> (u64, super::super::effects::Effect) {
        let id = self.next_id();
        self.requests.insert(id, RequestMeta { kind, sent_at: now });
        (id, super::super::effects::Effect::IpcRequest {
            request_id: id,
            request,
        })
    }

    pub fn remove_request(&mut self, id: u64) -> Option<RequestMeta> {
        self.requests.remove(&id)
    }

    /// Peek at a request's kind without removing it. For tracing/logging only.
    pub fn peek_request_kind(&self, id: u64) -> Option<&RequestKind> {
        self.requests.get(&id).map(|m| &m.kind)
    }

    /// Returns true if any in-flight request matches this kind.
    #[must_use]
    pub fn has_request_of_kind(&self, kind: &RequestKind) -> bool {
        self.requests.values().any(|m| m.kind == *kind)
    }

    pub fn remove_pending_send(&mut self, id: u64) -> Option<PendingSend> {
        self.pending_sends.remove(&id)
    }

    pub fn push_deferred(&mut self, id: u64, request: DaemonRequest) {
        self.deferred.push_back((id, request));
    }

    pub fn pop_deferred(&mut self) -> Option<(u64, DaemonRequest)> {
        self.deferred.pop_front()
    }

    pub fn peek_deferred(&self) -> Option<&(u64, DaemonRequest)> {
        self.deferred.front()
    }

    #[must_use]
    pub fn has_deferred(&self) -> bool {
        !self.deferred.is_empty()
    }

    /// Iterate all in-flight requests. Used by deadline checks for timeout detection.
    pub fn requests_iter(&self) -> impl Iterator<Item = (u64, &RequestMeta)> {
        self.requests.iter().map(|(&id, meta)| (id, meta))
    }

    /// Drain all pending_sends entries. Used by reconnect invalidation to
    /// fail each one individually.
    pub fn drain_pending_sends(&mut self) -> impl Iterator<Item = (u64, PendingSend)> + '_ {
        self.pending_sends.drain()
    }

    /// Clears requests, credits, and deferred queue. pending_sends are
    /// NOT cleared — the caller drains them via `drain_pending_sends`
    /// to fail each message individually.
    pub fn clear_all(&mut self) {
        self.requests.clear();
        self.credits.reset();
        self.deferred.clear();
    }
}

impl Default for InFlightState {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a DaemonRequest to its RequestKind for credit tracking.
pub fn request_kind_from(request: &DaemonRequest) -> RequestKind {
    use rekindle_types::daemon::{ChatRequest, LifecycleRequest};
    match request {
        DaemonRequest::Lifecycle(l) => match l {
            LifecycleRequest::Status => RequestKind::Status,
            LifecycleRequest::NetworkPeers => RequestKind::NetworkPeers,
            LifecycleRequest::Subscribe { .. } => RequestKind::Subscribe,
            LifecycleRequest::Unsubscribe { .. } => RequestKind::Subscribe,
            LifecycleRequest::Unlock { .. } => RequestKind::Other,
            LifecycleRequest::Lock => RequestKind::Other,
            LifecycleRequest::Shutdown => RequestKind::Other,
            LifecycleRequest::NetworkStatus => RequestKind::Other,
            LifecycleRequest::AgentRegister { .. } => RequestKind::Other,
            LifecycleRequest::AgentRevoke { .. } => RequestKind::Other,
            LifecycleRequest::PolicyReload => RequestKind::Other,
            LifecycleRequest::BulkTransferStart { .. } => RequestKind::Other,
            LifecycleRequest::BulkTransferComplete { .. } => RequestKind::Other,
            LifecycleRequest::BulkTransferCancel { .. } => RequestKind::Other,
            LifecycleRequest::BulkTransferStatus { .. } => RequestKind::Other,
            LifecycleRequest::EventResume { .. } => RequestKind::Other,
        },
        DaemonRequest::Chat(c) => match c {
            ChatRequest::DmInbox { .. } => RequestKind::DmInbox,
            ChatRequest::DmThread { peer_key, .. } => RequestKind::DmThread { peer_key: peer_key.clone() },
            ChatRequest::DmSend { .. } => RequestKind::Send,
            ChatRequest::DmTyping { .. } => RequestKind::Typing,
            ChatRequest::DmStart { .. } => RequestKind::Send,
            ChatRequest::DmAccept { .. } => RequestKind::Send,
            ChatRequest::ChannelHistory { community, channel, .. } => {
                RequestKind::ChannelHistory { community: community.clone(), channel: channel.clone() }
            }
            ChatRequest::ChannelSend { .. } => RequestKind::Send,
            ChatRequest::ChannelTyping { .. } => RequestKind::Typing,
            ChatRequest::ChannelList { .. } => RequestKind::Other,
            ChatRequest::ChannelCreate { .. } => RequestKind::Send,
            ChatRequest::ChannelDelete { .. } => RequestKind::Send,
            ChatRequest::ChannelUpdate { .. } => RequestKind::Send,
            ChatRequest::CommunityList => RequestKind::CommunityList,
            ChatRequest::CommunityInfo { governance_key } => {
                RequestKind::CommunityInfo { community: governance_key.clone() }
            }
            ChatRequest::CommunityCreate { .. } => RequestKind::Send,
            ChatRequest::CommunityJoin { .. } => RequestKind::Send,
            ChatRequest::CommunityLeave { .. } => RequestKind::Send,
            ChatRequest::CommunityApprove { .. } => RequestKind::Send,
            ChatRequest::CommunityReject { .. } => RequestKind::Send,
            ChatRequest::CommunityPendingMembers { governance_key } => RequestKind::PendingMembers { community: governance_key.clone() },
            ChatRequest::CommunityTransferOwnership { .. } => RequestKind::Send,
            ChatRequest::FriendList => RequestKind::FriendList,
            ChatRequest::FriendRequests => RequestKind::Other,
            ChatRequest::FriendAdd { .. } => RequestKind::Send,
            ChatRequest::FriendAccept { .. } => RequestKind::Send,
            ChatRequest::FriendReject { .. } => RequestKind::Send,
            ChatRequest::FriendRemove { .. } => RequestKind::Send,
            ChatRequest::IdentityCreate { .. } => RequestKind::Send,
            ChatRequest::IdentityShow => RequestKind::Identity,
            ChatRequest::IdentityExport => RequestKind::Other,
            ChatRequest::IdentityRotate => RequestKind::Send,
            ChatRequest::IdentityDestroy { .. } => RequestKind::Send,
            ChatRequest::IdentityWipe { .. } => RequestKind::Send,
            ChatRequest::IdentityExportEncrypted { .. } => RequestKind::Other,
            ChatRequest::IdentityImportEncrypted { .. } => RequestKind::Send,
            ChatRequest::IdentityImport { .. } => RequestKind::Send,
            ChatRequest::MarkRead { .. } => RequestKind::MarkRead,
            ChatRequest::MessageEdit { .. } => RequestKind::Send,
            ChatRequest::MessageDelete { .. } => RequestKind::Send,
            ChatRequest::MekList { .. } => RequestKind::Other,
            ChatRequest::MekRotate { .. } => RequestKind::Send,
            ChatRequest::MekRequest { .. } => RequestKind::Send,
            ChatRequest::PrekeyReplenish => RequestKind::Send,
            ChatRequest::PresenceSet { .. } => RequestKind::Send,
            ChatRequest::GamePresenceSet { .. } => RequestKind::Send,
            ChatRequest::GamePresenceClear => RequestKind::Send,
            ChatRequest::RoleList { .. } => RequestKind::Other,
            ChatRequest::RoleCreate { .. } => RequestKind::Send,
            ChatRequest::RoleUpdate { .. } => RequestKind::Send,
            ChatRequest::RoleDelete { .. } => RequestKind::Send,
            ChatRequest::RoleAssign { .. } => RequestKind::Send,
            ChatRequest::RoleUnassign { .. } => RequestKind::Send,
            ChatRequest::Kick { .. } => RequestKind::Send,
            ChatRequest::Ban { .. } => RequestKind::Send,
            ChatRequest::Unban { .. } => RequestKind::Send,
            ChatRequest::Timeout { .. } => RequestKind::Send,
            ChatRequest::BanList { community } => RequestKind::BanList { community: community.clone() },
            ChatRequest::InviteCreate { .. } => RequestKind::Send,
            ChatRequest::InviteList { community } => RequestKind::Invites { community: community.clone() },
            ChatRequest::InviteRevoke { .. } => RequestKind::Send,
            ChatRequest::ReactionAdd { .. } => RequestKind::Send,
            ChatRequest::ReactionRemove { .. } => RequestKind::Send,
            ChatRequest::PinAdd { .. } => RequestKind::Send,
            ChatRequest::PinRemove { .. } => RequestKind::Send,
            ChatRequest::PinList { community } => RequestKind::Pins { community: community.clone() },
            ChatRequest::EventCreate { .. } => RequestKind::Send,
            ChatRequest::EventUpdate { .. } => RequestKind::Send,
            ChatRequest::EventDelete { .. } => RequestKind::Send,
            ChatRequest::EventRsvp { .. } => RequestKind::Send,
            ChatRequest::EventRemind { .. } => RequestKind::Send,
            ChatRequest::EventList { community } => RequestKind::Events { community: community.clone() },
            ChatRequest::ThreadCreate { .. } => RequestKind::Send,
            ChatRequest::ThreadMessage { .. } => RequestKind::Send,
            ChatRequest::ThreadSend { .. } => RequestKind::Send,
            ChatRequest::ThreadArchive { .. } => RequestKind::Send,
            ChatRequest::ThreadList { .. } => RequestKind::Other,
            ChatRequest::ThreadHistory { thread_id, .. } => RequestKind::ThreadMessages { community: String::new(), thread_id: thread_id.clone() },
            ChatRequest::ReactionList { .. } => RequestKind::Other,
            ChatRequest::AuditLog { .. } => RequestKind::Other,
            ChatRequest::OnboardingConfigGet { community } => RequestKind::Onboarding { community: community.clone() },
            ChatRequest::OnboardingConfigSet { .. } => RequestKind::Send,
            ChatRequest::WelcomeScreenGet { community } => RequestKind::Onboarding { community: community.clone() },
            ChatRequest::WelcomeScreenSet { .. } => RequestKind::Send,
            ChatRequest::GameServerAdd { .. } => RequestKind::Send,
            ChatRequest::GameServerRemove { .. } => RequestKind::Send,
            ChatRequest::SystemAnnounce { .. } => RequestKind::Send,
            ChatRequest::RaidAlert { .. } => RequestKind::Send,
            ChatRequest::LockdownToggle { .. } => RequestKind::Send,
            ChatRequest::KickNotify { .. } => RequestKind::Send,
            ChatRequest::BootstrapRequest { .. } => RequestKind::Send,
            ChatRequest::BootstrapRespond { .. } => RequestKind::Send,
            ChatRequest::SyncRequest { .. } => RequestKind::Send,
            ChatRequest::SyncRespond { .. } => RequestKind::Send,
            ChatRequest::VoiceJoin { .. } => RequestKind::Send,
            ChatRequest::VoiceLeave => RequestKind::Send,
            ChatRequest::VoiceMute { .. } => RequestKind::Send,
            ChatRequest::VoiceDeafen { .. } => RequestKind::Send,
        },
    }
}
