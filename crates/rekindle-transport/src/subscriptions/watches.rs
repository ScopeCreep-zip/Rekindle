//! DHT watch lifecycle — create, renew, route ValueChange events.
//!
//! The watch registry maps DHT record keys to their purpose so that
//! when `VeilidUpdate::ValueChange` arrives, we know whether the changed
//! record is a friend inbox, a community registry, a channel log, etc.
//!
//! Watch renewal runs on a timer — Veilid watches have finite lifetimes
//! (default ~10 minutes) and must be proactively renewed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use tracing::{debug, info, warn};

use crate::broadcast::node::TransportNode;
use crate::payload::dht_types;
use crate::session::{CommunityMembership, Session};

/// Default watch renewal interval (4 minutes).
/// Veilid's default watch expiry is ~10 minutes; renewing at 4 gives margin.
const WATCH_RENEWAL_INTERVAL: Duration = Duration::from_secs(240);

/// What kind of record a watch is tracking.
#[derive(Debug, Clone)]
pub enum WatchKind {
    /// Our friend inbox (DFLT(32), subkeys 0-31).
    FriendInbox,
    /// A peer's DM DhtLog spine.
    DmLog { peer_key: String },
    /// Community governance manifest (subkeys: metadata, channels, roles, bans, invites).
    GovernanceManifest { community: String },
    /// Community member registry (subkeys: member index, MEK vault, moderation queue).
    MemberRegistry { community: String },
    /// Community join inbox (operator only, DFLT(32), subkeys 0-31).
    JoinInbox { community: String },
    /// A member's per-channel DhtLog spine.
    ChannelLog {
        community: String,
        channel_id: String,
        member_pseudonym: String,
    },
}

/// A single active watch entry.
#[derive(Debug, Clone)]
pub struct WatchEntry {
    /// What this watch is for.
    pub kind: WatchKind,
    /// Subkeys being watched.
    pub subkeys: Vec<u32>,
    /// When the watch was last established or renewed.
    pub established_at: Instant,
    /// How often to renew (before Veilid expires it).
    pub renewal_interval: Duration,
}

impl WatchEntry {
    /// Whether this watch needs renewal.
    pub fn needs_renewal(&self) -> bool {
        self.established_at.elapsed() > self.renewal_interval
    }
}

/// Registry of all active DHT watches, keyed by record key string.
#[derive(Debug, Default)]
pub struct WatchRegistry {
    pub entries: HashMap<String, WatchEntry>,
}

impl WatchRegistry {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Register a watch. Overwrites any existing watch for this key.
    pub fn insert(&mut self, record_key: String, entry: WatchEntry) {
        self.entries.insert(record_key, entry);
    }

    /// Remove a watch by record key.
    pub fn remove(&mut self, record_key: &str) -> Option<WatchEntry> {
        self.entries.remove(record_key)
    }

    /// Look up a watch by record key (for ValueChange routing).
    pub fn get(&self, record_key: &str) -> Option<&WatchEntry> {
        self.entries.get(record_key)
    }

    /// Collect all entries that need renewal.
    pub fn needs_renewal(&self) -> Vec<(String, WatchEntry)> {
        self.entries.iter()
            .filter(|(_, e)| e.needs_renewal())
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect()
    }

    /// Remove all watches for a community.
    pub fn remove_community(&mut self, community: &str) {
        self.entries.retain(|_, e| {
            !matches!(&e.kind,
                WatchKind::GovernanceManifest { community: c }
                | WatchKind::MemberRegistry { community: c }
                | WatchKind::JoinInbox { community: c }
                | WatchKind::ChannelLog { community: c, .. }
                if c == community
            )
        });
    }

    /// Remove all watches for a DM peer.
    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.entries.retain(|_, e| {
            !matches!(&e.kind, WatchKind::DmLog { peer_key: pk } if pk == peer_key)
        });
    }

    /// Total number of active watches.
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

// ── Watch establishment ────────────────────────────────────────────────

