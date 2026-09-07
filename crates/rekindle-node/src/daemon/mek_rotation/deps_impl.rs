//! `MekDistributeDeps` for the daemon.
//!
//! Most methods delegate to `DaemonGovernanceAdapter` through
//! `GovernanceRuntimeDeps` rather than reaching into `DaemonContext`
//! themselves. Identity, Lamport counter, membership and the online
//! roster are all questions that adapter already answers, and answering
//! them a second way here is how two adapters over one context drift
//! apart — the failure this branch exists to remove. What stays local
//! is what the governance trait has no notion of: MEK delivery,
//! rotation events, and the `MEKRotated` broadcast.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_mek_rotation::{
    ChannelMekCache, MekDistributeDeps, MekPersist, MekRotationError, MekRotationEvent,
    RotationRecipient,
};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{CryptoEvent, SubscriptionEvent};

use super::DaemonMekAdapter;
use crate::daemon::governance_adapter::{DaemonGovernanceAdapter, COMMUNITY_MEK_SLOT};

/// How long a single wrapped-MEK delivery may take.
///
/// Longer than the default RPC timeout because these calls route through
/// private routes and often relays. The rotation is already
/// asynchronous, so patience costs nothing here while a premature
/// timeout costs a member their key.
const MEK_DELIVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl DaemonMekAdapter {
    /// A short-lived governance adapter borrowed from our own `Arc`.
    fn governance(&self) -> DaemonGovernanceAdapter<'_> {
        DaemonGovernanceAdapter::new(&self.ctx)
    }

    fn transport(&self) -> Option<Arc<rekindle_transport::TransportNode>> {
        self.ctx.transport.read().as_ref().map(Arc::clone)
    }

    /// Publish a `SubscriptionEvent` if anyone is listening.
    fn publish(&self, event: SubscriptionEvent) {
        let guard = self.ctx.subscriptions.read();
        if let Some(manager) = guard.as_ref() {
            // Fails only when every receiver has dropped, which is the
            // normal state with no clients attached.
            let _ = manager.event_sender().send(event);
        }
    }

    /// Map the rotation crate's channel convention onto the cache's.
    ///
    /// `None`/empty means the community-wide key, which lives under the
    /// reserved slot the governance adapter reads from. Writing it under
    /// an empty channel id would put it where nothing looks for it.
    fn cache_channel_id(channel_id: Option<&str>) -> String {
        channel_id
            .filter(|id| !id.is_empty())
            .map_or_else(|| COMMUNITY_MEK_SLOT.to_string(), ToString::to_string)
    }
}

#[async_trait]
impl MekDistributeDeps for DaemonMekAdapter {
    fn cache(&self) -> Arc<dyn ChannelMekCache> {
        Arc::clone(&self.cache)
    }

