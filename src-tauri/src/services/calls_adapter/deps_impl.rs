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
use rekindle_protocol::messaging::envelope::MessagePayload;

use super::CallsAdapter;
use crate::channels::{ChatEvent, NotificationEvent};
use crate::state_helpers;

#[async_trait]
impl CallSignalingDeps for CallsAdapter {
    fn owner_key(&self) -> Result<String, CallError> {
        state_helpers::current_owner_key(&self.state).map_err(|_| CallError::IdentityNotLoaded)
    }

    fn identity_secret(&self) -> Result<[u8; 32], CallError> {
        self.state
            .identity_secret
            .lock()
            .as_ref()
            .copied()
            .ok_or(CallError::IdentityNotLoaded)
    }

    fn registry(&self) -> Arc<dyn CallRegistry> {
        Arc::clone(&self.registry)
    }

    fn group_registry(&self) -> Arc<dyn GroupCallRegistry> {
        Arc::clone(&self.group_registry)
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
        self.state
            .friends
            .read()
            .get(peer_pubkey_hex)
            .map(|f| f.display_name.clone())
            .unwrap_or_default()
    }

    async fn send_to_peer(
        &self,
        peer_pubkey_hex: &str,
        payload: MessagePayload,
    ) -> Result<(), CallError> {
        crate::services::message_service::send_to_peer_raw(
            &self.state,
            &self.pool,
            peer_pubkey_hex,
            &payload,
        )
        .await
        .map_err(CallError::Transport)
    }

    async fn start_voice_session(
        &self,
        _call_id: &str,
        peer_pubkey_hex: &str,
        _call_key: [u8; 32],
        _kind: CallKind,
    ) -> Result<(), CallError> {
        // The crate handler updates the registry with the derived
        // `call_key` BEFORE invoking this method, so the AEAD context
        // is already in `state.active_calls`. `start_session` will
        // pick it up via the existing lookup in `init_engine`.
        // 1:1 calls pass `peer_pubkey_hex` as the channel-id argument
        // (the function dual-purposes that parameter — `community_id`
        // is `None` for 1:1).
        crate::services::voice_adapter::start_session(
            peer_pubkey_hex,
            None,
            &self.app_handle,
            &self.state,
        )
        .await
        .map_err(|e| CallError::Session(format!("voice session start: {e}")))
    }

