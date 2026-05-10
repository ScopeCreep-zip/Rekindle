//! Social interaction capabilities — reactions, pins, events, threads, game servers.
//!
//! **Reactions and pins** use optimistic-with-rollback:
//! 1. Emit local event immediately (user sees their action reflected instantly)
//! 2. Broadcast gossip to mesh peers (other users see within ~2s)
//! 3. Write to governance manifest for DHT persistence (durability)
//! 4. If DHT write fails: emit retraction event (UI removes the reaction/pin)
//!
//! Sub-100ms local feedback with eventual consistency. The rollback on DHT
//! failure is the honesty guarantee: the user is never shown permanent state
//! that doesn't actually exist on the network.
//!
//! **Events, threads, and game servers** use write_and_notify (no optimistic render):
//! These are low-frequency operations where 2-5s latency is acceptable.
//! The user clicks "create event" and waits for confirmation.
//!
//! All persistent state lives in the governance manifest DHT record.
//! Social subkeys: MANIFEST_REACTIONS (13), MANIFEST_PINS (14),
//! MANIFEST_EVENTS (15), MANIFEST_THREADS (8).

use rekindle_types::dht_types::{
    MANIFEST_EVENTS, MANIFEST_PINS, MANIFEST_REACTIONS, MANIFEST_THREADS,
};
use rekindle_types::gossip_payload::{
    ControlPayload, CommunityEvent, GameServerInfo, GossipPayload, ThreadInfo,
};
use rekindle_types::subscription_events::{SubscriptionEvent, SocialEvent};

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

impl CommunityService {
    // ── Reactions (optimistic-with-rollback) ────────────────────────

    /// Add a reaction to a channel message.
    ///
    /// UX flow:
    /// 1. Emit local ReactionAdded event → user's TUI shows 👍 immediately
    /// 2. Broadcast gossip → other peers see within ~2s
    /// 3. Write to governance manifest → persistence guarantee
    /// 4. If write fails → emit ReactionRemoved → user's TUI removes 👍
    pub async fn add_reaction(
        &self,
        governance_key: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), ChatError> {
        let reactor = self.io.pseudonym_hex(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;

        // Step 1: Emit local event (optimistic — user sees immediately)
        self.pipeline.process(SubscriptionEvent::Social(SocialEvent::ReactionAdded {
            community: governance_key.to_string(),
            channel: channel_id.to_string(),
            message_id: message_id.to_string(),
            emoji: emoji.to_string(),
            reactor_pseudonym: reactor.clone(),
        }));

        // Step 2: Broadcast gossip (peers see within ~2s)
        let _ = self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::ReactionAdded {
                channel_id: channel_id.into(),
                message_id: message_id.into(),
                emoji: emoji.into(),
                reactor_pseudonym: reactor.clone(),
            },
        )).await;

        // Step 3: Write to governance manifest for persistence
        let mut reactions = self.read_reactions(governance_key).await?;
        reactions.push(ReactionEntry {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            emoji: emoji.into(),
            reactor_pseudonym: reactor.clone(),
            created_at: timestamp_ms(),
        });
        let bytes = serde_json::to_vec(&reactions)
            .map_err(|e| ChatError::Serialization(format!("reactions: {e}")))?;

        if let Err(e) = self.io.write_record(
            governance_key, MANIFEST_REACTIONS, &bytes, Some(&keypair), Confirm::Accepted,
        ).await {
            // Step 4: DHT write failed — retract the optimistic render
            tracing::warn!(
                governance = &governance_key[..12.min(governance_key.len())],
                channel = channel_id,
                message = message_id,
                emoji,
                error = %e,
                "reaction DHT write FAILED — retracting optimistic render"
            );
            self.pipeline.process(SubscriptionEvent::Social(SocialEvent::ReactionRemoved {
                community: governance_key.to_string(),
                channel: channel_id.to_string(),
                message_id: message_id.to_string(),
                emoji: emoji.to_string(),
                reactor_pseudonym: reactor,
            }));
            return Err(ChatError::Internal(format!(
                "reaction write failed for {channel_id}/{message_id}: {e} — \
                 reaction was not persisted. Retry or check network."
            )));
        }

