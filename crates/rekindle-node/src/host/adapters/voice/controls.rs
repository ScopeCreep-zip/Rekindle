//! Local controls serialized with the Host's voice-session lifetime.

use std::sync::Arc;

use rekindle_types::dto::voice::VoiceControls;
use rekindle_types::subscription_events::{CallEvent, SubscriptionEvent, VoiceEvent};

use crate::daemon::dispatch::DaemonContext;
use crate::host::Host;

impl Host {
    /// Dispatch the operating system's mute shortcut on the unlock scope.
    pub fn toggle_voice_muted(&self) {
        let ctx = Arc::clone(&self.ctx);
        ctx.unlock_scope_or_closed()
            .spawn_or_drop("voice mute shortcut", async move {
                let _transition = ctx.voice.transitions.lock().await;
                if let Some(muted) = ctx.voice.toggle_muted() {
                    publish_mute(&ctx, muted);
                }
            });
    }
}

pub(crate) async fn set_mute(ctx: &DaemonContext, muted: bool) -> Option<VoiceControls> {
    let _transition = ctx.voice.transitions.lock().await;
    if !ctx.voice.set_muted(muted) {
        return None;
    }
    publish_mute(ctx, muted);
    let (muted, deafened) = ctx.voice.controls()?;
    Some(VoiceControls { muted, deafened })
}

pub(crate) async fn set_deafen(ctx: &DaemonContext, deafened: bool) -> Option<VoiceControls> {
    let _transition = ctx.voice.transitions.lock().await;
    if !ctx.voice.set_deafened(deafened) {
        return None;
    }
    let (muted, deafened) = ctx.voice.controls()?;
    Some(VoiceControls { muted, deafened })
}

fn publish_mute(ctx: &DaemonContext, muted: bool) {
    let owner = ctx
        .session
        .read()
        .as_ref()
        .map(|s| s.identity.public_key_hex.clone());
    if let (Some(scope), Some(target_pseudonym)) = (ctx.voice.session(), owner) {
        ctx.publish_event(SubscriptionEvent::Voice(VoiceEvent::MuteChanged {
            scope,
            target_pseudonym,
            muted,
        }));
    }
    ctx.publish_event(SubscriptionEvent::Call(CallEvent::SelfMuteChanged {
        muted,
        timestamp_ms: ctx.clock.now_ms(),
    }));
}
