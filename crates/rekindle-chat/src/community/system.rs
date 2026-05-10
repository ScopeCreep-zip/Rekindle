//! System-level community operations — announcements, moderation alerts,
//! bootstrap, sync, lockdown.
//!
//! System operations are operator-level capabilities that affect the entire
//! community or provide infrastructure services (bootstrap for new members,
//! sync for history replay).
//!
//! Announcements and alerts broadcast to all mesh peers (gossip dedup).
//! Bootstrap and sync are point-to-point (gossip direct to the requester/joiner).
//! Lockdown modifies channel state (gossip dedup for notification).
//!
//! All operations require operator status except:
//! - bootstrap_request (any new member can request)
//! - sync_request (any member can request history)

use rekindle_types::gossip_payload::{ControlPayload, GossipPayload};

use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

impl CommunityService {
    // ── Announcements ───────────────────────────────────────────────

    /// Broadcast a system message to all community members.
    ///
    /// System messages are distinguished from user messages in the TUI —
    /// they appear as centered, dimmed text without a sender attribution.
    /// Used for: community-wide announcements, maintenance notices, policy
    /// changes, scheduled downtime warnings.
    pub async fn system_message(
        &self, governance_key: &str, body: &str,
    ) -> Result<(), ChatError> {
        self.require_operator(governance_key)?;
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::SystemMessage {
                body: body.into(),
                timestamp: timestamp_ms(),
            },
        )).await?;
        tracing::info!(
            governance = &governance_key[..12.min(governance_key.len())],
            body_len = body.len(),
            "system message broadcast"
        );
        Ok(())
    }

    // ── Moderation Alerts ───────────────────────────────────────────

    /// Toggle raid alert mode for the community.
    ///
    /// When active, the TUI displays a prominent warning banner. Operators
    /// use this to signal coordinated attacks (spam bots, mass join floods,
    /// content attacks). Clients may auto-enable stricter filtering when
    /// raid alert is active.
    pub async fn raid_alert(
        &self, governance_key: &str, active: bool,
    ) -> Result<(), ChatError> {
        self.require_operator(governance_key)?;
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::RaidAlert { active },
        )).await?;
        tracing::info!(
            governance = &governance_key[..12.min(governance_key.len())],
            active,
            "raid alert broadcast"
        );
        Ok(())
    }

    /// Toggle lockdown mode for the entire community.
    ///
    /// When locked: non-operator members cannot send messages, create threads,
    /// add reactions, or join voice channels. Enforcement is in the messaging
    /// send path (channel.rs) — not advisory, real enforcement.
    ///
    /// Lockdown state is:
    /// 1. Written to governance manifest MANIFEST_POLICIES for persistence
    /// 2. Cached in session_meta.locked_down for zero-latency send-path checks
    /// 3. Broadcast via gossip for immediate UI banner display on all peers
    pub async fn channel_lockdown(
        &self, governance_key: &str, locked: bool,
    ) -> Result<(), ChatError> {
        self.require_operator(governance_key)?;
        let keypair = self.require_governance_keypair(governance_key)?;

        // Persist lockdown state to governance manifest (survives restarts)
        let policy = serde_json::json!({ "locked_down": locked });
        let policy_bytes = serde_json::to_vec(&policy)
            .map_err(|e| ChatError::Serialization(format!("lockdown policy: {e}")))?;
        self.io.write_record(
            governance_key, rekindle_types::dht_types::MANIFEST_POLICIES,
            &policy_bytes, Some(&keypair), crate::io::Confirm::Accepted,
        ).await?;

        // Update local cached state (send path reads this)
        {
            let mut meta = self.session_meta.write();
            if let Some(membership) = meta.communities.get_mut(governance_key) {
                membership.locked_down = locked;
            }
        }

        // Broadcast gossip for immediate UI feedback on all peers
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::ChannelLockdown { locked },
        )).await?;

        tracing::info!(
            governance = &governance_key[..12.min(governance_key.len())],
            locked,
            "lockdown enforced — DHT persisted + session_meta cached + gossip broadcast"
        );
        Ok(())
    }

    /// Notify a specific member that they have been kicked.
    ///
    /// Sent directly (point-to-point) to the kicked member's pseudonym.
    /// Their client uses this to display a "you were kicked" modal and
    /// clean up community state locally.
    pub async fn kicked_notification(
        &self, governance_key: &str, target_pseudonym: &str,
    ) -> Result<(), ChatError> {
        self.require_operator(governance_key)?;
        self.io.send_gossip_direct(governance_key, target_pseudonym, GossipPayload::Control(
            ControlPayload::KickedNotification,
        )).await?;
        tracing::info!(
            governance = &governance_key[..12.min(governance_key.len())],
            target = &target_pseudonym[..12.min(target_pseudonym.len())],
            "kicked notification sent"
        );
        Ok(())
    }

    // ── Bootstrap ───────────────────────────────────────────────────

    /// Request a bootstrap data package from the community operator.
    ///
    /// New members send this after join approval to receive the full
    /// community state (governance entries, member list, channel MEKs,
    /// recent message history) in a single response instead of reading
    /// each DHT record individually.
    ///
    /// Any member may send this (not operator-only). The operator's daemon
    /// handles the request via handle_gossip → BootstrapRequest case.
    pub async fn bootstrap_request(
        &self, governance_key: &str,
    ) -> Result<(), ChatError> {
        let joiner = self.io.pseudonym_hex(governance_key)?;
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::BootstrapRequest {
                joiner_pseudonym: joiner,
                governance_key: governance_key.into(),
            },
        )).await?;
        tracing::info!(
            governance = &governance_key[..12.min(governance_key.len())],
            "bootstrap request broadcast"
        );
        Ok(())
    }

    /// Send a bootstrap response to a specific joiner (operator only).
    ///
    /// Contains serialized governance entries, member list, channel MEKs
    /// (wrapped for the joiner), and recent messages. Sent point-to-point
    /// to the joiner's pseudonym (not broadcast to the entire mesh).
    pub async fn bootstrap_response(
        &self,
        governance_key: &str,
        target_pseudonym: &str,
        governance_entries: Vec<Vec<u8>>,
        member_list: Vec<Vec<u8>>,
        channel_meks: Vec<Vec<u8>>,
        recent_messages: Vec<Vec<u8>>,
        wrapped_owner_keypair: Vec<u8>,
    ) -> Result<(), ChatError> {
        self.require_operator(governance_key)?;
        self.io.send_gossip_direct(governance_key, target_pseudonym, GossipPayload::Control(
            ControlPayload::BootstrapResponse {
                governance_entries,
                member_list,
                channel_meks,
                recent_messages,
                wrapped_owner_keypair,
            },
        )).await?;
        tracing::info!(
            governance = &governance_key[..12.min(governance_key.len())],
            target = &target_pseudonym[..12.min(target_pseudonym.len())],
            "bootstrap response sent"
        );
        Ok(())
    }

    // ── History Sync ────────────────────────────────────────────────

    /// Request message history for a channel since a given timestamp.
    ///
    /// Any member may request sync. The community's archiver node (if
    /// designated) or the operator responds with SyncResponse containing
    /// the requested messages.
    pub async fn sync_request(
        &self, governance_key: &str, channel_id: &str, since_timestamp: u64,
    ) -> Result<(), ChatError> {
        self.io.broadcast_gossip_dedup(governance_key, GossipPayload::Control(
            ControlPayload::SyncRequest {
                channel_id: channel_id.into(),
                since_timestamp,
            },
        )).await?;
        tracing::debug!(
            governance = &governance_key[..12.min(governance_key.len())],
            channel = channel_id,
            since_timestamp,
            "sync request broadcast"
        );
        Ok(())
    }

    /// Respond to a sync request with channel message history.
    ///
    /// Sent point-to-point to the requester. Messages are MEK-encrypted
    /// blobs — the requester decrypts with their cached MEKs.
    pub async fn sync_response(
        &self,
        governance_key: &str,
        target_pseudonym: &str,
        channel_id: &str,
        messages: Vec<Vec<u8>>,
    ) -> Result<(), ChatError> {
        let message_count = messages.len();
        self.io.send_gossip_direct(governance_key, target_pseudonym, GossipPayload::Control(
            ControlPayload::SyncResponse {
                channel_id: channel_id.into(),
                messages,
            },
        )).await?;
        tracing::debug!(
            governance = &governance_key[..12.min(governance_key.len())],
            target = &target_pseudonym[..12.min(target_pseudonym.len())],
            channel = channel_id,
            message_count,
            "sync response sent"
        );
        Ok(())
    }
}