/// Establish a DHT watch on a record and register it in the watch registry.
///
/// Opens the record readonly (if not already open), then calls
/// `watch_dht_values`. Returns `true` if the watch is active.
pub async fn establish_watch(
    node: &TransportNode,
    registry: &RwLock<WatchRegistry>,
    record_key: &str,
    subkeys: &[u32],
    kind: WatchKind,
) -> bool {
    // Ensure record is open (idempotent for already-open records)
    if let Err(e) = crate::broadcast::dht_writes::open_readonly(node, record_key).await {
        warn!(record_key, error = %e, "watch: cannot open record");
        return false;
    }

    let active = match crate::broadcast::dht_writes::watch(node, record_key, subkeys).await {
        Ok(active) => active,
        Err(e) => {
            warn!(record_key, error = %e, "watch: watch_dht_values failed");
            false
        }
    };

    if active {
        debug!(record_key, subkeys = ?subkeys, "watch established");
        registry.write().insert(record_key.to_string(), WatchEntry {
            kind,
            subkeys: subkeys.to_vec(),
            established_at: Instant::now(),
            renewal_interval: WATCH_RENEWAL_INTERVAL,
        });
    } else {
        warn!(record_key, "watch: Veilid declined the watch");
    }

    active
}

/// Renew a watch (re-call watch_dht_values with same parameters).
pub async fn renew_watch(
    node: &TransportNode,
    record_key: &str,
    subkeys: &[u32],
) -> bool {
    match crate::broadcast::dht_writes::watch(node, record_key, subkeys).await {
        Ok(active) => {
            if active {
                debug!(record_key, "watch renewed");
            } else {
                warn!(record_key, "watch renewal: Veilid declined");
            }
            active
        }
        Err(e) => {
            warn!(record_key, error = %e, "watch renewal failed");
            false
        }
    }
}

// ── Setup helpers ──────────────────────────────────────────────────────

/// Establish all identity-level watches (friend inbox).
pub async fn setup_identity_watches(
    node: &TransportNode,
    registry: &RwLock<WatchRegistry>,
    session: &Session,
) {
    // Watch friend inbox (all subkeys)
    if !session.identity.friend_inbox_key.is_empty() {
        let subkeys: Vec<u32> = (0..crate::payload::dht_types::FRIEND_INBOX_SUBKEY_COUNT).collect();
        establish_watch(
            node, registry,
            &session.identity.friend_inbox_key,
            &subkeys,
            WatchKind::FriendInbox,
        ).await;
    }

    // Watch each peer's DM logs (inbound for receiving, outbound for self-send confirmation)
    for (peer_key, peer_log) in &session.dm_peers {
        if !peer_log.inbound_log_key.is_empty() {
            establish_watch(
                node, registry,
                &peer_log.inbound_log_key,
                &[0], // spine subkey
                WatchKind::DmLog { peer_key: peer_key.clone() },
            ).await;
        }
    }
}

/// Establish all community-level watches.
pub async fn setup_community_watches(
    node: &TransportNode,
    registry: &RwLock<WatchRegistry>,
    membership: &CommunityMembership,
) {
    let community = &membership.governance_key;

    // Watch governance manifest (metadata, channels, roles, bans, invites)
    let gov_subkeys = vec![
        dht_types::MANIFEST_METADATA,
        dht_types::MANIFEST_CHANNELS,
        dht_types::MANIFEST_ROLES,
        dht_types::MANIFEST_BANS,
        dht_types::MANIFEST_INVITES,
    ];
    establish_watch(
        node, registry,
        &membership.governance_key,
        &gov_subkeys,
        WatchKind::GovernanceManifest { community: community.clone() },
    ).await;

    // Watch member registry (member index, MEK vault, moderation queue)
    let reg_subkeys = vec![
        dht_types::REGISTRY_MEMBER_INDEX,
        dht_types::REGISTRY_MEK_VAULT,
        dht_types::REGISTRY_MODERATION_QUEUE,
    ];
    establish_watch(
        node, registry,
        &membership.registry_key,
        &reg_subkeys,
        WatchKind::MemberRegistry { community: community.clone() },
    ).await;

    // Watch join inbox (operators only — they process incoming join requests)
    if membership.is_operator && !membership.join_inbox_key.is_empty() {
        let inbox_subkeys: Vec<u32> = (0..dht_types::JOIN_INBOX_SUBKEY_COUNT).collect();
        establish_watch(
            node, registry,
            &membership.join_inbox_key,
            &inbox_subkeys,
            WatchKind::JoinInbox { community: community.clone() },
        ).await;
    }

    info!(
        community = membership.community_name.as_str(),
        governance = %membership.governance_key,
        registry = %membership.registry_key,
        join_inbox = %membership.join_inbox_key,
        is_operator = membership.is_operator,
        "community watches established"
    );
}

