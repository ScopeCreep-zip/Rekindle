//! The Host binding for the verified voice control and media-key paths.

use async_trait::async_trait;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::permissions::compute_permissions;
use rekindle_secrets::media_sender_key::session;
use rekindle_transport::subscriptions::media_key::MediaKeyReply;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::lamport::{LamportClock, LamportError};
use rekindle_voice::signaling::{
    CommunityVoiceEvent, MediaKeyAnswer, StageChannelInfo, VoiceSignalingDeps,
};
use rekindle_voice::transport::VoiceTransport;
use rekindle_voice::{MediaKeySource, VoiceSessionDeps};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

use super::carrier::HostMediaKeyOutbox;
use super::deps::HostVoiceDeps;
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::channel::HostChannel;
use crate::host::community::state::ChannelType;

impl crate::host::Host {
    /// Deliver a verified voice control envelope from the desktop ingress.
    pub fn ingest_voice_signaling(&self, community: &str, sender: &str, payload: ControlPayload) {
        handle_signaling(&self.ctx, community, sender, payload);
    }

    /// Answer a bare voice media-key call admitted by the transport.
    pub async fn answer_voice_media_key_call(&self, payload: &ControlPayload) -> MediaKeyReply {
        answer_media_key_call(&self.ctx, payload).await
    }

    /// Rotate our channel voice key when a member is no longer admitted.
    pub fn voice_member_departed(&self, community: &str, departed: &str) {
        rekindle_voice::signaling::member_departed(
            &self.voice_signaling_deps(),
            community,
            departed,
        );
    }
}

pub(crate) fn handle_signaling(
    ctx: &Arc<DaemonContext>,
    community: &str,
    sender: &str,
    payload: ControlPayload,
) {
    let deps: Arc<dyn VoiceSignalingDeps> = Arc::new(HostVoiceDeps::new(Arc::clone(ctx), None));
    let community = community.to_owned();
    let sender = sender.to_owned();
    ctx.unlock_scope_or_closed()
        .spawn_or_drop("voice signaling", async move {
            rekindle_voice::signaling::handle_voice_signaling(deps, &community, &sender, payload)
                .await;
        });
}

pub(crate) async fn answer_media_key_call(
    ctx: &Arc<DaemonContext>,
    payload: &ControlPayload,
) -> MediaKeyReply {
    let deps: Arc<dyn VoiceSignalingDeps> = Arc::new(HostVoiceDeps::new(Arc::clone(ctx), None));
    match rekindle_voice::signaling::answer_media_key_call(&deps, payload).await {
        MediaKeyAnswer::Installed(ack) => MediaKeyReply::Installed(ack),
        MediaKeyAnswer::Requested => MediaKeyReply::Requested,
        MediaKeyAnswer::Refused => MediaKeyReply::Refused,
    }
}

#[async_trait]
impl VoiceSignalingDeps for HostVoiceDeps {
    fn clock(&self) -> Arc<dyn rekindle_utils::Clock> {
        self.voice().clock()
    }

    fn my_pseudonym(&self, community_id: &str) -> Option<String> {
        let key = self.ctx().my_pseudonym(community_id);
        (!key.is_empty()).then_some(key)
    }

    fn our_media_route_blob(&self) -> Option<Vec<u8>> {
        VoiceSessionDeps::our_media_route_blob(self)
    }

    fn stage_channel_info(&self, community_id: &str, channel_id: &str) -> Option<StageChannelInfo> {
        let communities = self.ctx().communities.map().read();
        let community = communities.get(community_id)?;
        let channel = community.channels.iter().find(|ch| ch.id == channel_id)?;
        Some(StageChannelInfo {
            is_stage: matches!(channel.channel_type, ChannelType::Stage),
            speakers: channel.stage_speakers.clone(),
            moderator: channel.stage_moderator.clone(),
        })
    }

