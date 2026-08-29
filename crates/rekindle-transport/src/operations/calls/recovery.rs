//! W16.8 — crash recovery: rehydrate persisted Outgoing/Incoming call
//! state from [`EnvelopeStore`](crate::envelope_store::EnvelopeStore) on
//! startup.

use rekindle_calls::{CallKind, CallStatus};
use rekindle_types::notification::TransportNotification;
use rekindle_utils::timestamp_ms;
use tracing::{debug, warn};

use super::util::{kind_str, parse_kind, parse_status};
use super::{CallRuntime, TimerKind};
use crate::envelope_store::PersistedCallState;

impl CallRuntime {
    /// W16.8 — crash recovery. Rehydrates persisted Outgoing/Incoming
    /// call state from [`EnvelopeStore`] and either:
    /// - **Resumes** the call if the ring window is still open (re-emit
    ///   `CallStarted`/`IncomingCall` notification, spawn a fresh
    ///   ring timer for the remaining window).
    /// - **Drops** the row as a missed call if the ring already
    ///   expired (emit `CallTimedOut`/`CallMissed`, delete the row).
    ///
    /// Active call state is intentionally NOT persisted (matches Signal
    /// + Discord — voice transport state is process-bound), so this
    /// method only restores Outgoing/Incoming. After rehydration runs
    /// a single immediate envelope-queue retry tick so any in-flight
    /// CallInvite/CallAccept envelopes that were due during the
    /// downtime fire fast on launch.
    ///
    /// Call once at app startup, after `TransportNode::start` and
    /// before accepting new user actions.
    pub async fn recover(&self) {
        let states = match self
            .inner
            .store
            .load_active_calls(&self.inner.owner_key)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "recover_active_calls: load_active_calls failed");
                return;
            }
        };
        debug!(count = states.len(), "CallRuntime::recover: rehydrating");

        let now = timestamp_ms();
        for persisted in states {
            self.recover_one(persisted, now).await;
        }

        // Run an immediate retry tick so any envelopes whose
        // `next_retry_at_ms` already passed during the downtime fire
        // now instead of waiting for the next tick.
        self.inner.queue.run_retry_tick().await;
    }

    async fn recover_one(&self, persisted: PersistedCallState, now_ms: u64) {
        let call_id = persisted.call_id.clone();
        let kind = parse_kind(&persisted.kind).unwrap_or(CallKind::Audio);
        let Some(status) = parse_status(&persisted.status) else {
            warn!(
                call_id,
                status = %persisted.status,
                "recover: unknown persisted status, deleting"
            );
            let _ = self
                .inner
                .store
                .delete_active_call(&self.inner.owner_key, &call_id)
                .await;
            return;
        };

        // Only Outgoing/Incoming are valid persisted statuses (W16.8
        // never persists Connecting/Active/Missed).
        if !matches!(status, CallStatus::Outgoing | CallStatus::Incoming) {
            warn!(
                call_id,
                ?status,
                "recover: unexpected persisted status, deleting"
            );
            let _ = self
                .inner
                .store
                .delete_active_call(&self.inner.owner_key, &call_id)
                .await;
            return;
        }

        // Expired during downtime — emit timeout/missed, delete.
        if persisted.expires_at_ms <= now_ms {
            let _ = self
                .inner
                .store
                .delete_active_call(&self.inner.owner_key, &call_id)
                .await;
            match status {
                CallStatus::Outgoing => {
                    self.inner
                        .notifications
                        .notify(&TransportNotification::CallTimedOut {
                            call_id: call_id.clone(),
                        });
                }
                CallStatus::Incoming => {
                    self.inner
                        .notifications
                        .notify(&TransportNotification::CallMissed {
                            call_id: call_id.clone(),
                            from: persisted.peer_pubkey.clone(),
                        });
                }
                _ => {}
            }
            return;
        }

        // Still within the ring window — rehydrate and resume.
        let my_secret = persisted
            .my_x25519_secret
            .as_deref()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .map(x25519_dalek::StaticSecret::from);
        let peer_pub = persisted
            .peer_x25519_pub
            .as_deref()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());

        let state = rekindle_calls::CallState {
            call_id: call_id.clone(),
            peer_pubkey: persisted.peer_pubkey.clone(),
            kind,
            status,
            expires_at_ms: persisted.expires_at_ms,
            my_x25519_secret: my_secret,
            peer_x25519_pub: peer_pub,
            // call_key is not persisted (held only in-memory once
            // derived); a recovered Outgoing/Incoming call hasn't
            // accepted yet, so call_key is None either way.
            call_key: None,
            // Peer caps aren't persisted either — a recovered pre-accept
            // call re-learns them from the CallAccept; senders treat
            // empty as the VP9 floor.
            peer_video_decode_codecs: Vec::new(),
        };
        self.inner.state_machine.lock().rehydrate(state);

        // Spawn the matching ring timer for the remaining window.
        match status {
            CallStatus::Outgoing => {
                self.spawn_timeout(call_id.clone(), persisted.expires_at_ms, TimerKind::Dialing);
                // Re-emit CallStarted so the UI re-mounts the
                // OutgoingCallPanel. Display name is unknown
                // post-restart (the friend-list resolver runs
                // shell-side; the runtime can't look it up here).
                self.inner
                    .notifications
                    .notify(&TransportNotification::CallStarted {
                        call_id,
                        kind: kind_str(kind).into(),
                        peer_key: persisted.peer_pubkey,
                        peer_display_name: String::new(),
                        expires_at_ms: persisted.expires_at_ms,
                        started_at_ms: persisted.inserted_at_ms,
                        status: "calling".into(),
                    });
            }
            CallStatus::Incoming => {
                self.spawn_timeout(
                    call_id.clone(),
                    persisted.expires_at_ms,
                    TimerKind::Incoming,
                );
                self.inner
                    .notifications
                    .notify(&TransportNotification::IncomingCall {
                        call_id,
                        kind: kind_str(kind).into(),
                        from: persisted.peer_pubkey,
                        display_name: String::new(),
                        expires_at_ms: persisted.expires_at_ms,
                        received_at_ms: persisted.inserted_at_ms,
                        is_group: false,
                    });
            }
            _ => {}
        }
    }
}
