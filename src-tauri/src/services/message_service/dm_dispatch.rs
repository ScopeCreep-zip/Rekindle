//! DM-family `app_message` payload dispatcher.
//!
//! Routes incoming `MessagePayload::Dm*` / `GroupDm*` variants (invite,
//! accept, decline, leave, group invite, video fragment) to the
//! `services::dm` ingest handlers. Lifted out of the main inbound
//! dispatcher so it stays under the workspace line budget.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::state::AppState;

pub(super) async fn handle_dm_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    payload: MessagePayload,
) {
    match payload {
        MessagePayload::DmInvite {
            record_key,
            slot_seed,
            alice_pseudonym,
            alice_subkey,
            bob_subkey,
        } => {
            if let Err(e) = crate::services::dm::handle_incoming_dm_invite(
                app_handle,
                state,
                pool,
                sender_hex,
                &record_key,
                &slot_seed,
                &alice_pseudonym,
                alice_subkey,
                bob_subkey,
            )
            .await
            {
                tracing::warn!(error = %e, from = %sender_hex, "failed to ingest DmInvite");
            }
        }
        MessagePayload::DmAccept { record_key: _ } => {
            // DmAccept is the reply to a DmInvite app_call; if it
            // arrives via app_message instead (peer used the wrong
            // path) we just log and ignore — Alice's outbound
            // app_call already resolved with the inline reply.
            tracing::trace!("received DmAccept via app_message; ignoring");
        }
        MessagePayload::DmDecline {
            record_key,
            reason: _,
        } => {
            if let Err(e) =
                crate::services::dm::handle_incoming_dm_decline(state, pool, &record_key).await
            {
                tracing::debug!(error = %e, "DmDecline drop");
            }
        }
        MessagePayload::GroupDmInvite {
            record_key,
            slot_seed,
            initiator_pseudonym,
            participants_json,
            wrapped_mek,
            mek_generation,
        } => {
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
        MessagePayload::DmLeave { record_key } => {
            if let Err(e) =
                crate::services::dm::handle_incoming_dm_leave(state, pool, sender_hex, &record_key)
                    .await
            {
                tracing::debug!(error = %e, "DmLeave drop");
            }
        }
        MessagePayload::DmVideoFragment {
            stream_id,
            frame_seq,
            fragment_index,
            fragment_count,
            keyframe,
            codec,
            timestamp,
            chunk,
        } => {
            // W11.4 — accumulate fragments and emit a `videoFrame`
            // event when the last chunk lands. The Signal layer has
            // already verified authenticity (sender's identity key is
            // bound to the envelope), so we don't repeat per-fragment
            // signatures the way community video does.
            //
            // Reader-validates: the codec tag is peer-controlled wire
            // data. The community path can't even decode an unknown
            // codec (closed capnp enum); mirror that gate here so an
            // arbitrary string never reaches the frontend's decoder
            // construction.
            if rekindle_types::video::Codec::from_wire_str(&codec).is_none() {
                tracing::debug!(peer = %sender_hex, %codec,
                    "DmVideoFragment with unknown codec tag; dropping");
                return;
            }
            if let Some(frame) =
                state
                    .dm_video_reassembly
                    .record_fragment(rekindle_dm::DmVideoFragment {
                        peer_pubkey: sender_hex,
                        stream_id,
                        frame_seq,
                        fragment_index,
                        fragment_count,
                        keyframe,
                        codec,
                        timestamp,
                        chunk,
                    })
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
            // Caller restricts payloads via the outer match;
            // unreachable in normal flow. Dropping silently for safety.
            tracing::debug!("handle_dm_payload: non-DM payload reached helper");
        }
    }
}