    fn persist(&self) -> Arc<dyn MekPersist> {
        Arc::clone(&self.persist)
    }

    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey> {
        self.governance()
            .community_membership(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .map(|hex| PseudonymKey::from_hex_lossy(&hex))
    }

    fn online_recipients(
        &self,
        community_id: &str,
        exclude_pseudonym: Option<&str>,
    ) -> Vec<RotationRecipient> {
        let excluded = exclude_pseudonym.unwrap_or_default();
        self.governance()
            .online_members(community_id)
            .into_iter()
            // A member with no route blob cannot be app_called, so
            // keeping them would only add a guaranteed failure to the
            // delivery loop.
            .filter(|m| !m.route_blob.is_empty() && m.pseudonym_hex != excluded)
            .map(|m| RotationRecipient {
                pseudonym_hex: m.pseudonym_hex,
                route_blob: m.route_blob,
            })
            .collect()
    }

    async fn voice_recipients(
        &self,
        _community_id: &str,
        _channel_id: &str,
        _trigger_pseudonym: &str,
        _include_trigger_in_recipients: bool,
    ) -> Vec<RotationRecipient> {
        // The daemon hosts no voice engine — there is no voice channel
        // transport to enumerate participants from, which is why
        // `InboundCall::CallInvite` is rejected here too. Empty makes
        // `rotate_voice_mek_for_membership` a no-op rather than a wrong
        // answer. Recorded as a capability gap rather than silently
        // stubbed: when the daemon grows a voice runtime, this method
        // has to grow with it.
        Vec::new()
    }

    async fn broadcast_to_peer(
        &self,
        _community_id: &str,
        peer_pseudonym_hex: &str,
        route_blob: &[u8],
        envelope_bytes: Vec<u8>,
    ) -> Result<Vec<u8>, MekRotationError> {
        let node = self
            .transport()
            .ok_or_else(|| MekRotationError::Transport("transport not started".into()))?;
        let target = node
            .import_route(route_blob)
            .map_err(|e| MekRotationError::Transport(format!("import route: {e}")))?;
        // Unframed and unsigned — the format the desktop's inbound
        // handler reads. Safe here and only here because the payload is
        // sealed to the recipient via ECDH against our pseudonym key,
        // so the sender is authenticated by whether it decrypts at all.
        // See `Caller::call_community_envelope`.
        node.caller()
            .call_community_envelope(&target, envelope_bytes, MEK_DELIVERY_TIMEOUT)
            .await
            .map_err(|e| {
                MekRotationError::Transport(format!("MEK app_call to {peer_pseudonym_hex}: {e}"))
            })
    }

    fn emit_event(&self, event: MekRotationEvent) {
        let mapped = match event {
            MekRotationEvent::RotationStarted {
                community_id,
                channel_id,
                new_generation,
                initiator_pseudonym_hex,
            } => CryptoEvent::MekRotated {
                community: community_id,
                channel: Some(channel_id),
                generation: new_generation,
                rotator_pseudonym: Some(initiator_pseudonym_hex),
            },
            MekRotationEvent::RotationComplete {
                community_id,
                channel_id,
                generation,
            } => CryptoEvent::MekRotated {
                community: community_id,
                channel: Some(channel_id),
                generation,
                rotator_pseudonym: None,
            },
            MekRotationEvent::MekDelivered {
                community_id,
                channel_id,
                generation,
                sender_pseudonym_hex,
            } => CryptoEvent::MekTransferred {
                community: community_id,
                channel: Some(channel_id),
                generation,
                sender_pseudonym: sender_pseudonym_hex,
            },
            // No `CryptoEvent` counterpart. Traced rather than dropped
            // silently, so a community that has stopped rotating shows
            // up in the daemon log instead of only in its symptoms.
            MekRotationEvent::RotationFailed {
                community_id,
                channel_id,
                reason,
            } => {
                tracing::warn!(
                    community = %community_id,
                    channel = %channel_id,
                    %reason,
                    "MEK rotation failed"
                );
                return;
            }
        };
        self.publish(SubscriptionEvent::Crypto(mapped));
    }

    fn current_lamport(&self, community_id: &str) -> u64 {
        self.governance()
            .community_membership(community_id)
            .map_or(0, |m| m.lamport_counter)
    }

    fn increment_lamport(&self, community_id: &str) -> u64 {
        self.governance().increment_lamport(community_id)
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.governance().identity_secret()
    }

    fn apply_received_mek_to_state(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        mek: &MediaEncryptionKey,
    ) {
        self.cache.insert(
            community_id,
            &Self::cache_channel_id(channel_id),
            mek.clone(),
        );
    }

    fn persist_received_mek(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        mek: &MediaEncryptionKey,
    ) {
        // `MekPersist` is async and this method is not, so the write is
        // detached. Losing it costs a keyring entry, not the key — the
        // cache above already holds it for this process.
        let persist = Arc::clone(&self.persist);
        let community_id = community_id.to_string();
        let channel_id = Self::cache_channel_id(channel_id);
        let generation = mek.generation();
        let bytes = mek.as_bytes().to_vec();
        tokio::spawn(async move {
            if let Err(e) = persist
                .store_mek_for_generation(&community_id, &channel_id, generation, bytes)
                .await
            {
                tracing::debug!(error = %e, "persisting received MEK failed");
            }
        });
    }

    fn emit_rotation_received(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        generation: u64,
    ) {
        self.publish(SubscriptionEvent::Crypto(CryptoEvent::MekRotated {
            community: community_id.to_string(),
            channel: channel_id.map(ToString::to_string),
            generation,
            rotator_pseudonym: None,
        }));
    }

    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: rekindle_types::governance::GovernanceEntry,
    ) -> Result<(), MekRotationError> {
        rekindle_governance_runtime::write_entry(&self.governance(), community_id, entry)
            .await
            .map_err(|e| MekRotationError::InvalidInput(e.to_string()))
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), MekRotationError> {
        // Only `MEKRotated` reaches here — it is the one broadcast the
        // rotation orchestrators make. Matching the variant by name
        // rather than accepting any control payload keeps an unhandled
        // one loud instead of silently unsent, which is how the
        // daemon's governance `send_to_mesh` used to lose messages.
        let CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id,
            new_generation,
            ..
        }) = envelope
        else {
            return Err(MekRotationError::InvalidInput(
                "send_to_mesh: only MEKRotated is wired on the daemon track".into(),
            ));
        };

        let node = self
            .transport()
            .ok_or_else(|| MekRotationError::Transport("transport not started".into()))?;
        let meshes = {
            let guard = self.ctx.broadcast_mgr.read();
            let manager = guard.as_ref().ok_or_else(|| {
                MekRotationError::Transport("broadcast manager not started".into())
            })?;
            Arc::clone(manager.meshes())
        };
        let signing_key = self
            .identity_secret()
            .ok_or_else(|| MekRotationError::Transport("locked — cannot sign gossip".into()))?;
        let sender = self
            .my_pseudonym(community_id)
            .map(|p| hex::encode(p.0))
            .ok_or_else(|| {
                MekRotationError::InvalidInput("no pseudonym for this community".into())
            })?;

        let community_id = community_id.to_string();
        let channel_id = channel_id.clone();
        let new_generation = *new_generation;
        tokio::spawn(async move {
            let report = rekindle_transport::broadcast::gossip::mek_rotated(
                &node,
                &meshes,
                &community_id,
                &sender,
                channel_id.as_deref(),
                new_generation,
                &signing_key,
            )
            .await;
            if report.delivered == 0 && !report.failures.is_empty() {
                tracing::debug!(
                    community = %community_id,
                    failures = report.failures.len(),
                    "gossip: MEKRotated reached no peers"
                );
            }
        });
        Ok(())
    }
}
