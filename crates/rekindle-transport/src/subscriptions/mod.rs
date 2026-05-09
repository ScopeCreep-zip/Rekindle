//! Consolidated subscription module — every inbound signal from Veilid.
//!
//! `SubscriptionManager` is the **sole owner** of all reactive state:
//! DHT watches, gossip processing, typing/presence/unread tracking,
//! and event emission. No other code in the workspace establishes
//! watches, processes ValueChange events, or maintains unread counts.
//!
//! # Submodules
//!
//! - `events/` — typed event enums per domain (channel, typing, presence, etc.)
//! - `state` — mutable state: unread, typing, presence, voice
//! - `watches` — DHT watch lifecycle: create, renew, route ValueChange
//! - `gossip_handler` — inbound gossip processing (all 52 ControlPayload variants)
//! - `dm_handler` — inbound DM processing (all 10 DmPayload variants)

pub mod events;
pub mod state;
pub mod watches;
pub mod state_effects;
pub mod dispatch;
pub mod dedup;
pub mod poll;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::crypto::mek::MekCache;
use crate::gossip::GossipMesh;
use crate::broadcast::node::TransportNode;
use crate::payload::dm::DmPayload;
use crate::payload::gossip::GossipPayload;
use crate::session::{CommunityMembership, Session};

use events::SubscriptionEvent;
use state::SubscriptionState;
use watches::WatchRegistry;

/// Broadcast channel capacity for subscription events.
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// Centralized subscription manager.
///
/// Owns all inbound reactive state. Created once during daemon resume.
/// The daemon's `InboundHandler` forwards every signal here. Consumers
/// (TUI, CLI) subscribe via `subscribe()` and receive typed events.
pub struct SubscriptionManager {
    /// Transport node for DHT operations.
    node: Arc<TransportNode>,
    /// Session state (identity, communities, DM keys).
    session: Arc<RwLock<Option<Session>>>,
    /// MEK cache for channel decryption context.
    mek_cache: Arc<RwLock<MekCache>>,
    /// Signal Protocol session manager for DM decryption in enrichment spawns.
    signal: Arc<RwLock<Option<crate::crypto::signal_session::SignalSessionManager>>>,
    /// Mutable state: unread counts, typing, presence, voice.
    state: Arc<RwLock<SubscriptionState>>,
    /// Active DHT watches registry.
    watches: Arc<RwLock<WatchRegistry>>,
    /// Cross-tier Blake3 content deduplication.
    dedup: Arc<RwLock<dedup::EventDedup>>,
    /// Per-community gossip mesh (shared with BroadcastManager).
    meshes: Arc<RwLock<HashMap<String, GossipMesh>>>,
    /// Broadcast sender for subscription events.
    event_tx: broadcast::Sender<SubscriptionEvent>,
    /// Handle for the background watch renewal task.
    renewal_handle: Option<JoinHandle<()>>,
    /// Shutdown signal for the renewal loop.
    renewal_shutdown_tx: Option<mpsc::Sender<()>>,
    /// Handle for the background poll loop (tier 3).
    poll_handle: Option<JoinHandle<()>>,
    /// Shutdown signal for the poll loop.
    poll_shutdown_tx: Option<mpsc::Sender<()>>,
}

