//! DM-family `app_message` dispatcher.
//!
//! Routes the incoming group DM kinds (`GroupDmInvite` 0x0120,
//! `GroupDmLeave` 0x0123, until E2.6) and DM video fragments (0x0220,
//! until E4.3.8) to the `services::dm` ingest handlers. Lifted out of
//! the main inbound dispatcher so it stays under the workspace line
//! budget. The 1:1 SMPL invite arms went with that path (plan E2.3).

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupDmInvite, GroupDmLeave};

use crate::state::AppState;
use rekindle_db::Db;

pub(crate) async fn handle_dm_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    sender_hex: &str,
    body: DirectBody,
) {
    match body {
        DirectBody::GroupDmInvite(GroupDmInvite {
            record_key,
            slot_seed,
            initiator_pseudonym,
            participants_json,
            wrapped_mek,
            mek_generation,
        }) => {
            if let Err(e) = crate::services::dm::handle_incoming_group_dm_invite(
                app_handle,
                state,
                pool,
                sender_hex,
                &record_key,
                &slot_seed,
                &initiator_pseudonym,
                &participants_json,
                &wrapped_mek,
                mek_generation,
            )
            .await
            {
                tracing::warn!(error = %e, from = %sender_hex, "failed to ingest GroupDmInvite");
            }
        }
        DirectBody::GroupDmLeave(GroupDmLeave { record_key }) => {
            if let Err(e) =
                crate::services::dm::handle_incoming_dm_leave(state, pool, sender_hex, &record_key)
                    .await
            {
                tracing::debug!(error = %e, "GroupDmLeave drop");
            }
        }
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
                // Phase 13 — assembled-frame event now flows through
                // DmDeps::emit_event(DmEvent::VideoFrameAssembled); the
                // adapter handles the base64 + json layout for the
                // `dm-video-frame` Tauri emit.
                let adapter = crate::services::dm_adapter::DmAdapter::new(
                    Arc::clone(state),
                    app_handle.clone(),
                    pool.clone(),
                );
                rekindle_dm::video::dispatch_assembled_frame(&*adapter, sender_hex, frame);
            }
        }
        _ => {
            // Caller restricts bodies via the outer match;
            // unreachable in normal flow. Dropping silently for safety.
            tracing::debug!("handle_dm_payload: non-DM body reached helper");
        }
    }
}
