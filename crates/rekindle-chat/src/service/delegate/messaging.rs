//! Messaging delegation — DM and channel message operations.

use rekindle_types::session_types::DmSmplPeer;

use crate::dm::DmError;
use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    // ── DMs (SMPL record path) ─────────────────────────────────

    pub async fn send_dm(
        &self, peer_key: &str, body: &str,
    ) -> Result<(), ChatError> {
        crate::validation::validate_key(peer_key, "peer key")?;
        crate::validation::validate_message_body(body)?;

        let record_key = {
            let meta = self.session_meta.read();
            meta.dm_smpl_peers.get(peer_key)
                .map(|p| p.record_key.clone())
                .ok_or_else(|| ChatError::NotFriends { peer_key: peer_key.into() })?
        };

        crate::dm::send_dm_message(&*self.dm_deps, &record_key, body).await?;
        Ok(())
    }

    pub async fn start_dm(
        &self, peer_key: &str, pseudonym: &str, is_group: bool,
    ) -> Result<String, ChatError> {
        crate::validation::validate_key(peer_key, "peer key")?;

        let record_key = crate::dm::start_dm(
            &*self.dm_deps, peer_key, pseudonym, is_group,
        ).await?;

        {
            let mut meta = self.session_meta.write();
            meta.dm_smpl_peers.insert(peer_key.to_string(), DmSmplPeer {
                record_key: record_key.clone(),
                is_group,
            });
        }
        self.mark_session_dirty();
        Ok(record_key)
    }

    pub async fn accept_dm(
        &self, record_key: &str,
    ) -> Result<(), ChatError> {
        crate::dm::accept_dm_invite(&*self.dm_deps, record_key).await?;
        Ok(())
    }

    pub fn dm_thread(
        &self, peer_key: &str, limit: u32,
    ) -> Result<Vec<rekindle_types::dm_store::DmMessageRecord>, ChatError> {
        let record_key = {
            let meta = self.session_meta.read();
            meta.dm_smpl_peers.get(peer_key)
                .map(|p| p.record_key.clone())
                .ok_or_else(|| {
                    tracing::warn!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        peer_count = meta.dm_smpl_peers.len(),
                        "dm_thread: peer_key NOT FOUND in dm_smpl_peers"
                    );
                    ChatError::NotFriends { peer_key: peer_key.into() }
                })?
        };
        let messages = self.dm_deps.store().dm_load_messages(&record_key, limit).map_err(DmError::from)?;
        tracing::info!(
            peer = &peer_key[..12.min(peer_key.len())],
            record_key = &record_key[..16.min(record_key.len())],
            message_count = messages.len(),
            limit,
            "dm_thread: query complete"
        );
        Ok(messages)
    }

    pub fn dm_inbox(
        &self, limit: u32,
    ) -> Result<Vec<rekindle_types::dm_store::DmConversation>, ChatError> {
        let mut convos = self.dm_deps.store().dm_list_conversations().map_err(DmError::from)?;
        convos.truncate(limit as usize);
        // Enrich: resolve record_key → peer Ed25519 pubkey from dm_smpl_peers.
        // DmConversation.record_key is a VLD0: DHT key. The TUI deserializes
        // it into DmThreadDisplay.peer_key via serde alias. But send_dm
        // looks up dm_smpl_peers by Ed25519 pubkey, not record_key. Without
        // this enrichment, the TUI sends VLD0: strings as peer_key which
        // fails at dm_smpl_peers lookup.
        let meta = self.session_meta.read();
        for convo in &mut convos {
            for (pubkey, smpl) in &meta.dm_smpl_peers {
                if smpl.record_key == convo.record_key {
                    convo.record_key = pubkey.clone();
                    // The peer's pubkey is the dm_smpl_peers key.
                    // Resolve their display name unconditionally —
                    // initiator_pseudonym from the database is the
                    // acceptor's name, not the peer's name.
                    if let Some(name) = meta.friend_display_names.get(pubkey) {
                        convo.initiator_pseudonym = name.clone();
                    }
                    break;
                }
            }
        }
        Ok(convos)
    }

    // ── Channels ───────────────────────────────────────────────

    pub async fn send_channel_message(
        &self, community: &str, channel: &str, body: &str, reply_to: Option<u64>,
    ) -> Result<crate::messaging::channel::ChannelSentResult, ChatError> {
        crate::validation::validate_message_body(body)?;
        self.messaging.send_channel_message(community, channel, body, reply_to).await
    }

    pub async fn edit_channel_message(
        &self, community: &str, channel: &str, message_id: &str, new_body: &str,
    ) -> Result<(), ChatError> {
        self.messaging.edit_channel_message(community, channel, message_id, new_body).await
    }

    pub async fn delete_channel_message(
        &self, community: &str, channel: &str, message_id: &str,
    ) -> Result<(), ChatError> {
        self.messaging.delete_channel_message(community, channel, message_id).await
    }

    pub async fn send_channel_typing(
        &self, community: &str, channel: &str,
    ) -> Result<(), ChatError> {
        self.messaging.send_channel_typing(community, channel).await
    }

    pub fn channel_history(
        &self, community: &str, channel: &str, limit: u32,
    ) -> Result<Vec<rekindle_storage::messages::ChannelRecord>, ChatError> {
        let (gov_key, channel_id) = {
            let meta = self.session_meta.read();
            let (_, m) = meta.resolve_community(community)
                .ok_or_else(|| ChatError::NotMember { community: community.into() })?;
            let cid = m.resolve_channel(channel)
                .map_err(|e| ChatError::ChannelNotFound {
                    community: community.into(),
                    channel: e.to_string(),
                })?;
            (m.governance_key.clone(), cid)
        };
        let results = self.vault.query_channel_history(&gov_key, &channel_id, limit)?;
        Ok(results)
    }

    // ── DM typing (ephemeral, not through dm/ module) ──────────

    pub async fn send_dm_typing(
        &self, peer_key: &str, typing: bool,
    ) -> Result<(), ChatError> {
        let payload = rekindle_types::dm_payload::DmPayload::Typing { typing };
        self.io.send_peer_notification(peer_key, payload, crate::io::Confirm::None).await?;
        Ok(())
    }
}
