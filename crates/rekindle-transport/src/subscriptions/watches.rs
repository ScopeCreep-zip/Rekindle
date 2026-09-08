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
    /// Community join inbox (operator only, DFLT(32), subkeys 0-31).
    JoinInbox { community: String },
    /// A channel's SMPL segment record.
    ///
    /// PATH 3 of the three-path model. One record per
    /// `(channel, segment)` with a subkey per member, so the changed
    /// subkey names the author's slot rather than identifying the
    /// record's single owner — which is why this no longer carries a
    /// `member_pseudonym` the way the per-member `DhtLog` version did.
    ChannelRecord {
        community: String,
        channel_id: String,
        segment_index: u32,
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
        Self {
            entries: HashMap::new(),
        }
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
        self.entries
            .iter()
            .filter(|(_, e)| e.needs_renewal())
            .map(|(k, e)| (k.clone(), e.clone()))
            .collect()
    }

    /// Remove all watches for a community.
    pub fn remove_community(&mut self, community: &str) {
        self.entries.retain(|_, e| {
            !matches!(&e.kind,
                WatchKind::GovernanceManifest { community: c }
                | WatchKind::JoinInbox { community: c }
                | WatchKind::ChannelRecord { community: c, .. }
                if c == community
            )
        });
    }

    /// Remove all watches for a DM peer.
    pub fn remove_dm_peer(&mut self, peer_key: &str) {
        self.entries
            .retain(|_, e| !matches!(&e.kind, WatchKind::DmLog { peer_key: pk } if pk == peer_key));
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
    establish_watch_as(node, registry, record_key, subkeys, kind, None).await
}

/// Establish a watch, optionally as a **schema member**.
///
/// Veilid reserves watch slots in two tiers (see `watch_dht_values` in
/// veilid-core): `public_watch_limit` (32) for anonymous readers,
/// first-come-first-served, and `member_watch_limit` (8) reserved for
/// watchers whose signer is a member of the record's SMPL schema. Its
/// own docs note that "members can be specified via the SMPL schema and
/// do not need to allocate writable subkeys in order to offer a member
/// watch capability" — so opening with our slot keypair moves us out of
/// the contended public pool at no cost.
///
/// `writer` is that keypair, in string form. `None` keeps the old
/// read-only open for records we hold no slot in.
pub async fn establish_watch_as(
    node: &TransportNode,
    registry: &RwLock<WatchRegistry>,
    record_key: &str,
    subkeys: &[u32],
    kind: WatchKind,
    writer: Option<&str>,
) -> bool {
    let opened = match writer {
        Some(writer) => {
            crate::broadcast::dht_writes::open_str(node, record_key, Some(writer)).await
        }
        None => crate::broadcast::dht_writes::open_readonly(node, record_key).await,
    };
    if let Err(e) = opened {
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

    // Registered whether or not Veilid granted the watch.
    //
    // The registry is also the 60-second inspect poll's work list
    // (`poll::run_poll_loop` iterates it), and a declined watch is the
    // case that needs the fallback *most*: with 8 member and 32 public
    // slots per record, a community larger than 40 members leaves the
    // rest with no watch at all. Registering only on success gave those
    // members neither push nor poll — the architecture's PATH 3 has two
    // halves and they were getting zero.
    //
    // The renewal loop retries the watch on its own schedule, so a slot
    // freed later is picked up without re-registering.
    if active {
        debug!(record_key, subkeys = ?subkeys, "watch established");
    } else {
        debug!(
            record_key,
            "watch: Veilid declined (slots full) — record still enrolled in the inspect poll"
        );
    }
    registry.write().insert(
        record_key.to_string(),
        WatchEntry {
            kind,
            subkeys: subkeys.to_vec(),
            established_at: Instant::now(),
            renewal_interval: WATCH_RENEWAL_INTERVAL,
        },
    );

    active
}

/// Renew a watch (re-call watch_dht_values with same parameters).
pub async fn renew_watch(node: &TransportNode, record_key: &str, subkeys: &[u32]) -> bool {
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
            node,
            registry,
            &session.identity.friend_inbox_key,
            &subkeys,
            WatchKind::FriendInbox,
        )
        .await;
    }

    // Watch each peer's DM log spine
    for (peer_key, dm_log_key) in &session.dm_log_keys {
        establish_watch(
            node,
            registry,
            dm_log_key,
            &[0], // spine subkey
            WatchKind::DmLog {
                peer_key: peer_key.clone(),
            },
        )
        .await;
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
        node,
        registry,
        &membership.governance_key,
        &gov_subkeys,
        WatchKind::GovernanceManifest {
            community: community.clone(),
        },
    )
    .await;

    // No member-registry watch. Its three subkeys were community-wide
    // v1.0 entries that no longer exist; under `o_cnt: 0` every subkey
    // is a member's presence row, and those are read by the presence
    // poll rather than watched. Watching them here reported one
    // arbitrary member's heartbeat as a governance signal.

    // Watch each channel's SMPL segment record — PATH 3.
    //
    // Opened **writable with our slot keypair**, which is what moves the
    // watch out of Veilid's contended `public_watch_limit` (32,
    // first-come-first-served) and into the `member_watch_limit` (8)
    // reserved for schema members. veilid-core's own docs are explicit
    // that members "do not need to allocate writable subkeys in order to
    // offer a member watch capability", so this costs nothing we were
    // not already entitled to.
    //
    // Eight slots does not cover a large community, and that is expected
    // rather than a failure: `establish_watch_as` enrols the record in
    // the 60-second inspect poll whether or not the watch is granted,
    // and peers that did get a slot relay what they see over gossip
    // (`CommunityEnvelope::WatchRelay`, architecture §14.3).
    let slot_writer = membership.slot_seed.and_then(|seed| {
        crate::broadcast::dht_writes::derive_slot_keypair_str(&seed, membership.slot_index).ok()
    });
    let segment_index = membership.segment_index.unwrap_or(0);
    let channel_subkeys: Vec<u32> = (0..dht_types::SLOTS_PER_SEGMENT).collect();
    for (channel_id, record_key) in &membership.channel_record_keys {
        establish_watch_as(
            node,
            registry,
            record_key,
            &channel_subkeys,
            WatchKind::ChannelRecord {
                community: community.clone(),
                channel_id: channel_id.clone(),
                segment_index,
            },
            slot_writer.as_deref(),
        )
        .await;
    }

    // Watch join inbox (operators only — they process incoming join requests)
    if membership.is_operator && !membership.join_inbox_key.is_empty() {
        let inbox_subkeys: Vec<u32> = (0..dht_types::JOIN_INBOX_SUBKEY_COUNT).collect();
        establish_watch(
            node,
            registry,
            &membership.join_inbox_key,
            &inbox_subkeys,
            WatchKind::JoinInbox {
                community: community.clone(),
            },
        )
        .await;
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
        node,
        registry,
        dm_log_key,
        &[0], // DhtLog spine subkey
        WatchKind::DmLog {
            peer_key: peer_key.to_string(),
        },
    )
    .await;
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
    use super::events::{NetworkEvent, SubscriptionEvent};

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
