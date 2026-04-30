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
use serde::Serialize;

use crate::crypto::mek::{MekCache, MekCacheEntrySnapshot};
use crate::dht::DhtStore;
use crate::error::{TransportError, Result};
use crate::payload::dht_types::{
    ChannelEntry, ChannelKind, ChannelMessage,
    MemberSummary, RoleEntry,
};
use crate::peer::PeerRegistry;
use crate::session::CommunityMembership;
use crate::shared::SharedState;

// ── Display types ───────────────────────────────────────────────────────

/// Overview of a joined community for list display.
#[derive(Debug, Clone, Serialize)]
pub struct CommunityOverview {
    pub governance_key: String,
    pub name: String,
    pub description: String,
    pub member_count: u32,
    pub channel_count: u32,
    pub our_pseudonym: String,
}

/// Detailed community info for the info command and TUI view.
#[derive(Debug, Clone, Serialize)]
pub struct CommunityDetail {
    pub governance_key: String,
    pub name: String,
    pub description: String,
    pub owner_pseudonym: String,
    pub created_at: u64,
    pub member_count: u32,
    pub channels: Vec<ChannelOverviewDisplay>,
    pub roles: Vec<RoleDisplay>,
    pub our_pseudonym: String,
    pub our_roles: Vec<u32>,
}

/// Channel info for list and tree display.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelOverviewDisplay {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category_id: Option<String>,
    pub topic: String,
    pub mek_generation: u64,
    pub log_key: Option<String>,
    pub sort_order: u16,
}

/// Decrypted channel message for history display.
#[derive(Debug, Clone, Serialize)]
pub struct DecryptedMessageDisplay {
    pub message_id: String,
    pub sequence: u64,
    pub author_pseudonym: String,
    pub author_display_name: String,
    pub body: String,
    pub timestamp: u64,
    pub reply_to_sequence: Option<u64>,
    pub mek_generation: u64,
    /// True if the message body is still encrypted (MEK not cached).
    pub is_encrypted: bool,
    /// If encrypted, which MEK generation is needed to decrypt.
    pub needs_mek: Option<u64>,
}

/// Friend with resolved display name and presence.
#[derive(Debug, Clone, Serialize)]
pub struct FriendDisplay {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: String,
    pub status_message: String,
    pub last_seen_ms: Option<u64>,
    pub profile_dht_key: Option<String>,
    pub has_route: bool,
}

/// DM conversation thread for inbox display.
#[derive(Debug, Clone, Serialize)]
pub struct DmThreadDisplay {
    pub peer_key: String,
    pub peer_name: String,
    pub last_message_at: u64,
    pub unread_count: u32,
    pub messages: Vec<DmMessageDisplay>,
}

/// Single DM message for display.
#[derive(Debug, Clone, Serialize)]
pub struct DmMessageDisplay {
    pub sender_key: String,
    pub sender_name: String,
    pub body: String,
    pub timestamp: u64,
    pub is_self: bool,
}

/// Role info for display.
#[derive(Debug, Clone, Serialize)]
pub struct RoleDisplay {
    pub id: u32,
    pub name: String,
    pub color: u32,
    pub permissions: u64,
    pub position: i32,
}

/// Node health data for the doctor diagnostic view.
#[derive(Debug, Clone, Serialize)]
pub struct NodeHealthDisplay {
    pub attachment: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    pub uptime_secs: u64,
    pub peer_count: usize,
    pub route_allocated: bool,
}

// ── QueryEngine ─────────────────────────────────────────────────────────