    async fn shutdown_voice_session(&self) {
        crate::services::voice_adapter::shutdown_voice(
            &self.state,
            &rekindle_voice::VoiceShutdownOpts::FULL,
        )
        .await;
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
        let handle = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(remaining.max(1))).await;
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
                let kind_u8 = i64::from(kind.as_u8());
                let expired = i64::try_from(expires_at_ms).unwrap_or(i64::MAX);
                crate::db_helpers::db_fire(
                    &pool,
                    "persist missed call (dialing timeout)",
                    move |conn| {
                        conn.execute(
                            "INSERT OR IGNORE INTO missed_calls \
                             (call_id, owner_key, peer_key, kind, expired_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            rusqlite::params![cid, owner_key, pk, kind_u8, expired],
                        )?;
                        Ok(())
                    },
                );
            }
            crate::event_dispatch::emit_live(
                &app,
                "chat-event",
                &ChatEvent::CallTimedOut { call_id },
            );
        });
        self.state.background_handles.lock().push(handle);
    }

    fn spawn_incoming_call_timeout(
        &self,
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    ) {
        // W13.2 — receiver-side 30 s ring timeout. Clones AppState +
        // AppHandle + DbPool into the spawned task (all Arc-backed +
        // Send), then on fire: check the registry, drop if still
        // Incoming, persist a `missed_calls` row, emit CallMissed.
        // Mirrors the pre-Phase-14 `services::calls::ring_timer::
        // spawn_incoming_timeout` body exactly.
        let state = Arc::clone(&self.state);
        let app = self.app_handle.clone();
        let pool = self.pool.clone();
        let now = rekindle_utils::timestamp_ms();
        let sleep_ms = expires_at_ms.saturating_sub(now);

        let handle = tauri::async_runtime::spawn(async move {
            if sleep_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
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
                let kind_u8 = i64::from(kind.as_u8());
                let expired = i64::try_from(expires_at_ms).unwrap_or(i64::MAX);
                crate::db_helpers::db_fire(
                    &pool,
                    "persist missed call (incoming timeout)",
                    move |conn| {
                        conn.execute(
                            "INSERT OR IGNORE INTO missed_calls \
                             (call_id, owner_key, peer_key, kind, expired_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            rusqlite::params![cid, owner_key, pk, kind_u8, expired],
                        )?;
                        Ok(())
                    },
                );
            }

            crate::event_dispatch::dispatch(
                &app,
                "chat-event",
                ChatEvent::CallMissed {
                    call_id: call_id.clone(),
                    from: peer_pubkey.clone(),
                },
            );
            tracing::info!(call = %call_id, peer = %peer_pubkey,
                "CallMissed — 30s ring with no user accept");
        });
        self.state.background_handles.lock().push(handle);
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
        let kind_u8 = i64::from(kind.as_u8());
        let expired = i64::try_from(expired_at_ms).unwrap_or(i64::MAX);
        crate::db_helpers::db_fire(&self.pool, "persist missed call (adapter)", move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO missed_calls (call_id, owner_key, peer_key, kind, expired_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![cid, owner_key, pk, kind_u8, expired],
            )?;
            Ok(())
        });
    }

    fn surface_window_for_call(&self, _call_id: &str) {
        crate::windows::surface_window_for_call(&self.app_handle);
    }

    fn emit_event(&self, event: CallSignalEvent) {
        let kind_str = |k: CallKind| match k {
            CallKind::Audio => "audio".to_string(),
            CallKind::Video => "video".to_string(),
        };
        let kind_u8_str = |k: u8| match k {
            0 => "audio".to_string(),
            _ => "video".to_string(),
        };

        match event {
            CallSignalEvent::IncomingCall {
                call_id,
                from_public_key,
                from_display_name,
                kind,
                expires_at_ms,
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::IncomingCall {
                        call_id: call_id.clone(),
                        from: from_public_key.clone(),
                        display_name: from_display_name.clone(),
                        kind: kind_str(kind),
                        expires_at_ms,
                    },
                );
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "notification-event",
                    &NotificationEvent::CallIncoming {
                        call_id,
                        from: from_public_key,
                        display_name: from_display_name,
                        kind: kind_str(kind),
                        expires_at_ms,
                        is_group: false,
                    },
                );
            }
            CallSignalEvent::CallRinging { call_id, .. } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "chat-event",
                    ChatEvent::CallRinging { call_id },
                );
            }
            CallSignalEvent::CallConnected {
                call_id,
                peer_public_key,
                kind,
            } => {
                let display_name = self.display_name_with_fallback(&peer_public_key);
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::CallConnected {
                        call_id,
                        kind: kind_str(kind),
                        peer_key: peer_public_key,
                        peer_display_name: display_name,
                        expected_local_camera: matches!(kind, CallKind::Video),
                    },
                );
            }
            CallSignalEvent::CallDeclined { call_id, reason, .. } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "chat-event",
                    ChatEvent::CallDeclined { call_id, reason },
                );
            }
            CallSignalEvent::CallEnded { call_id, reason, .. } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "chat-event",
                    ChatEvent::CallEnded { call_id, reason },
                );
            }
            CallSignalEvent::CallTimedOut { call_id, .. } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "chat-event",
                    ChatEvent::CallTimedOut { call_id },
                );
            }
            CallSignalEvent::CallMissed {
                call_id,
                peer_public_key,
                ..
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::CallMissed {
                        call_id,
                        from: peer_public_key,
                    },
                );
            }
            CallSignalEvent::ConversationFocusRequested {
                peer_public_key,
                peer_display_name,
                reason,
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::ConversationFocusRequested {
                        peer_key: peer_public_key,
                        display_name: peer_display_name,
                        reason,
                    },
                );
            }
            CallSignalEvent::CallStarted {
                call_id,
                peer_public_key,
                peer_display_name,
                kind,
                expires_at_ms,
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::CallStarted {
                        call_id,
                        kind: kind_str(kind),
                        peer_key: peer_public_key,
                        peer_display_name,
                        expires_at_ms,
                    },
                );
            }
            CallSignalEvent::IncomingGroupCall {
                call_id,
                initiator_public_key,
                initiator_display_name,
                participants,
                kind,
                expires_at_ms,
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::IncomingGroupCall {
                        call_id: call_id.clone(),
                        from: initiator_public_key.clone(),
                        display_name: initiator_display_name.clone(),
                        kind: kind_u8_str(kind),
                        participants,
                        expires_at_ms,
                    },
                );
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "notification-event",
                    &NotificationEvent::CallIncoming {
                        call_id,
                        from: initiator_public_key,
                        display_name: initiator_display_name,
                        kind: kind_u8_str(kind),
                        expires_at_ms,
                        is_group: true,
                    },
                );
            }
            CallSignalEvent::GroupCallConnected { call_id } => {
                crate::event_dispatch::dispatch(
                    &self.app_handle,
                    "chat-event",
                    ChatEvent::GroupCallConnected { call_id },
                );
            }
            CallSignalEvent::GroupCallParticipantJoined {
                call_id,
                peer_public_key,
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::GroupCallParticipantJoined {
                        call_id,
                        participant_pubkey: peer_public_key,
                    },
                );
            }
            CallSignalEvent::GroupCallParticipantLeft {
                call_id,
                peer_public_key,
                reason,
            } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::GroupCallParticipantLeft {
                        call_id,
                        participant_pubkey: peer_public_key,
                        reason,
                    },
                );
            }
            CallSignalEvent::GroupCallEnded { call_id, reason } => {
                crate::event_dispatch::dispatch(&self.app_handle, 
                    "chat-event",
                    &ChatEvent::GroupCallEnded { call_id, reason },
                );
            }
        }
    }

    fn register_background_handle(&self, handle: tokio::task::JoinHandle<()>) {
        let wrapped = tauri::async_runtime::spawn(async move {
            let _ = handle.await;
        });
        self.state.background_handles.lock().push(wrapped);
    }
}
