//! High-level query operations for CLI and TUI consumption.
//!
//! [`QueryEngine`] composes low-level DHT reads, MEK decryption, and
//! profile resolution into display-ready types. Every returned type
//! implements `Serialize + Clone + Debug` and contains no Veilid-internal
//! types — only strings, numbers, and bools.
//!
//! The CLI calls these methods directly for one-shot commands.
//! The TUI calls them from `tokio::spawn` tasks, routing results back
//! through the action channel as `CommandResult` variants.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::broadcast::dht::DhtStore;
use crate::broadcast::peer_registry::PeerRegistry;
use crate::crypto::mek::{MekCache, MekCacheEntrySnapshot};
use crate::error::{Result, TransportError};
use crate::payload::dht_types::{ChannelEntry, ChannelKind, ChannelMessage, RoleEntry};
use crate::session::CommunityMembership;
use crate::shared::SharedState;

// ── Display types (re-exported from rekindle-types) ─────────────────────
//
// These are the SSOT definitions in `rekindle_types::display`. The transport
// crate re-exports them so existing code that imports from here keeps working.
// New code should import from `rekindle_types::display` directly.

pub use rekindle_types::display::{
    ChannelOverviewDisplay, CommunityDetail, CommunityOverview, DecryptedMessageDisplay,
    DmMessageDisplay, DmThreadDisplay, FriendDisplay, RoleDisplay, TransportSnapshot,
};

// ── QueryEngine ─────────────────────────────────────────────────────────

/// High-level query interface for CLI and TUI.
///
/// Obtained via [`TransportNode::query()`](crate::broadcast::node::TransportNode).
/// Composes low-level DHT reads + MEK decryption + profile resolution
/// into display-ready types.
pub struct QueryEngine {
    dht: DhtStore,
    mek_cache: Arc<RwLock<MekCache>>,
    peer_registry: Arc<RwLock<PeerRegistry>>,
}

impl QueryEngine {
    /// Create a new query engine.
    pub fn new(
        dht: DhtStore,
        mek_cache: Arc<RwLock<MekCache>>,
        peer_registry: Arc<RwLock<PeerRegistry>>,
    ) -> Self {
        Self {
            dht,
            mek_cache,
            peer_registry,
        }
    }

    // ── Community queries ────────────────────────────────────────────

    /// List joined communities with overview metadata.
    ///
    /// Reads each community's governance metadata subkey for name/description
    /// and the member registry index for member count.
    pub async fn list_communities(
        &self,
        memberships: &[CommunityMembership],
    ) -> Result<Vec<CommunityOverview>> {
        let mut result = Vec::with_capacity(memberships.len());

        for m in memberships {
            let metadata = self
                .dht
                .governance()
                .read_metadata(&m.governance_key)
                .await?;

            let channels = self
                .dht
                .governance()
                .read_channels(&m.governance_key)
                .await
                .unwrap_or_default();

            let members = self
                .dht
                .registry()
                .read_member_index(&m.registry_key)
                .await
                .unwrap_or_default();

            let (name, description) = match metadata {
                Some(meta) => (meta.name, meta.description.unwrap_or_default()),
                None => (m.community_name.clone(), String::new()),
            };

            result.push(CommunityOverview {
                governance_key: m.governance_key.clone(),
                name,
                description,
                #[allow(clippy::cast_possible_truncation)]
                member_count: members.len() as u32,
                #[allow(clippy::cast_possible_truncation)]
                channel_count: channels.len() as u32,
                our_pseudonym: m.pseudonym_key.clone(),
            });
        }

        Ok(result)
    }

    /// Detailed info about a single community.
    pub async fn community_detail(
        &self,
        membership: &CommunityMembership,
    ) -> Result<CommunityDetail> {
        // Ensure governance and registry records are open for reading.
        // Records may have been closed since community creation/join.
        let _ = crate::broadcast::dht::record::open_readonly(
            self.dht.routing_context(),
            &membership.governance_key,
        )
        .await;
        let _ = crate::broadcast::dht::record::open_readonly(
            self.dht.routing_context(),
            &membership.registry_key,
        )
        .await;

        let metadata = self
            .dht
            .governance()
            .read_metadata(&membership.governance_key)
            .await?
            .ok_or_else(|| TransportError::DhtError {
                reason: format!(
                    "governance metadata not found for {}",
                    membership.governance_key
                ),
            })?;

        let channels = self
            .dht
            .governance()
            .read_channels(&membership.governance_key)
            .await?;

        let roles = self
            .dht
            .governance()
            .read_roles(&membership.governance_key)
            .await?;

        let members = self
            .dht
            .registry()
            .read_member_index(&membership.registry_key)
            .await?;

        Ok(CommunityDetail {
            governance_key: membership.governance_key.clone(),
            name: metadata.name,
            description: metadata.description.unwrap_or_default(),
            owner_pseudonym: metadata.owner_pseudonym,
            created_at: metadata.created_at,
            #[allow(clippy::cast_possible_truncation)]
            member_count: members.len() as u32,
            channels: channels.iter().map(channel_to_display).collect(),
            roles: roles.iter().map(role_to_display).collect(),
            our_pseudonym: membership.pseudonym_key.clone(),
            our_roles: membership.role_ids.clone(),
        })
    }