    fn update_stage_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator: String,
    ) {
        let mut communities = self.ctx().communities.map().write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(channel) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                if let Some(t) = topic {
                    channel.topic = t;
                }
                channel.stage_speakers = speakers;
                channel.stage_moderator = Some(moderator);
            }
        }
    }

    fn online_voice_members(&self, community_id: &str) -> Vec<(String, Vec<u8>)> {
        self.governance().reachable_members(community_id)
    }

    fn decode_channel_id(&self, channel_id: &str) -> Option<[u8; 16]> {
        hex::decode(channel_id).ok()?.try_into().ok()
    }

    fn my_permissions(&self, community_id: &str, channel_id: Option<[u8; 16]>) -> u64 {
        let map = self.ctx().communities.map().read();
        let Some(community) = map.get(community_id) else {
            return 0;
        };
        let (Some(governance), Some(pseudonym)) = (
            community.governance_state.as_ref(),
            community
                .my_pseudonym_key
                .as_deref()
                .and_then(|key| rekindle_types::key_format::pseudonym_hex(key).ok()),
        ) else {
            return 0;
        };
        compute_permissions(
            &PseudonymKey(pseudonym.to_bytes()),
            channel_id.map(ChannelId).as_ref(),
            governance,
            self.ctx().clock.now_secs(),
        )
    }

    fn sender_has_perm(
        &self,
        community_id: &str,
        sender_pseudonym_hex: &str,
        perm_mask: u64,
    ) -> bool {
        use rekindle_governance::permissions::{compute_permissions, has_all_capabilities};
        use rekindle_types::id::PseudonymKey;

        let communities = self.ctx().communities.map().read();
        let Some(community) = communities.get(community_id) else {
            return false;
        };
        let Some(gov) = community.governance_state.as_ref() else {
            return false;
        };
        let Ok(bytes) = hex::decode(sender_pseudonym_hex) else {
            return false;
        };
        let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) else {
            return false;
        };
        let perms = compute_permissions(&PseudonymKey(arr), None, gov, self.ctx().clock.now_secs());
        has_all_capabilities(perms, perm_mask)
    }

    fn transport_handle(&self) -> Option<Arc<AsyncMutex<VoiceTransport>>> {
        self.voice().transport()
    }

    fn voice_engine_channel_id(&self) -> Option<String> {
        self.voice()
            .session()
            .map(|scope| scope.session_key().to_owned())
    }

    fn voice_engine_bound_to(&self, community_id: &str, channel_id: &str) -> bool {
        self.voice().session().is_some_and(|scope| {
            scope.community() == Some(community_id) && scope.session_key() == channel_id
        })
    }

    fn set_voice_engine_muted(&self, muted: bool) {
        self.voice().set_muted(muted);
    }

    fn set_voice_engine_deafened(&self, deafened: bool) {
        self.voice().set_deafened(deafened);
    }

    fn media_live_peers(&self) -> std::collections::HashSet<String> {
        self.voice().live_peers()
    }

    fn channel_sender_keys(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Arc<rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys> {
        MediaKeySource::channel_sender_keys(self, community_id, channel_id)
    }

    fn sign_media_session_key(
        &self,
        community_id: &str,
        channel_id: &str,
        session_key: &[u8],
    ) -> Option<Vec<u8>> {
        VoiceSessionDeps::sign_media_session_key(self, community_id, channel_id, session_key)
    }

    fn sign_sealed_media_key(
        &self,
        community_id: &str,
        channel_id: &str,
        recipient: &str,
        key_index: u64,
        sealed: &[u8],
    ) -> Option<Vec<u8>> {
        self.sign(
            community_id,
            &session::sealed_key_message(channel_id, recipient, key_index, sealed),
        )
    }

    async fn call_peer(
        &self,
        route_blob: &[u8],
        envelope: &CommunityEnvelope,
    ) -> Option<CommunityEnvelope> {
        let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(envelope).ok()?;
        let node = self.ctx().transport.read().clone()?;
        let target = node.import_route(route_blob).ok()?;
        let reply = node
            .caller()
            .call_community_envelope(&target, route_blob, bytes)
            .await
            .map_err(|error| tracing::debug!(%error, "voice app call failed"))
            .ok()?;
        rekindle_codec::capnp_envelope::try_decode_community_envelope(&reply)
            .ok()
            .flatten()
    }

    fn media_key_outbox(&self) -> Arc<dyn rekindle_voice::signaling::MediaKeyOutbox> {
        Arc::new(HostMediaKeyOutbox(Arc::downgrade(self.ctx())))
    }

    fn send_to_mesh(&self, community_id: &str, envelope: &CommunityEnvelope) {
        self.governance().send_to_mesh_impl(community_id, envelope);
    }

    fn advertise_media_capabilities(&self, community_id: &str, channel_id: &str) {
        VoiceSessionDeps::broadcast_media_capabilities(self, community_id, channel_id);
    }

    async fn persist_hand_raise(&self, community_id: String, channel_id: String, raised: bool) {
        if let Err(error) = HostChannel::new(self.ctx())
            .persist_hand_raise(&community_id, &channel_id, raised)
            .await
        {
            tracing::debug!(%error, "voice: hand raise not persisted");
        }
    }

    fn next_lamport(&self, community_id: &str) -> Result<u64, LamportError> {
        let mut map = self.ctx().communities.map().write();
        let state = map
            .get_mut(community_id)
            .ok_or(LamportError::UnknownCommunity)?;
        let next = LamportClock::new(state.message_clock).increment()?;
        state.message_clock = next;
        Ok(next)
    }

    fn stage_speakers(&self, community_id: &str, channel_id: &str) -> Vec<String> {
        let communities = self.ctx().communities.map().read();
        communities
            .get(community_id)
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.stage_speakers.clone())
            })
            .unwrap_or_default()
    }

    fn start_mcu_loop(&self) {
        let deps: Arc<dyn VoiceSessionDeps> = Arc::new(self.clone());
        if let Err(error) = rekindle_voice::session::start_mcu_loop(&deps) {
            tracing::warn!(%error, "voice MCU did not start");
        }
    }

    async fn stop_mcu_loop(&self) {
        let deps: Arc<dyn VoiceSessionDeps> = Arc::new(self.clone());
        rekindle_voice::session::stop_mcu_loop(&deps).await;
    }

    fn send_to_channel(&self, community_id: &str, channel_id: &str, envelope: &CommunityEnvelope) {
        super::io::send_to_channel(self, community_id, channel_id, envelope);
    }

    fn member_shown_name(&self, community_id: &str, pseudonym: &str) -> String {
        self.governance().member_shown_name(community_id, pseudonym)
    }

    fn my_display_name(&self) -> Option<String> {
        VoiceSessionDeps::my_display_name(self)
    }

    fn emit_event(&self, event: CommunityVoiceEvent) {
        super::signaling_events::emit(self, event);
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        VoiceSessionDeps::scope(self)
    }
}

#[cfg(test)]
#[path = "signaling_tests.rs"]
mod tests;
