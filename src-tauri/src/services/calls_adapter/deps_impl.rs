//! Phase 14.r split — `impl CallSignalingDeps for CallsAdapter`.
//!
//! All call-signaling trait surface in one place. Each method either
//! reads/mutates the live AppState directly (under parking_lot), or
//! delegates to the existing src-tauri voice/message-service helpers.
//! The crate's 1:1 + group signaling handlers consume this impl
//! through `Arc<dyn CallSignalingDeps>`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_calls::signaling::{
    CallRegistry, CallSignalEvent, CallSignalingDeps, GroupCallRegistry,
};
use rekindle_calls::state::CallKind;
use rekindle_calls::CallError;
use rekindle_codec::capnp_codec::direct::DirectBody;
use rekindle_types::group_dm::GroupCallRoster;

use super::CallsAdapter;
use crate::state_helpers;

#[async_trait]
impl CallSignalingDeps for CallsAdapter {
    fn owner_key(&self) -> Result<String, CallError> {
        state_helpers::current_owner_key(&self.state).map_err(|_| CallError::IdentityNotLoaded)
    }

    fn identity_secret(&self) -> Result<[u8; 32], CallError> {
        state_helpers::identity_secret_bytes(&self.state).ok_or(CallError::IdentityNotLoaded)
    }

    fn registry(&self) -> Arc<dyn CallRegistry> {
        Arc::clone(&self.registry)
    }

    fn group_registry(&self) -> Arc<dyn GroupCallRegistry> {
        Arc::clone(&self.group_registry)
    }

    async fn group_roster(&self, group_id: &str) -> Result<GroupCallRoster, CallError> {
        let owner = self.owner_key()?;
        let id = group_id.to_string();
        let now_ms = rekindle_utils::timestamp_ms_i64();
        self.pool
            .call(move |conn| rekindle_db::repo::group_dms::call_roster(conn, &owner, &id, now_ms))
            .await
            .map_err(|e| CallError::InvalidState(format!("the group cannot be read: {e}")))
    }

    fn is_peer_temp_muted(&self, peer_pubkey_hex: &str) -> bool {
        let now_ms = rekindle_utils::timestamp_ms();
        let guard = self.state.temp_call_muted.lock();
        guard
            .get(peer_pubkey_hex)
            .is_some_and(|&until| until > now_ms)
    }

    fn friend_display_name(&self, peer_pubkey_hex: &str) -> String {
        // Return raw display_name (or "" if friend not found / empty).
        // The crate's handlers check `is_empty()` and apply their own
        // fallback using `initiator_pubkey` (which may differ from
        // `peer_pubkey_hex` in edge cases). Doing the fallback inside
        // the adapter would short-circuit that logic.
        crate::state_helpers::friend_display_name(&self.state, peer_pubkey_hex).unwrap_or_default()
    }

    fn local_video_decode_codecs(&self) -> Vec<String> {
        // Phase 5 — the WebView's probed decode set, as wire strings.
        // Before the probe runs, the conservative interim default
        // (VP9-only) applies — the same floor the community
        // capability broadcast uses (`voice_adapter::io_helpers`).
        crate::services::community::video_session::reported_local_caps(&self.state)
            .unwrap_or_else(rekindle_video::MediaCapabilities::interim_default)
            .decode_codecs
            .into_iter()
            .map(|c| c.wire_str().to_string())
            .collect()
    }

    async fn send_to_peer(&self, peer_pubkey_hex: &str, body: DirectBody) -> Result<(), CallError> {
        let host = crate::state_helpers::friend_host(&self.state).map_err(CallError::Transport)?;
        rekindle_transport::operations::dm::send_to_peer(host.as_ref(), peer_pubkey_hex, &body)
            .await
            .map_err(|e| CallError::Transport(e.to_string()))
    }