    // ── Channel queries ─────────────────────────────────────────────

    /// List channels in a community.
    pub async fn list_channels(&self, governance_key: &str) -> Result<Vec<ChannelOverviewDisplay>> {
        let channels = self.dht.governance().read_channels(governance_key).await?;
        Ok(channels.iter().map(channel_to_display).collect())
    }

    /// Read channel message history with MEK decryption.
    ///
    /// Per-member DhtLog architecture: each member owns their own
    /// append-only DhtLog per channel. This method scans the member
    /// registry for channel_records entries, opens each member's DhtLog,
    /// reads the last N messages from each, decrypts with the MEK, and
    /// merges all messages by (lamport_ts, sender_pseudonym) for
    /// deterministic total ordering across high-latency links.
    pub async fn channel_history(
        &self,
        community_id: &str,
        channel_id: &str,
        _channel_log_key: &str,
        registry_key: &str,
        limit: usize,
        local_channel_record_keys: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DecryptedMessageDisplay>> {
        // Read member index with force_refresh=true to get the latest
        // channel_records entries (RegisterChannelRecord may have just completed).
        let members: Vec<crate::payload::dht_types::MemberSummary> =
            match crate::broadcast::dht::record::get(
                self.dht.routing_context(),
                registry_key,
                crate::payload::dht_types::REGISTRY_MEMBER_INDEX,
                true,
            )
            .await
            {
                Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
                _ => Vec::new(),
            };

        // Collect all known DhtLog keys: from registry + from local session.
        // Local session has our own channel_record_keys that may not have
        // propagated to the registry yet (RegisterChannelRecord takes time).
        let mut log_keys_to_scan: Vec<(String, String)> = Vec::new(); // (display_name, log_key)

        for member in &members {
            if let Some(log_key) = member.channel_records.get(channel_id) {
                log_keys_to_scan.push((member.display_name.clone(), log_key.clone()));
            }
        }

        // Add our own local record key if not already in the registry list
        if let Some(local_key) = local_channel_record_keys.get(channel_id) {
            if !log_keys_to_scan.iter().any(|(_, k)| k == local_key) {
                log_keys_to_scan.push(("me".to_string(), local_key.clone()));
            }
        }

        tracing::info!(
            channel_id,
            registry_members = members.len(),
            log_keys_count = log_keys_to_scan.len(),
            local_keys = local_channel_record_keys.len(),
            "channel_history: scanning DhtLogs"
        );

        // Collect raw messages from each member's DhtLog.
        let mut raw_messages: Vec<(String, ChannelMessage)> = Vec::new();

        for (display_name, log_key) in &log_keys_to_scan {
            let log = match crate::broadcast::dht::channel_log::DhtLog::open_read(
                self.dht.routing_context(),
                log_key,
            )
            .await
            {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(
                        member = %display_name, log = %log_key,
                        error = %e, "channel_history: cannot open DhtLog"
                    );
                    continue;
                }
            };

            // Read the last `limit` entries from this member's log
            #[allow(clippy::cast_possible_truncation)]
            let entries = match log.tail(limit as u32).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!(
                        member = %display_name, error = %e,
                        "DhtLog tail read failed"
                    );
                    continue;
                }
            };

