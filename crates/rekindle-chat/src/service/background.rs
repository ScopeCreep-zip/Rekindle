//! ChatService background tasks + lock.

use std::sync::atomic::Ordering;

use rekindle_types::dm_store::DmStore;
use rekindle_types::subscription_events::{SubscriptionEvent, TypingEvent, TypingContext};

use crate::ChatError;

use super::ChatService;

impl ChatService {
    pub fn sweep_expired_skipped_keys(&self) -> Result<u64, ChatError> {
        let skipped = self.vault.sweep_expired_skipped_keys()?;
        let hashes = self.dm_deps.store()
            .sweep_dm_hashes(7 * 86400)
            .unwrap_or(0);
        if hashes > 0 {
            tracing::debug!(hashes, "swept expired DM message hashes");
        }
        Ok(skipped)
    }

    pub fn evict_sessions(&self, max: u64) -> Result<Vec<[u8; 32]>, ChatError> {
        Ok(self.vault.evict_oldest_sessions(max)?)
    }

    pub fn evict_expired_dedup(&self) {
        self.pipeline.evict_expired_dedup();
    }

    /// Collect expired typing indicators and emit TypingStopped events.
    pub fn collect_expired_typers(&self) {
        let (channel_expired, dm_expired) = {
            let mut state = self.pipeline.state().write();
            (
                state.typing.collect_expired_channel_typers(),
                state.typing.collect_expired_dm_typers(),
            )
        };

        for (community, channel, who) in channel_expired {
            self.pipeline.process(SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Channel { community, channel },
                who,
            }));
        }

        for peer_key in dm_expired {
            self.pipeline.process(SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Dm { peer_key: peer_key.clone() },
                who: peer_key,
            }));
        }
    }

    /// Mark session_meta as modified. The 5s background flush will persist it.
    pub fn mark_session_dirty(&self) {
        self.session_dirty.store(true, Ordering::Release);
    }

    /// Persist session.json if the dirty flag is set.
    pub fn flush_session_meta_if_dirty(&self) -> Result<(), ChatError> {
        if !self.session_dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let meta = self.session_meta.read().clone();
        let json = serde_json::to_vec_pretty(&meta)
            .map_err(|e| ChatError::Serialization(format!("session.json: {e}")))?;
        rekindle_storage::session_meta::save(&self.session_path, &self.session_mac_key, &json)
            .map_err(ChatError::Storage)?;
        tracing::debug!("session.json flushed");
        Ok(())
    }

    /// Check all communities for stale MEKs and rotate them.
    ///
    /// Called periodically (hourly) by the daemon's background task loop.
    /// Reads mek_rotation_interval_hours from each community's metadata,
    /// queries the vault for MEKs older than the interval, and rotates them.
    pub async fn check_mek_rotation(&self) -> u32 {
        let communities: Vec<(String, String)> = {
            let meta = self.session_meta.read();
            meta.communities.values()
                .filter(|m| m.is_operator)
                .map(|m| (m.governance_key.clone(), m.community_name.clone()))
                .collect()
        };

        let mut rotated = 0u32;
        for (gov_key, name) in &communities {
            let interval_hours = match self.community.read_governance_metadata_interval(gov_key).await {
                Some(h) if h > 0 => h,
                _ => continue,
            };
            let max_age_secs = u64::from(interval_hours) * 3600;
            let stale = match self.vault.stale_meks(gov_key, max_age_secs) {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(community = %name, error = %e, "stale MEK query failed");
                    continue;
                }
            };
            for (channel_id, _gen) in &stale {
                match self.mek_rotate(gov_key, channel_id).await {
                    Ok(new_gen) => {
                        tracing::info!(community = %name, channel = %channel_id, new_gen, "MEK auto-rotated");
                        rotated += 1;
                    }
                    Err(e) => {
                        tracing::warn!(community = %name, channel = %channel_id, error = %e, "MEK auto-rotation failed");
                    }
                }
            }
        }
        rotated
    }

    /// Check if the ML-KEM last-resort prekey needs rotation.
    /// Rotates if first used > 1 hour ago.
    pub async fn check_last_resort_rotation(&self) {
        let peers: Vec<String> = {
            let meta = self.session_meta.read();
            meta.dm_peers.keys().cloned().collect()
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut should_rotate = false;

        for peer_key in &peers {
            let prefix = &peer_key[..20.min(peer_key.len())];
            let label = format!("pq_lr_first_used:{prefix}");
            if let Ok(Some(value)) = self.vault.load_key(&label) {
                if value.len() == 8 {
                    let first_used = i64::from_le_bytes(
                        value[..8].try_into().unwrap_or([0; 8])
                    );
                    if now - first_used > 3600 {
                        tracing::info!(
                            peer = &peer_key[..12.min(peer_key.len())],
                            elapsed_secs = now - first_used,
                            "last-resort PQ prekey first-used > 1 hour"
                        );
                        should_rotate = true;
                        let _ = self.vault.delete_key(&label);
                    }
                }
            }
        }

        if should_rotate {
            if let Err(e) = self.identity.rotate_last_resort().await {
                tracing::warn!(error = %e, "last-resort rotation failed");
            }
        }
    }

    /// Populate gossip meshes for all joined communities.
    ///
    /// Called periodically (every 90s) by the daemon's background task loop.
    /// Re-reads the member registry from DHT for each community, discovers
    /// routes via the resolver (DHT fallback on cache miss), and upserts
    /// into the gossip mesh. This is populate, not refresh — it discovers
    /// NEW members that joined since the last cycle, not just refreshes
    /// existing ones. The mesh's own evict_stale (MEMBER_TTL=300s) handles
    /// members who left — their last_seen stops being refreshed, they age out.
    pub async fn refresh_community_routes(&self) -> u32 {
        let communities: Vec<(String, String)> = {
            let meta = self.session_meta.read();
            meta.communities.values()
                .map(|m| (m.governance_key.clone(), m.pseudonym_key.clone()))
                .collect()
        };

        for (gov_key, pseudonym) in &communities {
            let members = self.community.list_members(gov_key).await.unwrap_or_default();
            let pairs: Vec<(String, Option<String>)> = members.iter()
                .map(|m| (m.pseudonym_key.clone(), m.profile_dht_key.clone()))
                .collect();
            self.io.transport().populate_mesh(gov_key, pseudonym, &pairs).await;
        }

        let mesh_peers = self.io.transport().gossip_mesh_peer_count();
        if mesh_peers > 0 {
            tracing::debug!(
                communities = communities.len(),
                mesh_peers,
                "community mesh populate complete"
            );
        }
        mesh_peers as u32
    }

    /// Slow-path catch-up: read shared SMPL channel records for messages
    /// missed while offline or not delivered via gossip.
    ///
    /// For each community, iterates channel_record_keys from local membership
    /// (these are shared SMPL record keys stored in governance ChannelEntry).
    /// Inspects each SMPL record for populated subkeys, reads each one,
    /// delegates to messaging for MEK decrypt + vault store + event emit.
    /// EventDedup suppresses duplicates if gossip already delivered.
    pub async fn catch_up_channel_messages(&self) -> u32 {
        let communities: Vec<rekindle_types::session_types::CommunityMembership> = {
            let meta = self.session_meta.read();
            meta.communities.values().cloned().collect()
        };

        // Phase 1: Collect all (gov_key, channel_id, channel_key) triples
        // and fan out inspects with bounded concurrency.
        let mut inspect_tasks = tokio::task::JoinSet::new();
        for membership in &communities {
            if membership.channel_record_keys.is_empty() { continue; }
            let my_slot = membership.slot_index;
            for (channel_id, channel_key) in &membership.channel_record_keys {
                if channel_key.is_empty() { continue; }
                let io = self.io.clone();
                let gk = membership.governance_key.clone();
                let ch_id = channel_id.clone();
                let ch_key = channel_key.clone();
                inspect_tasks.spawn(async move {
                    let all_subkeys: Vec<u32> = (0..255u32).collect();
                    // catch-up uses local_seqs — "what do I have locally?"
                    let seqs = io.open_and_inspect(&ch_key, &all_subkeys).await
                        .map(|r| r.local_seqs);
                    (gk, ch_id, ch_key, my_slot, seqs)
                });
            }
        }

        // Phase 2: Collect inspect results, filter by DHT seq cache,
        // build read work list.
        struct ReadWork {
            gov_key: String,
            channel_id: String,
            channel_key: String,
            subkey: u32,
        }
        let mut reads: Vec<ReadWork> = Vec::new();

        while let Some(result) = inspect_tasks.join_next().await {
            let Ok((gov_key, channel_id, channel_key, my_slot, seqs_result)) = result else {
                continue;
            };
            let Ok(seqs) = seqs_result else { continue; };

            for (i, dht_seq) in seqs.iter().enumerate() {
                let Some(seq_val) = dht_seq else { continue };
                let subkey = i as u32;
                if subkey == my_slot { continue; }

                // DHT seq cache: skip if this subkey's DHT seq hasn't changed
                let dht_cache_key = format!("dht:{}:{}", channel_key, subkey);
                let cached_dht_seq = {
                    let meta = self.session_meta.read();
                    meta.communities.get(&gov_key)
                        .and_then(|m| m.last_seen_seqs.get(&dht_cache_key))
                        .copied()
                        .unwrap_or(0)
                };
                if u64::from(*seq_val) <= cached_dht_seq { continue; }

                reads.push(ReadWork { gov_key: gov_key.clone(), channel_id: channel_id.clone(), channel_key: channel_key.clone(), subkey });
            }
        }

        if reads.is_empty() {
            return 0;
        }

        tracing::info!(
            reads = reads.len(),
            "catch-up: reading changed SMPL subkeys"
        );

        // Phase 3: Fan out reads with bounded concurrency.
        let mut total_new = 0u32;
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
        let mut read_tasks = tokio::task::JoinSet::new();

        for work in reads {
            let io = self.io.clone();
            let sem = semaphore.clone();
            read_tasks.spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore not closed");
                let data = io.open_and_read(&work.channel_key, work.subkey, true).await;
                (work.gov_key, work.channel_id, work.channel_key, work.subkey, data)
            });
        }

        while let Some(result) = read_tasks.join_next().await {
            let Ok((gov_key, channel_id, channel_key, subkey, read_result)) = result else {
                continue;
            };
            let Ok(Some(data)) = read_result else { continue };
            if data.is_empty() { continue; }

            let app_seq = serde_json::from_slice::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| v.get("sequence").and_then(serde_json::Value::as_u64))
                .unwrap_or(0);

            let app_seq_key = format!("slot{}:{}", subkey, channel_id);
            let last_seen_app_seq = {
                let meta = self.session_meta.read();
                meta.communities.get(&gov_key)
                    .and_then(|m| m.last_seen_seqs.get(&app_seq_key))
                    .copied()
                    .unwrap_or(0)
            };

            if app_seq <= last_seen_app_seq {
                // Still update DHT seq cache even if app seq unchanged
                let dht_cache_key = format!("dht:{}:{}", channel_key, subkey);
                let mut meta = self.session_meta.write();
                if let Some(m) = meta.communities.get_mut(&gov_key) {
                    // Store current DHT seq so we skip this subkey next cycle
                    m.last_seen_seqs.insert(dht_cache_key, app_seq);
                }
                continue;
            }

            let sender = serde_json::from_slice::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| v.get("sender_pseudonym").and_then(serde_json::Value::as_str).map(String::from))
                .unwrap_or_else(|| format!("slot{subkey}"));

            self.messaging.handle_channel_log_change(
                &gov_key, &channel_id, &sender, Some(data),
            );

            {
                let dht_cache_key = format!("dht:{}:{}", channel_key, subkey);
                let mut meta = self.session_meta.write();
                if let Some(m) = meta.communities.get_mut(&gov_key) {
                    m.last_seen_seqs.insert(app_seq_key, app_seq);
                    m.last_seen_seqs.insert(dht_cache_key, app_seq);
                }
            }
            self.mark_session_dirty();
            total_new += 1;
        }
        total_new
    }

    /// DM SMPL catch-up: re-establish failed watches and read undelivered
    /// messages from DM SMPL records.
    ///
    /// Called periodically (every 30s) by the daemon's background task loop.
    /// For each peer in `dm_smpl_peers`:
    /// 1. If no active watch exists, attempt `open_and_watch` (re-establishment)
    /// 2. Read the peer's subkey via `open_and_read`
    /// 3. If data exists and is newer than what we've seen, call
    ///    `dm::handle_dm_subkey_change` to decrypt/persist/emit
    ///
    /// This is the poll fallback for DM delivery when DHT watches fail
    /// due to propagation timing or Veilid record eviction.
    pub async fn catch_up_dm_messages(&self) -> u32 {
        let dm_peers: Vec<(String, String)> = {
            let meta = self.session_meta.read();
            meta.dm_smpl_peers.iter()
                .map(|(peer_key, smpl)| (peer_key.clone(), smpl.record_key.clone()))
                .collect()
        };

        if dm_peers.is_empty() {
            return 0;
        }

        let mut caught = 0u32;
        let mut watches_established = 0u32;

        for (peer_key, record_key) in &dm_peers {
            let my_subkey = match self.vault.dm_get_session_meta(record_key) {
                Ok(Some(meta)) => meta.my_subkey,
                Ok(None) => {
                    tracing::warn!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        record_key = &record_key[..12.min(record_key.len())],
                        "dm catch-up: no session meta in vault — skipping peer"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        error = %e,
                        "dm catch-up: session meta load error — skipping peer"
                    );
                    continue;
                }
            };
            let peer_subkey = if my_subkey == 0 { 1u32 } else { 0u32 };

            // Re-establish watch if not active
            if self.watches.lookup(record_key).is_none() {
                match self.io.open_and_watch(
                    record_key, &[peer_subkey],
                    crate::events::registry::WatchKind::DmSmpl { record_key: record_key.clone() },
                    &self.watches,
                ).await {
                    Ok(()) => {
                        watches_established += 1;
                        tracing::info!(
                            peer = &peer_key[..12.min(peer_key.len())],
                            record_key = &record_key[..12.min(record_key.len())],
                            "dm catch-up: watch re-established"
                        );
                    }
                    Err(e) => {
                        tracing::debug!(
                            peer = &peer_key[..12.min(peer_key.len())],
                            error = %e,
                            "dm catch-up: watch re-establishment failed — will retry next cycle"
                        );
                    }
                }
            }

            // Inspect DHT seq before reading — only process if seq advanced
            let inspect_result = match self.io.open_and_inspect(record_key, &[peer_subkey]).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        error = %e,
                        "dm catch-up: inspect failed"
                    );
                    continue;
                }
            };
            let current_seq = inspect_result.local_seqs
                .get(peer_subkey as usize)
                .copied()
                .flatten();

            tracing::debug!(
                peer = &peer_key[..12.min(peer_key.len())],
                record_key = &record_key[..12.min(record_key.len())],
                peer_subkey,
                dht_seq = ?current_seq,
                "dm catch-up: inspected subkey"
            );

            if current_seq.is_none() {
                continue;
            }

            let data = match self.io.open_and_read(record_key, peer_subkey, true).await {
                Ok(Some(d)) if !d.is_empty() => d,
                _ => continue,
            };

            let data_fingerprint = {
                let h = blake3::hash(&data);
                let b = h.as_bytes();
                format!("{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
            };

            tracing::debug!(
                peer = &peer_key[..12.min(peer_key.len())],
                raw_len = data.len(),
                data_fingerprint,
                source = "catch-up",
                "dm catch-up: about to call handle_dm_subkey_change"
            );

            match crate::dm::handle_dm_subkey_change(
                &*self.dm_deps, record_key, peer_subkey, Some(&data),
            ).await {
                Ok(()) => {
                    caught += 1;
                    tracing::debug!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        dht_seq = current_seq,
                        "dm catch-up: message processed successfully"
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        dht_seq = current_seq,
                        data_fingerprint,
                        error = %e,
                        "dm catch-up: message processing failed"
                    );
                }
            }
        }

        if watches_established > 0 || caught > 0 {
            tracing::info!(
                peers = dm_peers.len(),
                watches_established,
                messages_caught = caught,
                "dm catch-up complete"
            );
        }

        caught
    }

    /// Scan join inboxes for all operator communities.
    ///
    /// Called periodically (every 30s) by the daemon's background task loop
    /// as a fallback for unreliable DHT watch notifications. The DHT watch
    /// fires on the first write to the inbox but may not re-fire for
    /// subsequent writes within the same watch TTL. This periodic scan
    /// ensures no join request sits unprocessed.
    pub async fn scan_join_inboxes(&self) -> u32 {
        let communities: Vec<String> = {
            let meta = self.session_meta.read();
            meta.communities.values()
                .filter(|m| m.is_operator)
                .map(|m| m.governance_key.clone())
                .collect()
        };

        tracing::info!(operator_communities = communities.len(), "join inbox scan tick");

        let mut total = 0u32;
        for gov_key in &communities {
            match self.community.process_join_inbox(gov_key).await {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!(
                            community = &gov_key[..20.min(gov_key.len())],
                            new_members = n,
                            "join inbox scan: members approved"
                        );
                    }
                    total += n;
                }
                Err(e) => {
                    tracing::warn!(
                        community = &gov_key[..20.min(gov_key.len())],
                        error = %e,
                        "join inbox scan failed"
                    );
                }
            }
        }
        total
    }

    /// Lock the chat service. Zeroize all in-memory secrets.
    ///
    /// Order: flush session.json → cancel watches → clear caches → clear signing key.
    pub async fn lock(&self) {
        if let Err(e) = self.flush_session_meta_if_dirty() {
            tracing::warn!(error = %e, "session.json flush failed during lock — \
                changes since last flush may be lost on next startup");
        }

        for (record_key, token) in self.watches.all_tokens() {
            if let Err(e) = self.io.cancel_watch(token).await {
                tracing::debug!(record_key, error = %e, "watch cancel failed during lock");
            }
        }

        self.session_cache.clear().await;
        self.mek_cache.clear();
        self.io.clear_identity();

        tracing::info!("chat service locked — identity, sessions, MEKs zeroized");
    }
}