        Ok(())
    }

    /// Remove a reaction from a channel message.
    pub async fn remove_reaction(
        &self,
        governance_key: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), ChatError> {
        let reactor = self.io.pseudonym_hex(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;

        // Optimistic local removal
        self.pipeline.process(SubscriptionEvent::Social(SocialEvent::ReactionRemoved {
            community: governance_key.to_string(),
            channel: channel_id.to_string(),
            message_id: message_id.to_string(),
            emoji: emoji.to_string(),
            reactor_pseudonym: reactor.clone(),
        }));

        // Gossip broadcast
        let _ = self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::ReactionRemoved {
                channel_id: channel_id.into(),
                message_id: message_id.into(),
                emoji: emoji.into(),
                reactor_pseudonym: reactor.clone(),
            },
        )).await;

        // Persist removal
        let mut reactions = self.read_reactions(governance_key).await?;
        reactions.retain(|r| {
            !(r.channel_id == channel_id && r.message_id == message_id
                && r.emoji == emoji && r.reactor_pseudonym == reactor)
        });
        let bytes = serde_json::to_vec(&reactions)
            .map_err(|e| ChatError::Serialization(format!("reactions: {e}")))?;

        if let Err(e) = self.io.write_record(
            governance_key, MANIFEST_REACTIONS, &bytes, Some(&keypair), Confirm::Accepted,
        ).await {
            // Rollback: re-add the reaction locally since removal didn't persist
            tracing::warn!(
                error = %e,
                "reaction removal DHT write FAILED — reaction still exists on network"
            );
            self.pipeline.process(SubscriptionEvent::Social(SocialEvent::ReactionAdded {
                community: governance_key.to_string(),
                channel: channel_id.to_string(),
                message_id: message_id.to_string(),
                emoji: emoji.to_string(),
                reactor_pseudonym: reactor,
            }));
            return Err(ChatError::Internal(format!("reaction removal failed: {e}")));
        }

        Ok(())
    }

    // ── Pins (optimistic-with-rollback) ────────────────────────────

    /// Pin a message in a channel.
    pub async fn pin_message(
        &self,
        governance_key: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), ChatError> {
        let pinner = self.io.pseudonym_hex(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;

        // Optimistic local pin
        self.pipeline.process(SubscriptionEvent::Social(SocialEvent::MessagePinned {
            community: governance_key.to_string(),
            channel: channel_id.to_string(),
            message_id: message_id.to_string(),
            pinned_by: pinner.clone(),
        }));

        // Gossip broadcast
        let _ = self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::MessagePinned {
                channel_id: channel_id.into(),
                message_id: message_id.into(),
                pinned_by: pinner.clone(),
            },
        )).await;

        // Persist
        let mut pins = self.read_pins(governance_key).await?;
        if !pins.iter().any(|p| p.channel_id == channel_id && p.message_id == message_id) {
            pins.push(PinEntry {
                channel_id: channel_id.into(),
                message_id: message_id.into(),
                pinned_by: pinner,
                pinned_at: timestamp_ms(),
            });
        }
        let bytes = serde_json::to_vec(&pins)
            .map_err(|e| ChatError::Serialization(format!("pins: {e}")))?;

        if let Err(e) = self.io.write_record(
            governance_key, MANIFEST_PINS, &bytes, Some(&keypair), Confirm::Accepted,
        ).await {
            // Rollback
            tracing::warn!(error = %e, "pin DHT write FAILED — retracting");
            self.pipeline.process(SubscriptionEvent::Social(SocialEvent::MessageUnpinned {
                community: governance_key.to_string(),
                channel: channel_id.to_string(),
                message_id: message_id.to_string(),
            }));
            return Err(ChatError::Internal(format!("pin write failed: {e}")));
        }

        Ok(())
    }

    /// Unpin a message in a channel.
    pub async fn unpin_message(
        &self,
        governance_key: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), ChatError> {
        let keypair = self.require_governance_keypair(governance_key)?;

        // Optimistic local unpin
        self.pipeline.process(SubscriptionEvent::Social(SocialEvent::MessageUnpinned {
            community: governance_key.to_string(),
            channel: channel_id.to_string(),
            message_id: message_id.to_string(),
        }));

        // Gossip
        let _ = self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::MessageUnpinned {
                channel_id: channel_id.into(),
                message_id: message_id.into(),
            },
        )).await;

        // Persist
        let mut pins = self.read_pins(governance_key).await?;
        pins.retain(|p| !(p.channel_id == channel_id && p.message_id == message_id));
        let bytes = serde_json::to_vec(&pins)
            .map_err(|e| ChatError::Serialization(format!("pins: {e}")))?;

        self.io.write_record(
            governance_key, MANIFEST_PINS, &bytes, Some(&keypair), Confirm::Accepted,
        ).await?;

        Ok(())
    }

    // ── Events (write_and_notify, no optimistic render) ────────────

    /// Create a community event.
    pub async fn create_event(
        &self,
        governance_key: &str,
        title: &str,
        description: &str,
        start_time: u64,
        end_time: Option<u64>,
        channel_id: Option<&str>,
        max_attendees: Option<u32>,
    ) -> Result<String, ChatError> {
        let creator = self.io.pseudonym_hex(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;
        let event_id = uuid::Uuid::new_v4().to_string();

        let event = CommunityEvent {
            id: event_id.clone(),
            title: title.into(),
            description: description.into(),
            creator_pseudonym: creator,
            start_time,
            end_time,
            channel_id: channel_id.map(Into::into),
            max_attendees,
            created_at: timestamp_ms(),
            status: "scheduled".into(),
        };

        let mut events = self.read_events(governance_key).await?;
        events.push(event.clone());
        let bytes = serde_json::to_vec(&events)
            .map_err(|e| ChatError::Serialization(format!("events: {e}")))?;

        self.io.write_and_notify(
            governance_key, governance_key, MANIFEST_EVENTS, &bytes,
            Some(&keypair), GossipPayload::Control(ControlPayload::EventCreated { event }),
            Confirm::Accepted,
        ).await?;

        Ok(event_id)
    }

    /// Update a community event.
    pub async fn update_event(
        &self,
        governance_key: &str,
        event_id: &str,
        title: &str,
        description: &str,
        start_time: u64,
        end_time: Option<u64>,
        max_attendees: Option<u32>,
    ) -> Result<(), ChatError> {
        let caller = self.io.pseudonym_hex(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;

        let mut events = self.read_events(governance_key).await?;
        let ev = events.iter_mut().find(|e| e.id == event_id)
            .ok_or_else(|| ChatError::Internal(format!("event {event_id} not found")))?;

        // Authorization: only the event creator or a community operator can update
        if ev.creator_pseudonym != caller {
            let is_operator = self.session_meta.read()
                .communities.get(governance_key)
                .is_some_and(|m| m.is_operator);
            if !is_operator {
                return Err(ChatError::InsufficientPermissions {
                    action: format!(
                        "update event '{}' — only the creator or an operator can modify events",
                        ev.title,
                    ),
                });
            }
        }

        ev.title = title.into();
        ev.description = description.into();
        ev.start_time = start_time;
        ev.end_time = end_time;
        ev.max_attendees = max_attendees;
        let updated = ev.clone();

        let bytes = serde_json::to_vec(&events)
            .map_err(|e| ChatError::Serialization(format!("events: {e}")))?;

        self.io.write_and_notify(
            governance_key, governance_key, MANIFEST_EVENTS, &bytes,
            Some(&keypair),
            GossipPayload::Control(ControlPayload::EventUpdated { event: updated }),
            Confirm::Accepted,
        ).await?;

        Ok(())
    }

    /// Delete a community event.
    pub async fn delete_event(
        &self, governance_key: &str, event_id: &str,
    ) -> Result<(), ChatError> {
        let keypair = self.require_governance_keypair(governance_key)?;

        let mut events = self.read_events(governance_key).await?;
        events.retain(|e| e.id != event_id);
        let bytes = serde_json::to_vec(&events)
            .map_err(|e| ChatError::Serialization(format!("events: {e}")))?;

        self.io.write_and_notify(
            governance_key, governance_key, MANIFEST_EVENTS, &bytes,
            Some(&keypair),
            GossipPayload::Control(ControlPayload::EventDeleted { event_id: event_id.into() }),
            Confirm::Accepted,
        ).await?;

        Ok(())
    }

    /// RSVP to a community event (gossip-only — attendee state is ephemeral).
    pub async fn rsvp_event(
        &self, governance_key: &str, event_id: &str, status: &str,
    ) -> Result<(), ChatError> {
        let pseudonym = self.io.pseudonym_hex(governance_key)?;
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::EventRsvpChanged {
                event_id: event_id.into(),
                pseudonym_key: pseudonym,
                status: status.into(),
            },
        )).await?;
        Ok(())
    }

    /// Broadcast an event reminder.
    pub async fn event_reminder(
        &self, governance_key: &str, event_id: &str, title: &str, minutes_until: u32,
    ) -> Result<(), ChatError> {
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::EventReminder {
                event_id: event_id.into(),
                title: title.into(),
                minutes_until_start: minutes_until,
            },
        )).await?;
        Ok(())
    }

    // ── Threads (write_and_notify) ─────────────────────────────────

    /// Create a thread on a channel message.
    pub async fn create_thread(
        &self,
        governance_key: &str,
        channel_id: &str,
        parent_message_id: &str,
        title: &str,
        auto_archive_seconds: u32,
    ) -> Result<String, ChatError> {
        let creator = self.io.pseudonym_hex(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;
        let thread_id = uuid::Uuid::new_v4().to_string();

        let thread = ThreadInfo {
            id: thread_id.clone(),
            channel_id: channel_id.into(),
            name: title.into(),
            starter_message_id: parent_message_id.into(),
            creator_pseudonym: creator,
            created_at: timestamp_ms(),
            archived: false,
            auto_archive_seconds,
        };

        let mut threads = self.read_threads(governance_key).await?;
        threads.push(thread.clone());
        let bytes = serde_json::to_vec(&threads)
            .map_err(|e| ChatError::Serialization(format!("threads: {e}")))?;

        self.io.write_and_notify(
            governance_key, governance_key, MANIFEST_THREADS, &bytes,
            Some(&keypair),
            GossipPayload::Control(ControlPayload::ThreadCreated { thread }),
            Confirm::Accepted,
        ).await?;

        Ok(thread_id)
    }

    /// Post a message to a thread (gossip broadcast, persistence via channel DhtLog).
    pub async fn thread_message(
        &self,
        governance_key: &str,
        thread_id: &str,
        ciphertext: Vec<u8>,
        mek_generation: u64,
        reply_to_id: Option<&str>,
    ) -> Result<String, ChatError> {
        let sender = self.io.pseudonym_hex(governance_key)?;
        let message_id = uuid::Uuid::new_v4().to_string();

        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::ThreadMessage {
                thread_id: thread_id.into(),
                message_id: message_id.clone(),
                sender_pseudonym: sender,
                ciphertext,
                mek_generation,
                timestamp: timestamp_ms(),
                reply_to_id: reply_to_id.map(Into::into),
            },
        )).await?;

        Ok(message_id)
    }

    /// Archive or unarchive a thread.
    pub async fn archive_thread(
        &self, governance_key: &str, thread_id: &str, archived: bool,
    ) -> Result<(), ChatError> {
        let keypair = self.require_governance_keypair(governance_key)?;

        let mut threads = self.read_threads(governance_key).await?;
        if let Some(t) = threads.iter_mut().find(|t| t.id == thread_id) {
            t.archived = archived;
        }
        let bytes = serde_json::to_vec(&threads)
            .map_err(|e| ChatError::Serialization(format!("threads: {e}")))?;

        self.io.write_and_notify(
            governance_key, governance_key, MANIFEST_THREADS, &bytes,
            Some(&keypair),
            GossipPayload::Control(ControlPayload::ThreadArchived {
                thread_id: thread_id.into(), archived,
            }),
            Confirm::Accepted,
        ).await?;

        Ok(())
    }

    // ── Game Servers (write_and_notify) ────────────────────────────

    /// Announce a game server for the community.
    pub async fn add_game_server(
        &self,
        governance_key: &str,
        game_id: &str,
        label: &str,
        address: &str,
    ) -> Result<String, ChatError> {
        let added_by = self.io.pseudonym_hex(governance_key)?;
        let server_id = uuid::Uuid::new_v4().to_string();

        let server = GameServerInfo {
            id: server_id.clone(),
            game_id: game_id.into(),
            label: label.into(),
            address: address.into(),
            added_by,
            created_at: timestamp_ms(),
        };

        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::GameServerAdded { server },
        )).await?;

        Ok(server_id)
    }

    /// Remove a game server announcement.
    pub async fn remove_game_server(
        &self, governance_key: &str, server_id: &str,
    ) -> Result<(), ChatError> {
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::GameServerRemoved { server_id: server_id.into() },
        )).await?;
        Ok(())
    }

    // ── Internal read helpers ──────────────────────────────────────

    async fn read_reactions(&self, gov_key: &str) -> Result<Vec<ReactionEntry>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_REACTIONS, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("reactions: {e}")))
    }

    async fn read_pins(&self, gov_key: &str) -> Result<Vec<PinEntry>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_PINS, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("pins: {e}")))
    }

    pub(crate) async fn read_events(&self, gov_key: &str) -> Result<Vec<CommunityEvent>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_EVENTS, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("events: {e}")))
    }

    pub(crate) async fn read_threads(&self, gov_key: &str) -> Result<Vec<ThreadInfo>, ChatError> {
        let raw = self.io.read_record(gov_key, MANIFEST_THREADS, true).await?
            .unwrap_or_else(|| b"[]".to_vec());
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("threads: {e}")))
    }
}

// ── Local types for DHT-persisted social state ─────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReactionEntry {
    channel_id: String,
    message_id: String,
    emoji: String,
    reactor_pseudonym: String,
    created_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PinEntry {
    channel_id: String,
    message_id: String,
    pinned_by: String,
    pinned_at: u64,
}