            for raw in &entries {
                let msg: ChannelMessage = match serde_json::from_slice(raw) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(error = %e, "skipping malformed DhtLog entry");
                        continue;
                    }
                };
                raw_messages.push((display_name.clone(), msg));
            }
        }

        // Decrypt — lock scoped to this block, no awaits
        let mut messages = Vec::with_capacity(raw_messages.len());
        {
            let mek_cache = self.mek_cache.read();
            for (author_name, channel_msg) in &raw_messages {
                let message_id = channel_msg
                    .message_id
                    .clone()
                    .unwrap_or_else(|| format!("seq:{}", channel_msg.sequence));

                let (body, is_encrypted, needs_mek) =
                    decrypt_channel_body(&mek_cache, community_id, channel_id, channel_msg);

                messages.push(DecryptedMessageDisplay {
                    message_id,
                    sequence: channel_msg.sequence,
                    author_pseudonym: channel_msg.sender_pseudonym.clone(),
                    author_display_name: author_name.clone(),
                    body,
                    timestamp: channel_msg.timestamp,
                    reply_to_sequence: channel_msg.reply_to,
                    mek_generation: channel_msg.mek_generation,
                    is_encrypted,
                    needs_mek,
                });
            }
        }

        // Deterministic total ordering: Lamport timestamp, then sender pseudonym
        messages.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then_with(|| a.author_pseudonym.cmp(&b.author_pseudonym))
        });

        // Return last N messages
        if messages.len() > limit {
            messages = messages.split_off(messages.len() - limit);
        }

        Ok(messages)
    }

    // ── Friend queries ──────────────────────────────────────────────

    /// Read friend list with resolved display names.
    ///
    /// For each friend, reads their profile display name and status
    /// subkeys. Profile reads that fail (peer offline, record unavailable)
    /// fall back to the stored nickname or public key abbreviation.
    pub async fn resolved_friends(&self, friend_list_key: &str) -> Result<Vec<FriendDisplay>> {
        let list = self.dht.friend_list().read(friend_list_key).await?;

        // Snapshot peer route state before the async loop to avoid holding
        // the RwLock across await points (clippy::await_holding_lock).
        let route_snapshot: Vec<(String, bool)> = {
            let peer_reg = self.peer_registry.read();
            list.friends
                .iter()
                .map(|f| {
                    let has = peer_reg.get_route(&f.public_key).is_some();
                    (f.public_key.clone(), has)
                })
                .collect()
        };

        let mut result = Vec::with_capacity(list.friends.len());

        for (i, friend) in list.friends.iter().enumerate() {
            let has_route = route_snapshot.get(i).is_some_and(|(_, has)| *has);

            let (display_name, status, status_message, last_seen) =
                if let Some(ref profile_key) = friend.profile_dht_key {
                    self.read_profile_summary(profile_key)
                        .await
                        .unwrap_or_else(|_| {
                            (
                                friend
                                    .nickname
                                    .clone()
                                    .unwrap_or_else(|| abbreviate_key(&friend.public_key)),
                                "unknown".to_string(),
                                String::new(),
                                None,
                            )
                        })
                } else {
                    (
                        friend
                            .nickname
                            .clone()
                            .unwrap_or_else(|| abbreviate_key(&friend.public_key)),
                        "unknown".to_string(),
                        String::new(),
                        None,
                    )
                };

            result.push(FriendDisplay {
                public_key: friend.public_key.clone(),
                display_name,
                nickname: friend.nickname.clone(),
                status,
                status_message,
                last_seen_ms: last_seen,
                profile_dht_key: friend.profile_dht_key.clone(),
                has_route,
            });
        }

        Ok(result)
    }

    // ── DM queries ───────────────────────────────────────────────────

    /// Read DM inbox grouped by conversation thread.
    ///
    /// Reads the DM conversation log, groups messages by sender peer key,
    /// resolves display names from the friend list, and returns threads
    /// sorted by most recent message first.
    ///
    /// The `dm_log_key` is the DHT key for the DM conversation log.
    /// The `friend_list_key` is used to resolve peer keys to display names.
    /// `limit_per_thread` caps how many messages to include per thread.
    pub async fn dm_inbox(
        &self,
        dm_log_key: &str,
        friend_list_key: &str,
        limit_per_thread: usize,
        our_public_key: &str,
    ) -> Result<Vec<DmThreadDisplay>> {
        // Read the DM log
        let dht_log = crate::broadcast::dht::channel_log::DhtLog::open_read(
            self.dht.routing_context(),
            dm_log_key,
        )
        .await?;

        // Read recent entries — cap at a reasonable total
        let total_limit = limit_per_thread.saturating_mul(50).min(500);
        #[allow(clippy::cast_possible_truncation)]
        let raw_entries = dht_log.tail(total_limit as u32).await?;

        // Read friend list for name resolution
        let friends = self.dht.friend_list().read(friend_list_key).await?;
        // Build name lookup: public_key → display name (nickname if set, else abbreviated key)
        let friend_display_names: std::collections::HashMap<String, String> = friends
            .friends
            .iter()
            .map(|f| {
                let name = f
                    .nickname
                    .clone()
                    .unwrap_or_else(|| abbreviate_key(&f.public_key));
                (f.public_key.clone(), name)
            })
            .collect();

        // Parse and group by peer
        let mut threads: std::collections::HashMap<String, Vec<DmMessageDisplay>> =
            std::collections::HashMap::new();

        for raw in &raw_entries {
            // DM log entries are stored as JSON with sender_key + body + timestamp
            let entry: serde_json::Value = match serde_json::from_slice(raw) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let sender_key = entry
                .get("sender_key")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            let body_raw = entry
                .get("body")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("[unreadable]");
            // DM bodies are stored as hex-encoded bytes in the DhtLog.
            // Decode hex → bytes → UTF-8. Fall back to raw string if decode fails.
            let body = hex::decode(body_raw)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| body_raw.to_string());
            let timestamp = entry
                .get("timestamp")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);

            let is_self = sender_key == our_public_key;
            let peer_key = if is_self {
                // For outgoing messages, the thread key is the recipient
                entry
                    .get("recipient_key")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&sender_key)
                    .to_string()
            } else {
                sender_key.clone()
            };

            let sender_name = friend_display_names
                .get(&sender_key)
                .cloned()
                .unwrap_or_else(|| abbreviate_key(&sender_key));

            threads.entry(peer_key).or_default().push(DmMessageDisplay {
                sender_key,
                sender_name,
                body,
                timestamp,
                is_self,
            });
        }

        // Build thread displays, sorted by most recent message
        let mut result: Vec<DmThreadDisplay> = threads
            .into_iter()
            .map(|(peer_key, mut messages)| {
                messages.sort_by_key(|m| m.timestamp);
                // Keep only the last N per thread
                if messages.len() > limit_per_thread {
                    let start = messages.len() - limit_per_thread;
                    messages = messages[start..].to_vec();
                }
                let last_at = messages.last().map_or(0, |m| m.timestamp);
                let peer_name = friend_display_names
                    .get(&peer_key)
                    .cloned()
                    .unwrap_or_else(|| abbreviate_key(&peer_key));

                DmThreadDisplay {
                    peer_key,
                    peer_name,
                    last_message_at: last_at,
                    unread_count: 0, // Unread tracking is a session-layer concern
                    messages,
                }
            })
            .collect();

        // Most recent thread first
        result.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
        Ok(result)
    }

    // ── Peer queries ────────────────────────────────────────────────

    /// Snapshot of all known peers for display.
    pub fn peer_snapshot(&self) -> Vec<crate::broadcast::peer_registry::PeerSnapshot> {
        self.peer_registry.read().snapshot()
    }

    // ── MEK queries ─────────────────────────────────────────────────

    /// MEK cache snapshot for a community.
    pub fn mek_cache_snapshot(&self, community_id: &str) -> Vec<MekCacheEntrySnapshot> {
        self.mek_cache.read().snapshot(community_id)
    }

    // ── Doctor queries ──────────────────────────────────────────────

    /// Node health summary for the doctor diagnostic view.
    ///
    /// `route_allocated` requires the route manager which `QueryEngine` doesn't
    /// own. The caller passes it explicitly from `TransportNode::status_snapshot()`.
    pub fn node_health(&self, shared: &SharedState, route_allocated: bool) -> TransportSnapshot {
        TransportSnapshot {
            attachment: shared.attachment_state().to_string(),
            is_attached: shared.is_attached(),
            public_internet_ready: shared.public_internet_ready(),
            uptime_secs: shared.uptime().as_secs(),
            peer_count: self.peer_registry.read().route_count(),
            route_allocated,
            route_age_secs: None,
        }
    }

    // ── Internal helpers ────────────────────────────────────────────

    /// Read profile display name, status, and last-seen from DHT.
    async fn read_profile_summary(
        &self,
        profile_key: &str,
    ) -> Result<(String, String, String, Option<u64>)> {
        use crate::payload::dht_types::{
            PROFILE_SUBKEY_DISPLAY_NAME, PROFILE_SUBKEY_STATUS, PROFILE_SUBKEY_STATUS_MESSAGE,
            STATUS_AWAY, STATUS_BUSY, STATUS_INVISIBLE, STATUS_OFFLINE, STATUS_ONLINE,
        };

        let profile = self.dht.profile();

        let display_name = match profile
            .get_subkey(profile_key, PROFILE_SUBKEY_DISPLAY_NAME)
            .await?
        {
            Some(data) if !data.is_empty() => String::from_utf8_lossy(&data).to_string(),
            _ => abbreviate_key(profile_key),
        };

        let (status, last_seen) = match profile
            .get_subkey(profile_key, PROFILE_SUBKEY_STATUS)
            .await?
        {
            Some(data) if !data.is_empty() => {
                let status_byte = data[0];
                let status_str = match status_byte {
                    STATUS_ONLINE => "online",
                    STATUS_AWAY => "away",
                    STATUS_BUSY => "busy",
                    STATUS_OFFLINE => "offline",
                    STATUS_INVISIBLE => "invisible",
                    _ => "unknown",
                };
                let last_seen_ms = if data.len() >= 9 {
                    let raw = i64::from_be_bytes(data[1..9].try_into().unwrap_or([0; 8]));
                    // Timestamps are always positive; clamp negative to 0
                    Some(u64::try_from(raw).unwrap_or(0))
                } else {
                    None
                };
                (status_str.to_string(), last_seen_ms)
            }
            _ => ("unknown".to_string(), None),
        };

        let status_message = match profile
            .get_subkey(profile_key, PROFILE_SUBKEY_STATUS_MESSAGE)
            .await?
        {
            Some(data) if !data.is_empty() => String::from_utf8_lossy(&data).to_string(),
            _ => String::new(),
        };

        Ok((display_name, status, status_message, last_seen))
    }
}

