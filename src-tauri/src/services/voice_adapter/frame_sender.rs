//! Veilid-backed [`VoiceFrameSender`] for the desktop voice path.
//!
//! This is the sanctioned adapter layer — `src-tauri` may use
//! `veilid-core`, so the RoutingContext / RouteId / `app_message`
//! plumbing that used to live inside `rekindle-voice::transport` lives
//! here instead, keeping the voice crate free of `veilid-core`
//! (Invariant 2). Imported routes are cached by route blob so repeated
//! frames to the same peer don't leak a fresh `RouteId` each send.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_route::contexts::{RouteContextKind, RouteContextSpec};
use rekindle_voice::{VoiceError, VoiceFrameSender};
use veilid_core::{RouteId, RoutingContext, SafetySelection, Sequencing, Target, VeilidAPI};

pub struct VeilidVoiceFrameSender {
    api: VeilidAPI,
    /// route blob → (voice routing context, imported route id). Cached
    /// so each peer's route is imported once, not per frame.
    routes: Mutex<HashMap<Vec<u8>, (RoutingContext, RouteId)>>,
}

impl VeilidVoiceFrameSender {
    pub fn new(api: VeilidAPI) -> Self {
        Self {
            api,
            routes: Mutex::new(HashMap::new()),
        }
    }

    /// Build the low-latency voice routing context
    /// (`SafetySelection::Unsafe` — trade sender privacy for latency).
    fn build_voice_routing_context(api: &VeilidAPI) -> Result<RoutingContext, VoiceError> {
        let spec = RouteContextSpec::rc_voice();
        api.routing_context()
            .map_err(|e| VoiceError::Transport(format!("routing context: {e}")))?
            .with_safety(match spec.kind {
                RouteContextKind::Voice => SafetySelection::Unsafe(Sequencing::NoPreference),
                RouteContextKind::Safe => SafetySelection::Unsafe(Sequencing::PreferOrdered),
            })
            .map_err(|e| VoiceError::Transport(format!("with_safety: {e}")))
    }

    /// Resolve (importing + caching on first use) the routing context +
    /// route id for `route_blob`.
    fn resolve_route(&self, route_blob: &[u8]) -> Result<(RoutingContext, RouteId), VoiceError> {
        if let Some(entry) = self.routes.lock().get(route_blob) {
            return Ok((entry.0.clone(), entry.1.clone()));
        }
        let routing_context = Self::build_voice_routing_context(&self.api)?;
        let route_id = self
            .api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| VoiceError::Transport(format!("import route: {e}")))?;
        let mut routes = self.routes.lock();
        let entry = routes
            .entry(route_blob.to_vec())
            .or_insert((routing_context, route_id));
        Ok((entry.0.clone(), entry.1.clone()))
    }
}

#[async_trait]
impl VoiceFrameSender for VeilidVoiceFrameSender {
    async fn send_voice_frame(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), VoiceError> {
        // Resolve + clone out of the lock before the await (parking_lot
        // guard is !Send and must not cross `.await`).
        let (routing_context, route_id) = self.resolve_route(route_blob)?;
        routing_context
            .app_message(Target::RouteId(route_id), data)
            .await
            .map_err(|e| VoiceError::Transport(format!("app_message: {e}")))
    }
}