/// Set up a DM peer watch.
pub async fn setup_dm_watch(
    node: &TransportNode,
    registry: &RwLock<WatchRegistry>,
    peer_key: &str,
    dm_log_key: &str,
) {
    establish_watch(
        node, registry,
        dm_log_key,
        &[0], // DhtLog spine subkey
        WatchKind::DmLog { peer_key: peer_key.to_string() },
    ).await;
}

/// Establish per-member channel log watches for all channels in a community.
///
/// Reads the registry member index to discover all members' DhtLog spine keys.
/// Each spine gets a watch on subkey 0 (the spine metadata subkey that changes
/// on every append). When a member sends a message, their spine subkey 0 changes,
/// the watch fires, and `on_value_change` emits a `ChannelMessage::New` event.
///
/// Called from `setup_community` (on join/resume) and from `on_value_change` when
/// `REGISTRY_MEMBER_INDEX` changes (new member registered their channel log).
///
/// Idempotent — skips records that already have a watch registered.
pub async fn setup_channel_watches<S: ::std::hash::BuildHasher>(
    node: &TransportNode,
    watch_registry: &RwLock<WatchRegistry>,
    community: &str,
    registry_key: &str,
    local_channel_record_keys: &std::collections::HashMap<String, String, S>,
) {
    // Read registry member index with force_refresh to see latest members.
    // Non-operator members' local cache may be stale — the operator may have
    // registered new members' channel logs that haven't propagated via poll.
    let members: Vec<crate::payload::dht_types::MemberSummary> =
        match crate::broadcast::dht_writes::get(
            node, registry_key,
            crate::payload::dht_types::REGISTRY_MEMBER_INDEX, true,
        ).await {
            Ok(Some(data)) => {
                match serde_json::from_slice(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(community, error = %e, bytes = data.len(),
                            "channel watches: registry member index parse failed");
                        return;
                    }
                }
            }
            Ok(None) => {
                debug!(community, "channel watches: registry member index empty");
                return;
            }
            Err(e) => {
                warn!(community, error = %e, "channel watches: registry read failed");
                return;
            }
        };

    let mut new_watches = 0u32;
    for member in &members {
        for (channel_id, log_key) in &member.channel_records {
            // Skip our OWN channel log — opening it read-only would downgrade
            // the writable DHT handle and cause "value is not writable" on next send.
            if local_channel_record_keys.values().any(|k| k == log_key) {
                continue;
            }
            // Skip if we already have a watch on this log key (idempotent)
            if watch_registry.read().get(log_key).is_some() {
                continue;
            }
            establish_watch(
                node, watch_registry, log_key,
                &[0], // DhtLog spine subkey — changes on every append
                WatchKind::ChannelLog {
                    community: community.to_string(),
                    channel_id: channel_id.clone(),
                    member_pseudonym: member.pseudonym_key.clone(),
                },
            ).await;
            new_watches += 1;
        }
    }

    if new_watches > 0 {
        info!(
            community, members = members.len(), new_watches,
            "channel log watches established"
        );
    }
}

// ── Renewal loop ───────────────────────────────────────────────────────

/// Background task that renews watches before they expire.
///
/// Runs every 60 seconds. For each watch past its renewal interval,
/// re-calls `watch_dht_values` and updates the `established_at` timestamp.
pub async fn run_renewal_loop(
    node: Arc<TransportNode>,
    registry: Arc<RwLock<WatchRegistry>>,
    event_tx: tokio::sync::broadcast::Sender<super::events::SubscriptionEvent>,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
) {
    use super::events::{SubscriptionEvent, NetworkEvent};

    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.tick().await; // skip immediate first tick

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let stale = registry.read().needs_renewal();
                for (record_key, entry) in stale {
                    if renew_watch(&node, &record_key, &entry.subkeys).await {
                        if let Some(e) = registry.write().entries.get_mut(&record_key) {
                            e.established_at = Instant::now();
                        }
                        let _ = event_tx.send(SubscriptionEvent::Network(
                            NetworkEvent::WatchRenewed { record_key: record_key.clone() },
                        ));
                    } else {
                        let _ = event_tx.send(SubscriptionEvent::Network(
                            NetworkEvent::WatchFailed {
                                record_key: record_key.clone(),
                                error: "renewal declined by Veilid".into(),
                            },
                        ));
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("watch renewal loop shutting down");
                break;
            }
        }
    }
}