impl SubscriptionManager {
    /// Create a new subscription manager. Does NOT start background tasks.
    ///
    /// Call `setup_identity()` and `setup_community()` to begin watching.
    /// Call `start_renewal_loop()` to enable automatic watch renewal.
    pub fn new(
        node: Arc<TransportNode>,
        session: Arc<RwLock<Option<Session>>>,
        mek_cache: Arc<RwLock<MekCache>>,
        signal: Arc<RwLock<Option<crate::crypto::signal_session::SignalSessionManager>>>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            node,
            session,
            mek_cache,
            signal,
            state: Arc::new(RwLock::new(SubscriptionState::default())),
            watches: Arc::new(RwLock::new(WatchRegistry::new())),
            dedup: Arc::new(RwLock::new(dedup::EventDedup::default())),
            meshes: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            renewal_handle: None,
            renewal_shutdown_tx: None,
            poll_handle: None,
            poll_shutdown_tx: None,
        }
    }

    /// Subscribe to all subscription events. Multiple subscribers supported.
    /// Dropping the receiver auto-unsubscribes.
    pub fn subscribe(&self) -> broadcast::Receiver<SubscriptionEvent> {
        self.event_tx.subscribe()
    }

    /// Access the broadcast sender for passing to the IPC event delivery system.
    ///
    /// The IPC server uses this to subscribe internally and route events
    /// through the EventRouter to connected clients.
    pub fn event_sender(&self) -> &broadcast::Sender<SubscriptionEvent> {
        &self.event_tx
    }

    /// Start the background watch renewal loop.
    ///
    /// Renews DHT watches every 60 seconds. Must be called after construction.
    pub fn start_renewal_loop(&mut self) {
        let (tx, rx) = mpsc::channel(1);
        let handle = tokio::spawn(watches::run_renewal_loop(
            Arc::clone(&self.node),
            Arc::clone(&self.watches),
            self.event_tx.clone(),
            rx,
        ));
        self.renewal_handle = Some(handle);
        self.renewal_shutdown_tx = Some(tx);
        info!("subscription watch renewal loop started");
    }

    /// Start the background poll loop (tier 3 fallback).
    ///
    /// Sweeps all watched DHT records every `interval_secs` with force_refresh.
    /// When changes are found, emits `ValueChanged` events through the broadcast
    /// channel. The daemon-internal consumer acts on these to trigger `process_inbox`
    /// and friend inbox scans — completing the tier 3 guarantee for daemon actions.
    pub fn start_poll_loop(&mut self, interval_secs: u64) {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let (change_tx, mut change_rx) = mpsc::channel::<(String, Vec<u32>)>(64);

        // Spawn the poll loop (reads DHT with force_refresh, signals changes)
        let handle = tokio::spawn(poll::run_poll_loop(
            Arc::clone(&self.node),
            Arc::clone(&self.watches),
            interval_secs,
            change_tx,
            shutdown_rx,
        ));

        // Spawn the change consumer (routes poll signals into the event pipeline)
        let event_tx = self.event_tx.clone();
        let dedup = Arc::clone(&self.dedup);
        tokio::spawn(async move {
            while let Some((record_key, changed_subkeys)) = change_rx.recv().await {
                let event = SubscriptionEvent::Network(
                    events::NetworkEvent::ValueChanged {
                        record_key,
                        changed_subkeys,
                    },
                );
                // Dedup gate — suppresses if this change was already seen via watch
                if dedup.write().check(&event) {
                    let _ = event_tx.send(event);
                }
            }
        });

        self.poll_handle = Some(handle);
        self.poll_shutdown_tx = Some(shutdown_tx);
        info!(interval_secs, "poll loop started (tier 3 fallback)");
    }

    /// Shut down: stop all background loops, clear all state.
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.renewal_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.renewal_handle.take() {
            let _ = h.await;
        }
        if let Some(tx) = self.poll_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.poll_handle.take() {
            let _ = h.await;
        }
        self.watches.write().entries.clear();
        info!("subscription manager shut down");
    }

    // ── Setup / Teardown ───────────────────────────────────────────────

    /// Establish all identity-level watches (friend inbox, DM logs).
    ///
    /// Called once after identity is loaded and session is resumed.
    /// Idempotent — safe to call multiple times.
    pub async fn setup_identity(&self, session: &Session) {
        watches::setup_identity_watches(&self.node, &self.watches, session).await;
        info!(
            friend_inbox = %session.identity.friend_inbox_key,
            dm_peers = session.dm_peers.len(),
            "identity watches established"
        );
    }

    /// Establish all community-level watches (governance, registry, inbox).
    ///
    /// Called for each community during resume and on community join.
    pub async fn setup_community(&self, membership: &CommunityMembership) {
        info!(community = %membership.community_name, governance = %membership.governance_key, "sub: setup_community");
        watches::setup_community_watches(&self.node, &self.watches, membership).await;
        // Establish per-member channel log watches for real-time message delivery.
        // Reads registry to discover all members' DhtLog spine keys.
        watches::setup_channel_watches(
            &self.node, &self.watches,
            &membership.governance_key, &membership.registry_key,
            &membership.channel_record_keys,
        ).await;
        self.meshes.write().entry(membership.governance_key.clone())
            .or_insert_with(|| GossipMesh::new(membership.governance_key.clone()));
    }

    /// Remove all watches and state for a community.
    pub fn teardown_community(&self, governance_key: &str) {
        self.watches.write().remove_community(governance_key);
        self.meshes.write().remove(governance_key);
        self.state.write().unread.remove_community(governance_key);
        self.state.write().typing.remove_community(governance_key);
        self.state.write().presence.remove_community(governance_key);
        self.state.write().voice.remove_community(governance_key);
        debug!(governance_key, "community subscriptions torn down");
    }

    /// Set up a DM peer watch.
    pub async fn setup_dm_peer(&self, peer_key: &str, dm_log_key: &str) {
        debug!(peer = &peer_key[..12.min(peer_key.len())], dm_log_key, "sub: setup_dm_peer");
        watches::setup_dm_watch(&self.node, &self.watches, peer_key, dm_log_key).await;
    }

    /// Remove DM watch and state for a peer.
    pub fn teardown_dm_peer(&self, peer_key: &str) {
        debug!(peer = &peer_key[..12.min(peer_key.len())], "sub: teardown_dm_peer");
        self.watches.write().remove_dm_peer(peer_key);
        self.state.write().unread.remove_dm_peer(peer_key);
        self.state.write().typing.remove_dm_peer(peer_key);
        self.state.write().presence.remove_dm_peer(peer_key);
    }

    // ── Inbound event routing ──────────────────────────────────────────

    /// Maximum allowed Lamport clock drift above local value.
    /// Rejects gossip from peers claiming impossibly far-future timestamps.
    const MAX_LAMPORT_DRIFT: u64 = 10_000;

    /// Route a gossip payload. Called by the daemon's InboundHandler.
    ///
    /// Pipeline: drift check → payload.into_event() → state_effects → dedup → emit
    pub fn on_gossip(
        &self, community_id: &str, sender_pseudonym: &str,
        payload: GossipPayload, lamport_ts: u64,
    ) {
        // Lamport drift rejection — prevents clock corruption from malicious peers
        let local_ts = self.meshes.read().get(community_id).map_or(0, |m| m.clock.value());
        if lamport_ts > local_ts + Self::MAX_LAMPORT_DRIFT {
            warn!(
                community = community_id,
                sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
                received = lamport_ts, local = local_ts,
                drift = lamport_ts - local_ts,
                "rejecting gossip: Lamport clock drift exceeds maximum"
            );
            return;
        }

        debug!(community = community_id, sender = &sender_pseudonym[..12.min(sender_pseudonym.len())], lamport = lamport_ts, "sub: on_gossip");
        let event = payload.into_event(community_id, sender_pseudonym);
        self.process_event(event);
    }

    /// Route a DM payload. Called by the daemon's InboundHandler.
    ///
    /// Pipeline: payload.into_event() → state_effects → dedup → emit
    pub fn on_dm(&self, sender_key: &str, payload: DmPayload) {
        debug!(sender = &sender_key[..12.min(sender_key.len())], "sub: on_dm");
        let event = payload.into_event(sender_key);
        self.process_event(event);
    }

    /// Central event processing pipeline: enrich → state effects → dedup → emit.
    ///
    /// Single point of emission for ALL events regardless of source tier
    /// (watch, gossip, poll, or direct construction in on_value_change).
    fn process_event(&self, mut event: SubscriptionEvent) {
        // Enrich: decrypt message bodies if MEK available, resolve display names
        self.enrich(&mut event);

        // Apply state side-effects (unread, typing, presence, voice)
        let extra_events = state_effects::apply(&mut self.state.write(), &event);

        // Dedup gate: suppress duplicates from parallel tiers
        if self.dedup.write().check(&event) {
            let _ = self.event_tx.send(event);
        }

        // Emit any additional events (e.g., UnreadChanged)
        for extra in extra_events {
            let _ = self.event_tx.send(extra);
        }
    }

    /// Enrich an event with decrypted message bodies and resolved display names.
    ///
    /// Channel messages: attempt MEK decrypt if body is None.
    /// DMs: resolve sender_name from session friend list.
    /// All other events: no-op.
    fn enrich(&self, event: &mut SubscriptionEvent) {
        match event {
            SubscriptionEvent::ChannelMessage(
                events::ChannelMessageEvent::New { community, body, .. }
            ) if body.is_none() => {
                // Attempt decrypt from MEK cache.
                // The full decrypt requires reading the ciphertext from DHT and
                // decrypting with the cached MEK. For gossip-originated events,
                // the ciphertext is not in the event — it's in the DHT record.
                // The enrichment reads the channel DhtLog entry and decrypts.
                //
                // For now: leave body as None. The TUI will show the message
                // metadata (sender, timestamp) and the body will be populated
                // on the next history load or when the poll tier refreshes.
                // Full inline decrypt is wired when QueryEngine is accessible here.
                let _ = community; // suppress unused warning until decrypt is wired
            }
            SubscriptionEvent::ChannelMessage(
                events::ChannelMessageEvent::DirectMessageReceived { peer_key, sender_name, .. }
            ) if sender_name.is_none() => {
                // Resolve display name from friend list in session
                let guard = self.session.read();
                if let Some(ref session) = *guard {
                    if let Some(request) = session.pending_friend_requests.iter()
                        .find(|r| r.public_key == *peer_key)
                    {
                        *sender_name = Some(request.display_name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// Route a DHT ValueChange by record key.
    pub fn on_value_change(
        &self, record_key: &str, changed_subkeys: Vec<u32>,
        _first_value: Option<Vec<u8>>,
    ) {
        let watch_kind = self.watches.read().get(record_key).map(|e| e.kind.clone());
        let Some(kind) = watch_kind else {
            self.process_event(SubscriptionEvent::Network(
                events::NetworkEvent::ValueChanged {
                    record_key: record_key.into(),
                    changed_subkeys,
                },
            ));
            return;
        };

        match kind {
            watches::WatchKind::FriendInbox => {
                debug!(record_key, subkeys = ?changed_subkeys, "friend inbox changed");
                // Update pending count from session
                let pending_count = self.session.read().as_ref()
                    .map_or(0, |s| u32::try_from(s.pending_friend_requests.len()).unwrap_or(u32::MAX));
                self.state.write().unread.friend_requests = pending_count;
                let count = pending_count;
                self.process_event(SubscriptionEvent::UnreadChanged {
                    context: events::UnreadContext::FriendRequests,
                    count,
                });
            }
            watches::WatchKind::DmLog { peer_key } => {
                debug!(peer = %peer_key, "DM log changed");

                // Don't emit body-less notification here — we can't know synchronously
                // whether this change is from us or the peer (shared DhtLog).
                // The enrichment spawn below reads the log entry, determines is_self,
                // and emits the event with correct attribution + body content.
                // Unread increment is also deferred to the spawn to avoid counting
                // our own sent messages as unread.

                // Spawn async DhtLog read to populate body and re-emit with content.
                let dm_log_key = self.session.read().as_ref()
                    .and_then(|s| s.dm_peers.get(&peer_key).map(|p| p.inbound_log_key.clone()))
                    .filter(|k| !k.is_empty());
                if let Some(log_key) = dm_log_key {
                    let node = Arc::clone(&self.node);
                    let event_tx = self.event_tx.clone();
                    let session = Arc::clone(&self.session);
                    let state = Arc::clone(&self.state);
                    let signal = Arc::clone(&self.signal);
                    let peer_key_owned = peer_key;
                    tokio::spawn(async move {
                        let Ok(dht) = node.dht() else { return };
                        let log = match crate::broadcast::dht::channel_log::DhtLog::open_read(
                            dht.routing_context(), &log_key,
                        ).await {
                            Ok(l) => l,
                            Err(e) => {
                                tracing::debug!(error = %e, "DM body enrich: DhtLog open failed");
                                return;
                            }
                        };
                        let entries = match log.tail(1).await {
                            Ok(e) if !e.is_empty() => e,
                            _ => return,
                        };
                        let entry: serde_json::Value = match serde_json::from_slice(&entries[0]) {
                            Ok(v) => v,
                            Err(_) => return,
                        };
                        let sender_key = entry.get("sender_key")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let body_hex = entry.get("body")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let body = hex::decode(body_hex)
                            .ok()
                            .and_then(|ciphertext| {
                                let guard = signal.read();
                                guard.as_ref().and_then(|mgr| {
                                    mgr.decrypt(&sender_key, &ciphertext)
                                        .map_err(|e| tracing::debug!(
                                            error = %e,
                                            peer = &sender_key[..12.min(sender_key.len())],
                                            "Signal DM decrypt failed in enrichment"
                                        ))
                                        .ok()
                                })
                            })
                            .and_then(|plaintext| String::from_utf8(plaintext).ok())
                            .unwrap_or_default();
                        let timestamp = entry.get("timestamp")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        // Resolve sender display name from local session cache.
                        // 1. friend_display_names (populated on accept — no network I/O)
                        // 2. pending_friend_requests (pre-accept fallback)
                        let sender_name = {
                            let guard = session.read();
                            guard.as_ref().and_then(|s| {
                                s.friend_display_names.get(&sender_key).cloned()
                                    .or_else(|| {
                                        s.pending_friend_requests.iter()
                                            .find(|r| r.public_key == sender_key)
                                            .map(|r| r.display_name.clone())
                                    })
                            })
                        };
                        let is_self = {
                            let guard = session.read();
                            guard.as_ref().is_some_and(|s| s.identity.public_key_hex == sender_key)
                        };
                        if !body.is_empty() {
                            // Increment unread only for peer-sent messages, not our own
                            if !is_self {
                                let count = state.write().unread.increment_dm(&peer_key_owned);
                                let _ = event_tx.send(SubscriptionEvent::UnreadChanged {
                                    context: events::UnreadContext::Dm { peer_key: peer_key_owned.clone() },
                                    count,
                                });
                            }
                            let _ = event_tx.send(SubscriptionEvent::ChannelMessage(
                                events::ChannelMessageEvent::DirectMessageReceived {
                                    peer_key: peer_key_owned,
                                    timestamp,
                                    sender_name,
                                    body: Some(body),
                                    is_self,
                                },
                            ));
                        }
                    });
                }
            }
            watches::WatchKind::GovernanceManifest { community } => {
                for subkey in &changed_subkeys {
                    let event = match *subkey {
                        crate::payload::dht_types::MANIFEST_METADATA =>
                            events::GovernanceEvent::MetadataChanged { community: community.clone() },
                        crate::payload::dht_types::MANIFEST_CHANNELS =>
                            events::GovernanceEvent::ChannelsChanged { community: community.clone() },
                        crate::payload::dht_types::MANIFEST_ROLES =>
                            events::GovernanceEvent::RolesChanged { community: community.clone() },
                        crate::payload::dht_types::MANIFEST_BANS =>
                            events::GovernanceEvent::BansChanged { community: community.clone() },
                        crate::payload::dht_types::MANIFEST_INVITES =>
                            events::GovernanceEvent::InvitesChanged { community: community.clone() },
                        other => events::GovernanceEvent::GovernanceSubkeyUpdated {
                            community: community.clone(), subkey_index: other, lamport_ts: 0,
                        },
                    };
                    self.process_event(SubscriptionEvent::Governance(event));
                }
            }
            watches::WatchKind::MemberRegistry { community } => {
                for subkey in &changed_subkeys {
                    match *subkey {
                        crate::payload::dht_types::REGISTRY_MEMBER_INDEX => {
                            debug!(community = %community, "member index changed — refreshing channel watches");
                            // Look up registry_key from session for this community
                            let registry_key = self.session.read().as_ref()
                                .and_then(|s| s.communities.get(&community))
                                .map(|m| m.registry_key.clone());
                            if let Some(reg_key) = registry_key {
                                // Spawn async watch setup — on_value_change is synchronous.
                                // setup_channel_watches is idempotent (skips existing watches)
                                // so repeated calls from rapid registry changes are safe.
                                let node = Arc::clone(&self.node);
                                let w = Arc::clone(&self.watches);
                                let community_owned = community.clone();
                                // Get our own channel record keys so the watch setup can
                                // skip our writable log (opening it read-only would downgrade
                                // the DHT handle and cause "value is not writable" on next send).
                                let local_keys = self.session.read().as_ref()
                                    .and_then(|s| s.communities.get(&community))
                                    .map(|m| m.channel_record_keys.clone())
                                    .unwrap_or_default();
                                tokio::spawn(async move {
                                    watches::setup_channel_watches(
                                        &node, &w,
                                        &community_owned, &reg_key,
                                        &local_keys,
                                    ).await;
                                });
                            }
                        }
                        crate::payload::dht_types::REGISTRY_MEK_VAULT => {
                            // Read the current max generation from the mek_cache
                            let generation = self.mek_cache.read()
                                .snapshot(&community)
                                .iter()
                                .map(|e| e.generation)
                                .max()
                                .unwrap_or(0);
                            debug!(community = %community, generation, "MEK vault changed");
                            self.process_event(SubscriptionEvent::Crypto(
                                events::CryptoEvent::MekRotated {
                                    community: community.clone(),
                                    channel: None, generation,
                                    rotator_pseudonym: None,
                                },
                            ));
                        }
                        crate::payload::dht_types::REGISTRY_MODERATION_QUEUE => {
                            debug!(community = %community, "moderation queue changed");
                        }
                        _ => {}
                    }
                }
            }
            watches::WatchKind::JoinInbox { community } => {
                debug!(community = %community, subkeys = ?changed_subkeys, "join inbox changed");
                // The daemon's inbox processor is triggered by this signal.
            }
            watches::WatchKind::ChannelLog { community, channel_id, member_pseudonym } => {
                // Skip own channel log changes — emit_local already handled it.
                // Without this, self-sent messages produce duplicate "(decrypting...)" placeholders.
                let is_own_log = self.session.read().as_ref()
                    .and_then(|s| s.communities.get(&community))
                    .is_some_and(|m| m.pseudonym_key == member_pseudonym);
                if is_own_log {
                    debug!(community = %community, "skipping own channel log watch notification");
                    return;
                }

                let count = self.state.write().unread.increment_channel(&community, &channel_id);

                // Emit immediate notification (body: None) for unread count
                self.process_event(SubscriptionEvent::ChannelMessage(
                    events::ChannelMessageEvent::New {
                        community: community.clone(),
                        channel: channel_id.clone(),
                        message_id: String::new(),
                        sender_pseudonym: member_pseudonym.clone(),
                        sequence: 0,
                        timestamp: rekindle_utils::timestamp_ms(),
                        body: None,
                        reply_to_sequence: None,
                        is_self: false,
                    },
                ));
                self.process_event(SubscriptionEvent::UnreadChanged {
                    context: events::UnreadContext::Channel {
                        community: community.clone(), channel: channel_id.clone(),
                    },
                    count,
                });

                // Spawn async DhtLog read + MEK decrypt to populate body.
                // record_key is the DhtLog spine key that triggered this watch.
                let log_key = record_key.to_string();
                let node = Arc::clone(&self.node);
                let event_tx = self.event_tx.clone();
                let mek_cache = Arc::clone(&self.mek_cache);
                let community_owned = community;
                let channel_owned = channel_id;
                let pseudonym_owned = member_pseudonym;
                tokio::spawn(async move {
                    let Ok(dht) = node.dht() else { return };
                    let log = match crate::broadcast::dht::channel_log::DhtLog::open_read(
                        dht.routing_context(), &log_key,
                    ).await {
                        Ok(l) => l,
                        Err(e) => {
                            tracing::debug!(error = %e, "channel body enrich: DhtLog open failed");
                            return;
                        }
                    };
                    let entries = match log.tail(1).await {
                        Ok(e) if !e.is_empty() => e,
                        _ => return,
                    };
                    let msg: crate::payload::dht_types::ChannelMessage =
                        match serde_json::from_slice(&entries[0]) {
                            Ok(m) => m,
                            Err(_) => return,
                        };

                    // MEK decrypt
                    let body = {
                        let cache = mek_cache.read();
                        cache.get_generation(&community_owned, &channel_owned, msg.mek_generation)
                            .and_then(|mek| mek.decrypt(&msg.ciphertext).ok())
                            .map(|plaintext| String::from_utf8_lossy(&plaintext).into_owned())
                    };

                    let message_id = msg.message_id
                        .unwrap_or_else(|| format!("seq:{}", msg.sequence));

                    if let Some(body_text) = body {
                        let _ = event_tx.send(SubscriptionEvent::ChannelMessage(
                            events::ChannelMessageEvent::New {
                                community: community_owned,
                                channel: channel_owned,
                                message_id,
                                sender_pseudonym: pseudonym_owned,
                                sequence: msg.sequence,
                                timestamp: msg.timestamp,
                                body: Some(body_text),
                                reply_to_sequence: msg.reply_to,
                                is_self: false,
                            },
                        ));
                    }
                });
            }
        }
    }

    /// Handle attachment state change.
    pub async fn on_attachment_change(&self, is_attached: bool, public_internet_ready: bool) {
        self.process_event(SubscriptionEvent::Network(
            events::NetworkEvent::AttachmentChanged { is_attached, public_internet_ready },
        ));

        // Re-establish ALL watches on re-attach — not just stale ones.
        // After a brief outage, no watches are past their renewal interval locally,
        // but the remote storage nodes may have expired them during the outage.
        if is_attached && public_internet_ready {
            let all_watches: Vec<(String, watches::WatchEntry)> = self.watches.read()
                .entries.iter()
                .map(|(k, e)| (k.clone(), e.clone()))
                .collect();
            info!(count = all_watches.len(), "network re-attached — re-establishing all watches");
            for (record_key, entry) in all_watches {
                if watches::renew_watch(&self.node, &record_key, &entry.subkeys).await {
                    if let Some(e) = self.watches.write().entries.get_mut(&record_key) {
                        e.established_at = std::time::Instant::now();
                    }
                }
            }
        }
    }

    /// Handle route deaths.
    pub fn on_route_change(&self, local_died: usize, remote_died: Vec<String>) {
        if local_died > 0 {
            self.process_event(SubscriptionEvent::Network(
                events::NetworkEvent::LocalRoutesDied { count: local_died },
            ));
        }
        if !remote_died.is_empty() {
            self.process_event(SubscriptionEvent::Network(
                events::NetworkEvent::RemoteRoutesDied { peer_keys: remote_died },
            ));
        }
    }

    /// Handle a watch death (count=0 or empty subkeys).
    pub async fn on_watch_died(&self, record_key: &str) {
        let entry = self.watches.read().get(record_key).cloned();
        if let Some(entry) = entry {
            info!(record_key, "watch died — re-establishing");
            if watches::renew_watch(&self.node, record_key, &entry.subkeys).await {
                if let Some(e) = self.watches.write().entries.get_mut(record_key) {
                    e.established_at = std::time::Instant::now();
                }
                self.process_event(SubscriptionEvent::Network(
                    events::NetworkEvent::WatchReestablished { record_key: record_key.into() },
                ));
            } else {
                warn!(record_key, "watch re-establishment failed");
                self.process_event(SubscriptionEvent::Network(
                    events::NetworkEvent::WatchFailed {
                        record_key: record_key.into(),
                        error: "re-establishment failed after watch death".into(),
                    },
                ));
            }
        } else {
            debug!(record_key, "watch died for unregistered record — ignoring");
        }
    }

    // ── State queries ──────────────────────────────────────────────────

    /// Read-only access to unread state.
    pub fn unread_channels(&self) -> HashMap<(String, String), u32> {
        self.state.read().unread.channels.clone()
    }

    pub fn unread_dms(&self) -> HashMap<String, u32> {
        self.state.read().unread.dms.clone()
    }

    pub fn unread_friend_requests(&self) -> u32 {
        self.state.read().unread.friend_requests
    }

    /// Mark a channel as read.
    pub fn mark_channel_read(&self, community: &str, channel: &str) {
        let prev = self.state.write().unread.mark_channel_read(community, channel);
        if prev > 0 {
            self.process_event(SubscriptionEvent::UnreadChanged {
                context: events::UnreadContext::Channel {
                    community: community.into(), channel: channel.into(),
                },
                count: 0,
            });
        }
    }

    /// Mark a DM conversation as read.
    pub fn mark_dm_read(&self, peer_key: &str) {
        let prev = self.state.write().unread.mark_dm_read(peer_key);
        if prev > 0 {
            self.process_event(SubscriptionEvent::UnreadChanged {
                context: events::UnreadContext::Dm { peer_key: peer_key.into() },
                count: 0,
            });
        }
    }

    /// Active typers in a channel.
    pub fn typing_in_channel(&self, community: &str, channel: &str) -> Vec<String> {
        self.state.write().typing.channel_typers(community, channel)
    }

    /// Whether a peer is typing in DM.
    pub fn typing_in_dm(&self, peer_key: &str) -> bool {
        self.state.read().typing.is_dm_typing(peer_key)
    }

    /// Community member presence.
    pub fn presence(&self, community: &str) -> Vec<(String, state::PresenceInfo)> {
        self.state.read().presence.community_members(community)
    }

    /// Friend presence.
    pub fn friend_presence(&self, peer_key: &str) -> Option<state::PresenceInfo> {
        self.state.read().presence.friend(peer_key).cloned()
    }

    /// Voice channel participants.
    pub fn voice_participants(&self, community: &str, channel: &str) -> Vec<state::VoiceParticipantInfo> {
        self.state.write().voice.participants(community, channel)
    }

    /// Total active watch count (for diagnostics).
    pub fn watch_count(&self) -> usize {
        self.watches.read().count()
    }

    /// Access the shared gossip meshes (for BroadcastManager).
    pub fn meshes(&self) -> &Arc<RwLock<HashMap<String, GossipMesh>>> {
        &self.meshes
    }

    /// Access the transport node Arc (for callers that need to set up watches
    /// without holding the SubscriptionManager lock across an await point).
    pub fn node(&self) -> &Arc<TransportNode> {
        &self.node
    }

    /// Access the watch registry Arc (for callers that need to set up watches
    /// without holding the SubscriptionManager lock across an await point).
    pub fn watches(&self) -> &Arc<RwLock<WatchRegistry>> {
        &self.watches
    }

    /// Access the dedup state (for diagnostics — dedup entry count and suppressed count).
    pub fn dedup(&self) -> &Arc<RwLock<dedup::EventDedup>> {
        &self.dedup
    }

    /// Emit a locally-originated event through the standard pipeline.
    ///
    /// Used by the daemon after a successful DHT write (channel send, DM send)
    /// so the sender's own UI sees the message via the same subscription path
    /// that peers use. The event goes through dedup + state_effects + broadcast,
    /// so it behaves identically to a remotely-received event.
    ///
    /// If the write failed, the caller must NOT call this — the user should see
    /// an error, not a phantom message that was never persisted.
    pub fn emit_local(&self, event: SubscriptionEvent) {
        self.process_event(event);
    }
}
