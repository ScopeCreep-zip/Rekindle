//! DM-family `app_message` dispatcher.
//!
//! Routes DM video fragments (0x0220, until E4.3.8) to the `services::dm`
//! handlers. The group-DM kinds never reach it: their sender-key bodies
//! (plan E2.6) are `rekindle-dm`'s, applied from slice G2, and the
//! transport drops them until then. Lifted out of
//! the main inbound dispatcher so it stays under the workspace line
//! budget. The 1:1 SMPL invite arms went with that path (plan E2.3).

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::DirectBody;

use crate::state::AppState;

pub(crate) fn handle_dm_payload(state: &Arc<AppState>, sender_hex: &str, body: DirectBody) {
    match body {
        DirectBody::DmVideoFragment(fragment) => {
            // W11.4 — accumulate fragments and emit a `videoFrame`
            // event when the last chunk lands. The ratchet has already
            // authenticated the sender (its AD binds both identities),
            // so we don't repeat per-fragment signatures the way
            // community video does.
            //
            // Reader-validates: the codec tag is peer-controlled wire
            // data. The community path can't even decode an unknown
            // codec (closed capnp enum); mirror that gate here so an
            // arbitrary string never reaches the frontend's decoder
            // construction.
            if rekindle_types::video::Codec::from_wire_str(&fragment.codec).is_none() {
                tracing::debug!(peer = %sender_hex, codec = %fragment.codec,
                    "DmVideoFragment with unknown codec tag; dropping");
                return;
            }
            if let Some(frame) = state
                .dm_video_reassembly
                .record_fragment(sender_hex, fragment)
            {
                crate::services::dm::video::deliver_assembled_frame(state, sender_hex, frame);
            }
        }
        _ => {
            // Caller restricts bodies via the outer match;
            // unreachable in normal flow. Dropping silently for safety.
            tracing::debug!("handle_dm_payload: non-DM body reached helper");
        }
    }
}