// ── Free functions ──────────────────────────────────────────────────────

fn channel_to_display(entry: &ChannelEntry) -> ChannelOverviewDisplay {
    ChannelOverviewDisplay {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: channel_kind_str(entry.kind),
        category_id: entry.category_id.clone(),
        topic: entry.topic.clone(),
        mek_generation: entry.mek_generation,
        log_key: entry.log_key.clone(),
        sort_order: entry.sort_order,
    }
}

fn channel_kind_str(kind: ChannelKind) -> String {
    match kind {
        ChannelKind::Text => "text",
        ChannelKind::Voice => "voice",
        ChannelKind::Announcement => "announcement",
        ChannelKind::Forum => "forum",
        ChannelKind::Stage => "stage",
        ChannelKind::Directory => "directory",
        ChannelKind::Media => "media",
        ChannelKind::Events => "events",
        ChannelKind::Dm => "dm",
    }
    .to_string()
}

fn role_to_display(entry: &RoleEntry) -> RoleDisplay {
    RoleDisplay {
        id: entry.id,
        name: entry.name.clone(),
        color: entry.color,
        permissions: entry.permissions,
        position: entry.position,
    }
}

/// Abbreviate a hex key for display: first 8 + "…" + last 4.
fn abbreviate_key(key: &str) -> String {
    if key.len() > 12 {
        format!("{}…{}", &key[..8], &key[key.len() - 4..])
    } else {
        key.to_string()
    }
}