/// High-level query interface for CLI and TUI.
///
/// Obtained via [`TransportNode::query()`](crate::node::TransportNode).
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
    pub async fn list_channels(
        &self,
        governance_key: &str,
    ) -> Result<Vec<ChannelOverviewDisplay>> {
        let channels = self.dht.governance().read_channels(governance_key).await?;
        Ok(channels.iter().map(channel_to_display).collect())
    }

    /// Read channel message history with MEK decryption.
    ///
    /// Messages with missing MEK generations are returned with
    /// `is_encrypted = true` and `needs_mek = Some(generation)`.
    /// The body is set to a human-readable placeholder.
    pub async fn channel_history(
        &self,
        community_id: &str,
        channel_id: &str,
        channel_log_key: &str,
        registry_key: &str,
        limit: usize,
    ) -> Result<Vec<DecryptedMessageDisplay>> {
        // Read member index for author name resolution
        let members = self
            .dht
            .registry()
            .read_member_index(registry_key)
            .await
            .unwrap_or_default();

        // Read raw channel messages from the log
        let dht_log = crate::dht::channel_log::DhtLog::open_read(
            self.dht.routing_context(),
            channel_log_key,
        )
        .await?;

        #[allow(clippy::cast_possible_truncation)]
        let raw_entries = dht_log.tail(limit as u32).await?;

        let mek_cache = self.mek_cache.read();
        let mut messages = Vec::with_capacity(raw_entries.len());

        for raw in &raw_entries {
            let channel_msg: ChannelMessage = match serde_json::from_slice(raw) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "skipping malformed channel message");
                    continue;
                }
            };

            let author_name = resolve_pseudonym_name(
                &channel_msg.sender_pseudonym,
                &members,
            );

            let message_id = channel_msg
                .message_id
                .clone()
                .unwrap_or_else(|| format!("seq:{}", channel_msg.sequence));

            let (body, is_encrypted, needs_mek) =
                decrypt_channel_body(&mek_cache, community_id, channel_id, &channel_msg);

            messages.push(DecryptedMessageDisplay {
                message_id,
                sequence: channel_msg.sequence,
                author_pseudonym: channel_msg.sender_pseudonym,
                author_display_name: author_name,
                body,
                timestamp: channel_msg.timestamp,
                reply_to_sequence: channel_msg.reply_to,
                mek_generation: channel_msg.mek_generation,
                is_encrypted,
                needs_mek,
            });
        }

        Ok(messages)
    }

    // ── Friend queries ──────────────────────────────────────────────

    /// Read friend list with resolved display names.
    ///
    /// For each friend, reads their profile display name and status
    /// subkeys. Profile reads that fail (peer offline, record unavailable)
    /// fall back to the stored nickname or public key abbreviation.
    pub async fn resolved_friends(
        &self,
        friend_list_key: &str,
    ) -> Result<Vec<FriendDisplay>> {
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
            let has_route = route_snapshot
                .get(i)
                .is_some_and(|(_, has)| *has);

            let (display_name, status, status_message, last_seen) =
                if let Some(ref profile_key) = friend.profile_dht_key {
                    self.read_profile_summary(profile_key).await.unwrap_or_else(|_| {
                        (
                            friend.nickname.clone().unwrap_or_else(|| abbreviate_key(&friend.public_key)),
                            "unknown".to_string(),
                            String::new(),
                            None,
                        )
                    })
                } else {
                    (
                        friend.nickname.clone().unwrap_or_else(|| abbreviate_key(&friend.public_key)),
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
        let dht_log = crate::dht::channel_log::DhtLog::open_read(
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
        let friend_names: std::collections::HashMap<&str, &str> = friends
            .friends
            .iter()
            .filter_map(|f| {
                f.nickname.as_deref().or(Some(f.public_key.as_str()))
                    .map(|name| (f.public_key.as_str(), name))
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
            let body = entry
                .get("body")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("[unreadable]")
                .to_string();
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

            let sender_name = friend_names
                .get(sender_key.as_str())
                .copied()
                .map_or_else(|| abbreviate_key(&sender_key), String::from);

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
                let peer_name = friend_names
                    .get(peer_key.as_str())
                    .copied()
                    .map_or_else(|| abbreviate_key(&peer_key), String::from);

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
    pub fn peer_snapshot(&self) -> Vec<crate::peer::PeerSnapshot> {
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
    pub fn node_health(&self, shared: &SharedState, route_allocated: bool) -> NodeHealthDisplay {
        NodeHealthDisplay {
            attachment: shared.attachment_state().to_string(),
            is_attached: shared.is_attached(),
            public_internet_ready: shared.public_internet_ready(),
            uptime_secs: shared.uptime().as_secs(),
            peer_count: self.peer_registry.read().route_count(),
            route_allocated,
        }
    }

    // ── Internal helpers ────────────────────────────────────────────

    /// Read profile display name, status, and last-seen from DHT.
    async fn read_profile_summary(
        &self,
        profile_key: &str,
    ) -> Result<(String, String, String, Option<u64>)> {
        use crate::payload::dht_types::{
            PROFILE_SUBKEY_DISPLAY_NAME, PROFILE_SUBKEY_STATUS,
            PROFILE_SUBKEY_STATUS_MESSAGE, STATUS_AWAY, STATUS_BUSY,
            STATUS_INVISIBLE, STATUS_OFFLINE, STATUS_ONLINE,
        };

        let profile = self.dht.profile();

        let display_name = match profile.get_subkey(profile_key, PROFILE_SUBKEY_DISPLAY_NAME).await? {
            Some(data) if !data.is_empty() => {
                String::from_utf8_lossy(&data).to_string()
            }
            _ => abbreviate_key(profile_key),
        };

        let (status, last_seen) = match profile.get_subkey(profile_key, PROFILE_SUBKEY_STATUS).await? {
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
                    let raw = i64::from_be_bytes(
                        data[1..9].try_into().unwrap_or([0; 8]),
                    );
                    // Timestamps are always positive; clamp negative to 0
                    Some(u64::try_from(raw).unwrap_or(0))
                } else {
                    None
                };
                (status_str.to_string(), last_seen_ms)
            }
            _ => ("unknown".to_string(), None),
        };

        let status_message = match profile.get_subkey(profile_key, PROFILE_SUBKEY_STATUS_MESSAGE).await? {
            Some(data) if !data.is_empty() => {
                String::from_utf8_lossy(&data).to_string()
            }
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

/// Resolve a pseudonym key to a display name from the member index.
fn resolve_pseudonym_name(pseudonym_key: &str, members: &[MemberSummary]) -> String {
    members
        .iter()
        .find(|m| m.pseudonym_key == pseudonym_key)
        .map_or_else(
            || abbreviate_key(pseudonym_key),
            |m| m.display_name.clone(),
        )
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
