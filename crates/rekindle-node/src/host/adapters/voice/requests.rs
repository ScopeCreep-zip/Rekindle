//! Start and stop the Host's one live voice session.

use std::sync::Arc;
use std::time::Duration;

use rekindle_types::audio::VoiceFailure;
use rekindle_types::id::ChannelId;
use rekindle_types::subscription_events::{SubscriptionEvent, VoiceScope};
use rekindle_voice::{AudioPrefs, CallMediaKeys, VoiceError, VoiceSessionDeps, VoiceShutdownOpts};

use super::deps::HostVoiceDeps;
use super::{DirectMedia, HostVoice};
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::{governance::HostGovernance, presence};

impl crate::host::Host {
    /// The one engine shared by the desktop and daemon request paths.
    #[must_use]
    pub fn voice(&self) -> &HostVoice {
        &self.ctx.voice
    }

    /// Reopen the live audio devices after their preferences were written.
    pub async fn reopen_voice_devices(
        &self,
        input: Option<String>,
        output: Option<String>,
    ) -> Result<(), String> {
        let _transition = self.voice().transitions.lock().await;
        if !self.voice().engine_present() {
            return Ok(());
        }
        let prefs = audio_prefs(&self.ctx).await?;
        let deps = Arc::new(HostVoiceDeps::new(Arc::clone(&self.ctx), Some(prefs)));
        rekindle_voice::session::change_audio_devices(&deps, input, output)
            .await
            .map_err(|error| session_notice(&error))
    }

    /// Reconfigure the held session's mixer topology.
    pub async fn set_voice_mode(&self, mode: &str, host: Option<String>) -> Result<(), String> {
        let _transition = self.voice().transitions.lock().await;
        let deps: Arc<dyn VoiceSessionDeps> =
            Arc::new(HostVoiceDeps::new(Arc::clone(&self.ctx), None));
        rekindle_voice::session::set_voice_mode(&deps, mode, host)
            .await
            .map_err(|error| session_notice(&error))
    }

    /// Existing signaling actions use the same deps as inbound voice.
    #[must_use]
    pub fn voice_signaling_deps(&self) -> Arc<dyn rekindle_voice::signaling::VoiceSignalingDeps> {
        Arc::new(HostVoiceDeps::new(Arc::clone(&self.ctx), None))
    }

    /// The call interpreter claims the engine with the accepted call's media keys.
    pub async fn start_direct_voice(
        &self,
        call_id: &str,
        peer: &str,
        keys: CallMediaKeys,
    ) -> Result<(), String> {
        let voice = self.voice();
        let _transition = voice.transitions.lock().await;
        if let Some(refusal) = start_refusal(&self.ctx) {
            return Err(refusal);
        }
        self.ctx
            .unlock_scope
            .read()
            .as_ref()
            .filter(|scope| !scope.is_closed())
            .ok_or_else(|| VoiceFailure::SignInRequired.notice())?;
        let prefs = audio_prefs(&self.ctx).await?;
        let deps = Arc::new(HostVoiceDeps::new(Arc::clone(&self.ctx), Some(prefs)));
        *voice.call.lock() = Some(DirectMedia {
            id: call_id.to_owned(),
            peer: peer.to_owned(),
            keys,
        });
        if let Err(error) = rekindle_voice::session::start_session(&deps, peer, None).await {
            rekindle_voice::session::shutdown_voice(&deps, &VoiceShutdownOpts::FULL).await;
            return Err(session_notice(&error));
        }
        Ok(())
    }

    /// Stop the engine only when the ending call owns its media.
    pub async fn stop_direct_voice(&self, call_id: &str) {
        let _transition = self.voice().transitions.lock().await;
        if self
            .voice()
            .call
            .lock()
            .as_ref()
            .is_some_and(|call| call.id == call_id)
        {
            stop_locked(&self.ctx).await;
        }
    }

    /// Pre-stage bounded direct media ingress before the accept is sent.
    pub fn pre_stage_direct_voice(&self) {
        if !self.voice().engine_present() {
            HostVoiceDeps::new(Arc::clone(&self.ctx), None).pre_stage_voice_channel();
        }
    }
}

pub(crate) fn reannounce_route(ctx: &DaemonContext) {
    if let Some(ctx) = ctx.this.upgrade() {
        let deps = Arc::new(HostVoiceDeps::new(ctx, None));
        rekindle_voice::session::reannounce_voice_route(&deps);
    }
}

async fn audio_prefs(ctx: &DaemonContext) -> Result<AudioPrefs, String> {
    let owner = ctx
        .session
        .read()
        .as_ref()
        .map(|s| s.identity.public_key_hex.clone())
        .ok_or_else(|| VoiceFailure::SignInRequired.notice())?;
    let db = ctx
        .db
        .current()
        .map_err(|_| VoiceFailure::SignInRequired.notice())?;
    let prefs = db
        .call(move |conn| rekindle_db::repo::preferences::load(conn, &owner))
        .await
        .inspect_err(|error| tracing::warn!(%error, "voice preferences unreadable"))
        .map_err(|_| VoiceFailure::Session.notice())?;
    Ok(AudioPrefs {
        noise_suppression: prefs.noise_suppression,
        echo_cancellation: prefs.echo_cancellation,
        input_volume: prefs.input_volume,
        output_volume: prefs.output_volume,
        input_device: prefs.input_device,
        output_device: prefs.output_device,
        input_channels: prefs.input_channels,
        io: ctx.voice.io,
    })
}