/// Decrypt a channel message body using the MEK cache.
///
/// Returns `(body, is_encrypted, needs_mek_generation)`.
///
/// If the MEK for the message's generation is cached, decrypts and returns
/// the UTF-8 body. If the MEK is missing, returns a placeholder body with
/// `is_encrypted = true` and `needs_mek = Some(generation)` so the caller
/// can request the MEK and retry.
fn decrypt_channel_body(
    mek_cache: &MekCache,
    community_id: &str,
    channel_id: &str,
    msg: &ChannelMessage,
) -> (String, bool, Option<u64>) {
    if msg.ciphertext.is_empty() {
        return (String::new(), false, None);
    }

    let Some(mek) = mek_cache.get_generation(community_id, channel_id, msg.mek_generation) else {
        return (
            format!("[encrypted, MEK gen {} not cached]", msg.mek_generation),
            true,
            Some(msg.mek_generation),
        );
    };

    match mek.decrypt(&msg.ciphertext) {
        Ok(plaintext) => {
            let body = String::from_utf8_lossy(&plaintext).into_owned();
            (body, false, None)
        }
        Err(e) => {
            tracing::debug!(
                generation = msg.mek_generation,
                error = %e,
                "MEK decryption failed — key may be stale or message corrupt"
            );
            (
                format!("[decryption failed, MEK gen {}]", msg.mek_generation),
                true,
                None, // MEK exists but decryption failed — don't request again
            )
        }
    }
}
