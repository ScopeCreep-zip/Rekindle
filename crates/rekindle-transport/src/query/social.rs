//! Friend resolution, DM inbox, and profile reads.

use crate::error::Result;

use super::display_map::abbreviate_key;
use super::{DmMessageDisplay, DmThreadDisplay, FriendDisplay, QueryEngine};

impl QueryEngine {
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
        let raw_entries = dht_log
            .tail(u32::try_from(total_limit).unwrap_or(u32::MAX))
            .await?;

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