    async fn commit_call_end(
        &self,
        peer_pubkey_hex: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError> {
        let host = crate::state_helpers::friend_host(&self.state).map_err(CallError::Transport)?;
        let deadline = i64::try_from(deadline_ms).unwrap_or(i64::MAX);
        rekindle_transport::operations::dm::handshake::enqueue_until(
            host.as_ref(),
            peer_pubkey_hex,
            &body,
            deadline,
        )
        .await
        .map(drop)
        .map_err(|e| CallError::Transport(e.to_string()))
    }

    async fn commit_call_invite(
        &self,
        peer_pubkey_hex: &str,
        call_id: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError> {
        let host = crate::state_helpers::friend_host(&self.state).map_err(CallError::Transport)?;
        let deadline = i64::try_from(deadline_ms).unwrap_or(i64::MAX);
        rekindle_transport::operations::dm::call_invite::commit(
            host.as_ref(),
            peer_pubkey_hex,
            call_id,
            &body,
            deadline,
        )
        .await
        .map_err(|e| CallError::Transport(e.to_string()))
    }

    async fn settle_call_invite(&self, call_id: &str, answered: bool) {
        super::call_media::settle_invite(&self.state, call_id, answered).await;
    }

    fn now_ms(&self) -> u64 {
        self.state.clock.now_ms()
    }

    fn last_inbound_media_ms(&self, call_id: &str) -> Option<u64> {
        if !self.state.call_media.is(call_id) {
            return None;
        }
        let engine = self.state.voice_engine.lock();
        engine.as_ref().and_then(|h| h.media_liveness.latest())
    }

    fn spawn_consent_watch(&self, call_id: String, since_ms: u64) {
        // Plan CE1 (b): RFC 7675 consent, checked against the clock port
        // each time the 30 s window from the last proof of life runs out.
        let watcher = super::CallsAdapter::new(
            Arc::clone(&self.state),
            self.app_handle.clone(),
            self.pool.clone(),
        );
        state_helpers::spawn_in_login_with_token(&self.state, "call consent", |stop| async move {
            loop {
                let check =
                    rekindle_calls::signaling::check_consent(watcher.as_ref(), &call_id, since_ms)
                        .await;
                let rekindle_calls::signaling::ConsentCheck::Live { recheck_in_ms } = check else {
                    return;
                };
                let wait = tokio::time::sleep(std::time::Duration::from_millis(recheck_in_ms));
                if stop.run_until_cancelled(wait).await.is_none() {
                    return;
                }
            }
        });
    }

    async fn start_voice_session(
        &self,
        call_id: &str,
        peer_pubkey_hex: &str,
        _kind: CallKind,
    ) -> Result<(), CallError> {
        super::call_media::start(&self.state, &self.app_handle, call_id, peer_pubkey_hex).await
    }

    async fn shutdown_voice_session(&self, call_id: &str) {
        super::call_media::shutdown(&self.state, call_id).await;
    }

    fn voice_active(&self) -> bool {
        self.state.voice_engine.lock().is_some()
    }

    fn pre_stage_voice_channel(&self) {
        // W14.1: drop any stale staged receiver first so a previous
        // aborted accept doesn't leak a dangling rx; create a fresh
        // bounded mpsc and install both ends on AppState.
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.state.voice_packet_tx.write() = Some(tx);
        *self.state.voice_packet_rx_staged.lock() = Some(rx);
        tracing::info!("W14.1 — pre-staged voice receive channel on CallAccept arrival");
    }

    fn spawn_dialing_call_timeout(
        &self,
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    ) {
        // W13.2 caller-side — sleep until `expires_at_ms`. If still
        // Outgoing, drop the registry entry, log the missed_calls
        // row, emit CallTimedOut. Relocated from the deleted
        // `services::calls::ring_timer::spawn_dialing_timeout`.
        let task_state = Arc::clone(&self.state);
        let app = self.app_handle.clone();
        let pool = self.pool.clone();
        let now = rekindle_utils::timestamp_ms();
        let remaining = expires_at_ms.saturating_sub(now);
        let ring = std::time::Duration::from_millis(remaining.max(1));
        state_helpers::spawn_in_login_with_token(
            &self.state,
            "call ring timeout (outgoing)",
            |stop| async move {
                if stop
                    .run_until_cancelled(tokio::time::sleep(ring))
                    .await
                    .is_none()
                {
                    return;
                }
                let still_dialing = task_state
                    .active_calls
                    .get(&call_id)
                    .is_some_and(|c| matches!(c.status, rekindle_calls::CallStatus::Outgoing));
                if !still_dialing {
                    return;
                }
                task_state.active_calls.remove(&call_id);
                // Persist missed_calls row (relocated from deleted
                // `services::calls::mod::persist_missed_call`).
                if let Ok(owner_key) = state_helpers::current_owner_key(&task_state) {
                    let cid = call_id.clone();
                    let pk = peer_pubkey.clone();
                    crate::db_helpers::db_fire(
                        &pool,
                        "persist missed call (dialing timeout)",
                        move |conn| {
                            rekindle_db::repo::calls::record_missed(
                                conn,
                                &owner_key,
                                &cid,
                                &pk,
                                kind.as_u8(),
                                expires_at_ms,
                            )?;
                            Ok(())
                        },
                    );
                }
                crate::event_dispatch::emit_call(
                    &app,
                    rekindle_types::subscription_events::CallEvent::TimedOut { call_id },
                );
            },
        );
    }

    fn spawn_incoming_call_timeout(
        &self,
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    ) {
        // W13.2 — receiver-side 30 s ring timeout. Clones AppState +
        // AppHandle + Db into the spawned task (all Arc-backed +
        // Send), then on fire: check the registry, drop if still
        // Incoming, persist a `missed_calls` row, emit CallMissed.
        // Mirrors the pre-Phase-14 `services::calls::ring_timer::
        // spawn_incoming_timeout` body exactly.
        let state = Arc::clone(&self.state);
        let app = self.app_handle.clone();
        let pool = self.pool.clone();
        let now = rekindle_utils::timestamp_ms();
        let sleep_ms = expires_at_ms.saturating_sub(now);

        let ring = std::time::Duration::from_millis(sleep_ms);
        state_helpers::spawn_in_login_with_token(
            &self.state,
            "call ring timeout (incoming)",
            |stop| async move {
                if stop
                    .run_until_cancelled(tokio::time::sleep(ring))
                    .await
                    .is_none()
                {
                    return;
                }
                let still_incoming = state
                    .active_calls
                    .get(&call_id)
                    .is_some_and(|c| matches!(c.status, rekindle_calls::CallStatus::Incoming));
                if !still_incoming {
                    return;
                }
                state.active_calls.remove(&call_id);

                // Persist missed_calls row (best-effort).
                if let Ok(owner_key) = state_helpers::current_owner_key(&state) {
                    let cid = call_id.clone();
                    let pk = peer_pubkey.clone();
                    crate::db_helpers::db_fire(
                        &pool,
                        "persist missed call (incoming timeout)",
                        move |conn| {
                            rekindle_db::repo::calls::record_missed(
                                conn,
                                &owner_key,
                                &cid,
                                &pk,
                                kind.as_u8(),
                                expires_at_ms,
                            )?;
                            Ok(())
                        },
                    );
                }

                crate::event_dispatch::emit_call(
                    &app,
                    rekindle_types::subscription_events::CallEvent::Missed {
                        call_id: call_id.clone(),
                        from: peer_pubkey.clone(),
                    },
                );
                tracing::info!(call = %call_id, peer = %peer_pubkey,
                "CallMissed — 30s ring with no user accept");
            },
        );
    }

    fn persist_missed_call(
        &self,
        call_id: &str,
        peer_pubkey_hex: &str,
        kind: CallKind,
        expired_at_ms: u64,
    ) {
        let Ok(owner_key) = state_helpers::current_owner_key(&self.state) else {
            return;
        };
        let cid = call_id.to_string();
        let pk = peer_pubkey_hex.to_string();
        let kind = kind.as_u8();
        crate::db_helpers::db_fire(&self.pool, "persist missed call (adapter)", move |conn| {
            rekindle_db::repo::calls::record_missed(
                conn,
                &owner_key,
                &cid,
                &pk,
                kind,
                expired_at_ms,
            )?;
            Ok(())
        });
    }

    fn surface_window_for_call(&self, _call_id: &str) {
        crate::windows::surface_buddy_list(&self.app_handle);
    }

    fn present_active_call(&self, call_id: &str) {
        if let Err(e) = crate::windows::open_call_window(&self.app_handle, call_id) {
            tracing::warn!(error = %e, "could not open the call window");
        }
    }

    fn emit_event(&self, event: CallSignalEvent) {
        self.emit_signal_event(event);
    }
}
