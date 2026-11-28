//! Hand audio to the Host and enqueue its video plane for the shell worker.

use std::sync::Arc;
use std::time::Instant;

use rekindle_transport::subscriptions::lanes::IngressItem;

use crate::state::AppState;

pub(super) fn handle_media_tag(state: &Arc<AppState>, message: &[u8], arrived: Instant) -> bool {
    state.host.current().is_some_and(|host| {
        host.ingest_voice_media(message, arrived, |signed| {
            let is_video =
                rekindle_video::video_payload_channel_from_bytes(&signed.envelope_bytes).is_some();
            state
                .gossip_ingress
                .push(IngressItem::Gossip { signed, is_video });
        })
    })
}