fn session_notice(error: &VoiceError) -> String {
    tracing::warn!(%error, "voice action failed");
    error.notice()
}

pub(crate) fn start_refusal(ctx: &DaemonContext) -> Option<String> {
    match ctx.voice.session()? {
        VoiceScope::Community { community, .. } => {
            let map = ctx.communities.map().read();
            Some(map.get(&community).map_or_else(
                || VoiceFailure::Session.notice(),
                |held| super::leave_voice_in(&held.name),
            ))
        }
        VoiceScope::Dm { .. } => Some(super::LEAVE_CALL_FIRST.to_owned()),
    }
}

pub(crate) async fn join(
    ctx: &DaemonContext,
    community: &str,
    channel: &str,
    muted: bool,
    deafened: bool,
) -> Result<(bool, bool), String> {
    let _transition = ctx.voice.transitions.lock().await;
    let ctx = ctx
        .this
        .upgrade()
        .ok_or_else(|| VoiceFailure::SignInRequired.notice())?;
    let channel_id = ChannelId::from_hex(channel).ok_or_else(|| VoiceFailure::Session.notice())?;
    let requested = super::events::scope_of(channel, Some(community));
    if ctx.voice.session().as_ref() == Some(&requested) {
        return ctx
            .voice
            .controls()
            .ok_or_else(|| VoiceFailure::NotConnected.notice());
    }
    if let Some(refusal) = start_refusal(&ctx) {
        return Err(refusal);
    }
    HostGovernance::new(Arc::clone(&ctx)).require_own_permissions(
        community,
        Some(&channel_id),
        rekindle_types::permissions::CONNECT,
    )?;
    let prefs = audio_prefs(&ctx).await?;
    let deps = HostVoiceDeps::new(Arc::clone(&ctx), Some(prefs));
    let kind = deps.channel(community, channel, |channel| channel.channel_type);
    use rekindle_types::channel::ChannelKind;
    if !matches!(kind, Some(ChannelKind::Voice | ChannelKind::Stage)) {
        return Err(VoiceFailure::ChannelUnavailable.notice());
    }
    let is_audience = deps.channel_is_stage(community, channel)
        && !deps.we_are_stage_speaker(community, channel, &ctx.my_pseudonym(community));
    let deps = Arc::new(deps.with_initial_controls(muted || is_audience, deafened));
    if let Err(error) =
        rekindle_voice::session::join_voice_channel(&deps, channel, Some(community)).await
    {
        rekindle_voice::session::shutdown_voice(&deps, &VoiceShutdownOpts::FULL).await;
        return Err(session_notice(&error));
    }
    rekindle_voice::session::set_local_mute(&deps, muted || is_audience);
    presence::voice_changed(&ctx, community);
    Ok((muted || is_audience, deafened))
}

pub(crate) async fn leave(ctx: &DaemonContext) -> Option<(String, String)> {
    let _transition = ctx.voice.transitions.lock().await;
    let VoiceScope::Community { community, channel } = ctx.voice.session()? else {
        return None;
    };
    stop_locked(ctx).await;
    presence::voice_changed(ctx, &community);
    Some((community, channel))
}

pub(crate) async fn stop(ctx: &DaemonContext) {
    let _transition = ctx.voice.transitions.lock().await;
    stop_locked(ctx).await;
}

pub(crate) async fn leave_community(ctx: &DaemonContext, community: &str) {
    let _transition = ctx.voice.transitions.lock().await;
    if ctx.voice.session().as_ref().and_then(VoiceScope::community) == Some(community) {
        stop_locked(ctx).await;
    }
}

async fn stop_locked(ctx: &DaemonContext) {
    let Some(ctx) = ctx.this.upgrade() else {
        return;
    };
    let deps = Arc::new(HostVoiceDeps::new(ctx, None));
    if tokio::time::timeout(
        Duration::from_secs(3),
        rekindle_voice::session::announce_leave(&deps),
    )
    .await
    .is_err()
    {
        tracing::warn!("voice leave announcement timed out");
    }
    rekindle_voice::session::shutdown_voice(&deps, &VoiceShutdownOpts::FULL).await;
}

pub(crate) fn spawn_telemetry(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let ctx = ctx.this.clone();
    scope.spawn_with_token("voice drop telemetry", move |stop| async move {
        loop {
            if stop
                .run_until_cancelled(tokio::time::sleep(Duration::from_secs(1)))
                .await
                .is_none()
            {
                return;
            }
            let Some(ctx) = ctx.upgrade() else { return };
            for event in ctx.voice.drops.drain(ctx.voice.session()) {
                ctx.publish_event(SubscriptionEvent::Voice(event));
            }
        }
    })
}
