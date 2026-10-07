//! DHT watch lifecycle — create, renew, route ValueChange events.
//!
//! The watch registry maps DHT record keys to their purpose so that
//! when `VeilidUpdate::ValueChange` arrives, we know whether the changed
//! record is a friend inbox, a community registry, a channel log, etc.
//!
//! Every watch rides a lease on the session's record pool (plan C7.7): the
//! pool watches the union of its borrowers' subkeys, Veilid renews a watch
//! itself, and the pool re-arms a dead one while it is held (plan C7.8), so
//! nothing here renews.

use std::collections::HashMap;

use parking_lot::RwLock;
use tracing::{info, warn};

use crate::broadcast::dht_writes::LeaseId;
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types;
use crate::session::{CommunityMembership, Session};

/// What kind of record a watch is tracking.
#[derive(Debug, Clone)]
pub enum WatchKind {
    /// Our friend inbox (DFLT(32), subkeys 0-31).
    FriendInbox,
    /// A peer's DM DhtLog spine.
    DmLog { peer_key: String },
    /// A community's SMPL governance record.
    ///
    /// One member's signed entry history per subkey, so a change means
    /// "that member wrote governance" — not "this section changed". The
    /// reader re-merges; it cannot tell what changed from the subkey
    /// index alone.
    GovernanceRecord { community: String },
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
    /// The pool lease the watch rides; released when the watch is removed.
    pub lease: LeaseId,
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

    /// Register a watch, returning the one it replaced (whose lease the
    /// caller releases).
    pub fn insert(&mut self, record_key: String, entry: WatchEntry) -> Option<WatchEntry> {
        self.entries.insert(record_key, entry)
    }

    /// Remove a watch by record key.
    pub fn remove(&mut self, record_key: &str) -> Option<WatchEntry> {
        self.entries.remove(record_key)
    }

    /// Look up a watch by record key (for ValueChange routing).
    pub fn get(&self, record_key: &str) -> Option<&WatchEntry> {
        self.entries.get(record_key)
    }

    /// Remove every watch matching `gone`; their leases, to release.
    fn remove_where(&mut self, gone: impl Fn(&WatchKind) -> bool) -> Vec<LeaseId> {
        let mut leases = Vec::new();
        self.entries.retain(|_, e| {
            let keep = !gone(&e.kind);
            if !keep {
                leases.push(e.lease);
            }
            keep
        });
        leases
    }

    /// Remove all watches for a community; their leases, to release.
    pub fn remove_community(&mut self, community: &str) -> Vec<LeaseId> {
        self.remove_where(|kind| {
            matches!(kind,
                WatchKind::GovernanceRecord { community: c }
                | WatchKind::ChannelRecord { community: c, .. }
                if c == community
            )
        })
    }

    /// Remove all watches for a DM peer; their leases, to release.
    pub fn remove_dm_peer(&mut self, peer_key: &str) -> Vec<LeaseId> {
        self.remove_where(
            |kind| matches!(kind, WatchKind::DmLog { peer_key: pk } if pk == peer_key),
        )
    }

    /// Total number of active watches.
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

// ── Watch establishment ────────────────────────────────────────────────

/// Establish a DHT watch on a record and register it in the watch registry.
///
/// Borrows the record read-only from the session's pool and watches on
/// that lease. Returns `true` if the watch was registered.
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
/// `writer` is that keypair, in string form; it becomes the lease's sticky
/// writer. `None` borrows read-only, for records we hold no slot in.
pub async fn establish_watch_as(
    node: &TransportNode,
    registry: &RwLock<WatchRegistry>,
    record_key: &str,
    subkeys: &[u32],
    kind: WatchKind,
    writer: Option<&str>,
) -> bool {
    let lease = match crate::broadcast::dht_writes::acquire_str(node, record_key, writer).await {
        Ok(lease) => lease,
        Err(e) => {
            warn!(record_key, error = %e, "watch: cannot open record");
            return false;
        }
    };

    let active = match crate::broadcast::dht_writes::watch_leased(node, lease, subkeys).await {
        Ok(()) => true,
        Err(e) => {
            warn!(record_key, error = %e, "watch: watch_dht_values failed");
            false
        }
    };

    // Registered unconditionally (with its lease, which keeps the record
    // open), because a watch call cannot tell us whether a watch exists.
    //
    // `watch_dht_values` "records the desired watch state and returns
    // without a network round-trip; a background task reconciles it
    // with a remote node", and "no network errors surface here". So
    // Success means the desired state was accepted locally, not that a
    // remote node agreed to watch — a record refused for want of a slot is
    // indistinguishable here from one that succeeded.
    //
    // Failure surfaces later, as a `ValueChange` with `count == 0` or an
    // empty subkey range, which the record pool re-arms while the lease
    // is held.
    //
    // Registering regardless is therefore the only correct behaviour,
    // and it is also what enrols the record in the 60-second inspect
    // poll (`poll::run_poll_loop` iterates this registry). That matters
    // most for the members who did not get a slot: with 8 member and 32
    // public slots per record, a community past ~40 members leaves most
    // of them watchless, and PATH 3 has two halves.
    let replaced = registry.write().insert(
        record_key.to_string(),
        WatchEntry {
            kind,
            subkeys: subkeys.to_vec(),
            lease,
        },
    );
    if let Some(old) = replaced {
        crate::broadcast::dht_writes::release(node, old.lease).await;
    }

    active
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

    // Watch the whole governance record.
    //
    // This used to watch subkeys 0, 1, 3, 4 and 7 — the v1.0 manifest's
    // metadata / channels / roles / bans / invites sections. Under
    // `o_cnt: 0` those are not sections, they are **member slots**: the
    // governance record holds one member's signed `GovernanceEntry`
    // history per subkey. So it watched five arbitrary members and was
    // blind to everyone else's bans, role changes and channel creation.
    //
    // The whole range, because any member may write governance and the
    // CRDT is only correct if we see all of it. Same slot addressing as
    // the registry and channel records.
    let gov_subkeys: Vec<u32> = (0..dht_types::SLOTS_PER_SEGMENT).collect();
    let slot_writer = membership.slot_seed.and_then(|seed| {
        crate::broadcast::dht_writes::derive_slot_keypair_str(&seed, membership.slot_index).ok()
    });
    establish_watch_as(
        node,
        registry,
        &membership.governance_key,
        &gov_subkeys,
        WatchKind::GovernanceRecord {
            community: community.clone(),
        },
        slot_writer.as_deref(),
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

    info!(
        community = membership.community_name.as_str(),
        governance = %membership.governance_key,
        registry = %membership.registry_key,
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
