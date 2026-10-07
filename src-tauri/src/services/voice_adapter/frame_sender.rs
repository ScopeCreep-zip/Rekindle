//! Veilid-backed [`VoiceFrameSender`] for the desktop voice path.
//!
//! This is the sanctioned adapter layer — `src-tauri` may use
//! `veilid-core`, so the RoutingContext / RouteId / `app_message`
//! plumbing that used to live inside `rekindle-voice::transport` lives
//! here instead, keeping the voice crate free of `veilid-core`
//! (Invariant 2). Routes are resolved through the process's one importer
//! (`RouteImports`, plan C7.6). This sender used to keep its own map and
//! release imported routes, and a release removes the **shared** import
//! for every sender in the process, so a voice heal dropped the chat route
//! to the same peer.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_protocol::dht::route_imports::RouteImports;
use rekindle_voice::{VoiceError, VoiceFrameSender};
use veilid_core::{RoutingContext, Target, VeilidAPI, VeilidAPIError};

pub struct VeilidVoiceFrameSender {
    routing_context: RoutingContext,
    route_imports: Arc<RouteImports>,
}

impl VeilidVoiceFrameSender {
    /// # Errors
    /// The voice routing context could not be built.
    pub fn new(api: &VeilidAPI, route_imports: Arc<RouteImports>) -> Result<Self, VoiceError> {
        Ok(Self {
            routing_context: Self::build_voice_routing_context(api)?,
            route_imports,
        })
    }

    /// Build the voice routing context: a 3-hop Tor-class Safe route
    /// (sender hidden behind an ephemeral route id) with `LowLatency`
    /// stability — the lowest-latency variant that is still anonymous.
    /// Voice never uses `SafetySelection::Unsafe`: a vulnerable user's
    /// real node identity must not be exposed to a relay just to shave
    /// latency, and Unsafe routing is gated behind veilid-core's
    /// `footgun-nodeid-target` feature, which we never enable.
    fn build_voice_routing_context(api: &VeilidAPI) -> Result<RoutingContext, VoiceError> {
        api.routing_context()
            .map_err(|e| VoiceError::Transport(format!("routing context: {e}")))?
            .with_safety(rekindle_protocol::dht::pool::safety_selection(
                &rekindle_types::config::SafetyProfile::default_voice(),
            ))
            .map_err(|e| VoiceError::Transport(format!("with_safety: {e}")))
    }
}

#[async_trait]
impl VoiceFrameSender for VeilidVoiceFrameSender {
    async fn send_voice_frame(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), VoiceError> {
        let route_id = self
            .route_imports
            .get_or_import(route_blob)
            .map_err(|e| VoiceError::Transport(format!("import route: {e}")))?;
        match self
            .routing_context
            .app_message(Target::RouteId(route_id.clone()), data)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                // `NoConnection` ("could not get remote private route")
                // or `InvalidTarget`: the route is unusable. The importer
                // forgets it, so the next frame re-imports
                // fresh (Piece 4 mechanism A). Matched on the veilid
                // variant — this is the sanctioned adapter layer.
                if matches!(
                    e,
                    VeilidAPIError::NoConnection { .. } | VeilidAPIError::InvalidTarget { .. }
                ) {
                    self.route_imports.invalidate_after_send_failure(&route_id);
                }
                Err(VoiceError::Transport(format!("app_message: {e}")))
            }
        }
    }
}
