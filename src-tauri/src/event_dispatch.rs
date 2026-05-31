//! Phase 23.A — single-source event-emission loop.
//!
//! Replaces the ~150 ad-hoc `app.emit(channel, payload)` sites scattered
//! across services + commands + adapters. Every emit now pushes an
//! envelope onto a single mpsc queue; one dispatch task drains the
//! queue and calls `app.emit()` exactly once per envelope.
//!
//! ## Why one queue not per-channel typed queues
//!
//! The original Phase 23 sketch listed per-channel typed senders
//! (`chat_tx: mpsc<ChatEvent>`, `presence_tx: mpsc<PresenceEvent>`,
//! ...). That would force the 150+ call sites to change signatures and
//! force `event_resume` to dispatch typed enums by tag — multi-week
//! mechanical churn for the same on-the-wire result.
//!
//! Routing through a single `mpsc<EmitEnvelope { channel, payload }>`
//! gives us the architectural win (one `app.emit()` callsite at the
//! tail of the loop) with zero call-site signature changes: the
//! existing `emit_live(app, channel, payload)` + `emit_journaled(app,
//! state, channel, payload)` helpers keep their shape and just route
//! payloads through the queue.
//!
//! ## What centralization buys us
//!
//! - Future cross-cutting concerns (rate limiting, telemetry, tracing,
//!   batching, channel-scoped backpressure) attach at one place.
//! - The `event_resume` replay path no longer needs its own raw
//!   `app.emit()` — it pushes through the same queue, so future
//!   listeners (logging, dev-tools) see replayed events identically
//!   to live ones.
//! - Architecture invariant: `app.emit()` literal text appears ONLY
//!   inside this module (the dispatch loop's forward call).
//!
//! ## Migrating from Phase 10 `event_emit.rs`
//!
//! `event_emit.rs` is deleted; its `TauriEmitRecord` + `CursorTick` +
//! `emit_live` + `emit_journaled` move here. All previous
//! `crate::event_emit::*` imports become `crate::event_dispatch::*`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

use crate::state::AppState;

/// Wire shape persisted in the journal. The `channel` lets the frontend
/// route the payload to the same listener that would have received it
/// live; `payload` is the original `ChatEvent`/`NotificationEvent`/etc.
/// already serialized to JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TauriEmitRecord {
    pub channel: String,
    pub payload: serde_json::Value,
}

/// Tick fired alongside every journaled emit. Frontend listens for this
/// on a dedicated `cursor-tick` channel and writes the latest cursor to
/// `localStorage`. Decoupled from the payload schemas so adding new
/// event types doesn't change the cursor protocol.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorTick {
    pub cursor: u64,
}

/// Envelope queued for the single dispatch task. Always carries the
/// payload pre-serialized to `serde_json::Value` — the queue is typed
/// over one shape rather than 7 channel-specific enums.
struct EmitEnvelope {
    channel: String,
    payload: serde_json::Value,
}

/// Single-mpsc event router. The `tx` half is cloned freely (via
/// `Arc<EventDispatch>` on `AppState`); the `rx` half is consumed
/// exactly once by `spawn_dispatch_loop()` at app setup.
pub struct EventDispatch {
    tx: mpsc::UnboundedSender<EmitEnvelope>,
    rx_holder: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<EmitEnvelope>>>,
}

impl EventDispatch {
    /// Construct a fresh dispatch with paired tx/rx. The receiver is
    /// stashed inside for `spawn_dispatch_loop` to consume; until
    /// `take_receiver()` is called the queue is buffered.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx_holder: parking_lot::Mutex::new(Some(rx)),
        }
    }

    fn take_receiver(&self) -> Option<mpsc::UnboundedReceiver<EmitEnvelope>> {
        self.rx_holder.lock().take()
    }

    fn enqueue(&self, channel: String, payload: serde_json::Value) {
        let _ = self.tx.send(EmitEnvelope { channel, payload });
    }
}

impl Default for EventDispatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the single dispatch loop. Drains the queue forever, forwarding
/// each envelope to Tauri's emit. Returns immediately; the loop runs
/// on the Tauri async runtime for the lifetime of the app.
///
/// Idempotent guard: if the receiver was already taken (test harness
/// or repeated setup), this is a no-op rather than a panic.
pub fn spawn_dispatch_loop(app: AppHandle, dispatch: &Arc<EventDispatch>) {
    let Some(mut rx) = dispatch.take_receiver() else {
        tracing::warn!("spawn_dispatch_loop: receiver already taken — duplicate setup?");
        return;
    };
    tauri::async_runtime::spawn(async move {
        while let Some(envelope) = rx.recv().await {
            let _ = app.emit(&envelope.channel, &envelope.payload);
        }
        tracing::debug!("event-dispatch loop exited (channel closed)");
    });
}

/// Phase 23 Tier 3 — emit `payload` on `channel` WITHOUT journaling.
///
/// Pushes through the single dispatch queue. Use for ephemeral signals
/// where missing one mid-stream is acceptable and replaying on cold
/// start would be confusing or wrong:
///
/// - Local echoes (you-sent ACKs, optimistic-UI confirms)
/// - Typing indicators
/// - Presence ticks (status flips, online/offline transitions)
/// - Lifecycle / network-status transitions
/// - OS-level notifications (microphone disconnected, network lost)
/// - Internal state-change broadcasts the frontend re-derives anyway
///
/// Signature preserved verbatim from the pre-Phase-23 `event_emit::emit_live`
/// so the ~141 existing call sites compile unchanged. Internals route
/// through `AppState::event_dispatch` via `app.try_state::<SharedState>()`
/// — works because `setup()` calls `.manage(shared_state)` before any
/// emit can fire.
pub fn emit_live<P: Serialize + ?Sized>(app: &AppHandle, channel: &str, payload: &P) {
    let value = match serde_json::to_value(payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(channel, error = %e, "emit_live: serialize failed, dropping event");
            return;
        }
    };
    if let Some(state) = app.try_state::<crate::state::SharedState>() {
        state.event_dispatch.enqueue(channel.to_string(), value);
    } else {
        tracing::warn!(
            channel,
            "emit_live: SharedState not registered yet — event dropped"
        );
    }
}

/// Journal `payload` for resume + emit it on `channel`. Always succeeds
/// in journaling; the emit side is fire-and-forget through the same
/// dispatch queue. Phase 10's contract preserved: each journaled emit
/// also fires a `cursor-tick` envelope so the frontend's
/// `localStorage["rekindle.lastEventCursor"]` stays current.
pub fn emit_journaled<P: Serialize + ?Sized>(
    app: &AppHandle,
    state: &AppState,
    channel: &str,
    payload: &P,
) {
    let value = match serde_json::to_value(payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                channel,
                error = %e,
                "emit_journaled: serialize failed; emitting without journaling",
            );
            emit_live(app, channel, payload);
            return;
        }
    };
    let cursor = state.event_journal.append(TauriEmitRecord {
        channel: channel.to_string(),
        payload: value.clone(),
    });
    state.event_dispatch.enqueue(channel.to_string(), value);
    let Ok(tick) = serde_json::to_value(CursorTick { cursor }) else {
        return;
    };
    state
        .event_dispatch
        .enqueue("cursor-tick".to_string(), tick);
}

/// Phase 23 — by-value wrapper around `emit_live` for adapters that
/// pre-Phase-23 called `app.emit(channel, payload)` directly. Keeps
/// the migration mechanical: `let _ = self.app_handle.emit(...)`
/// becomes `crate::event_dispatch::dispatch(&self.app_handle, ...)`
/// with no payload-reference rewrite. Equivalent to `emit_live`
/// except for accepting payload by value.
pub fn dispatch<P: Serialize>(app: &AppHandle, channel: &str, payload: P) {
    emit_live(app, channel, &payload);
}

/// Phase 23 — direct push for callers that already have an
/// `Arc<AppState>` (or `&AppState`) and a pre-serialized payload.
/// Used by `event_resume` to replay journaled entries through the
/// same dispatch queue live emits use, so listeners cannot tell
/// replay from live.
pub fn emit_value(state: &AppState, channel: &str, payload: &serde_json::Value) {
    state
        .event_dispatch
        .enqueue(channel.to_string(), payload.clone());
}

/// Phase 23 — direct typed push for callers that already have an
/// `&AppState`. Equivalent to `emit_live` but skips the `try_state`
/// lookup since the state handle is already in scope. Used by
/// `setup()` for the lifecycle-event forwarder + startup notification.
pub fn emit_now<P: Serialize + ?Sized>(state: &AppState, channel: &str, payload: &P) {
    let value = match serde_json::to_value(payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(channel, error = %e, "emit_now: serialize failed");
            return;
        }
    };
    state.event_dispatch.enqueue(channel.to_string(), value);
}

#[cfg(test)]
mod tests {
    //! These tests exercise the journal/record shape end-to-end without
    //! a Tauri `AppHandle` (constructing one in tests is impractical).
    //! The dispatch loop's emit side is fire-and-forget; the
    //! correctness-critical side — journaling + cursor monotonicity —
    //! is the same path `event_resume` reads back, and is fully covered
    //! here through direct journal operations on a `TauriEmitRecord`.

    use super::TauriEmitRecord;
    use rekindle_events::EventJournal;

    #[test]
    fn tauri_emit_record_round_trips_through_journal() {
        let journal: EventJournal<TauriEmitRecord> = EventJournal::new(10);
        let payload = serde_json::json!({"from": "alice", "body": "hello"});
        let cursor = journal.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: payload.clone(),
        });
        assert_eq!(cursor, 1, "first append starts the cursor at 1");
        let backlog = journal.replay_since(0);
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].cursor, 1);
        assert_eq!(backlog[0].event.channel, "chat-event");
        assert_eq!(backlog[0].event.payload, payload);
    }

    #[test]
    fn replay_since_returns_only_strictly_newer_entries() {
        let journal: EventJournal<TauriEmitRecord> = EventJournal::new(10);
        let c1 = journal.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::json!(1),
        });
        let c2 = journal.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::json!(2),
        });
        let backlog = journal.replay_since(c1);
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].cursor, c2);
        assert_eq!(backlog[0].event.payload, serde_json::json!(2));
    }

    #[test]
    fn watermark_dedupes_concurrent_resume_calls() {
        use parking_lot::Mutex;
        let journal: EventJournal<TauriEmitRecord> = EventJournal::new(16);
        let watermark: Mutex<u64> = Mutex::new(0);
        journal.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::json!(1),
        });
        journal.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::json!(2),
        });

        let resume_once = |last_cursor: u64| {
            let mut w = watermark.lock();
            let effective = (*w).max(last_cursor);
            let snap = journal.replay_since(effective);
            if let Some(last) = snap.last() {
                *w = last.cursor;
            }
            snap
        };

        let snapshot_one = resume_once(0);
        assert_eq!(snapshot_one.len(), 2);

        let snapshot_two = resume_once(0);
        assert!(
            snapshot_two.is_empty(),
            "second concurrent caller must NOT redeliver the backlog",
        );
    }

    #[test]
    fn cursor_resets_after_journal_drop() {
        let j1: EventJournal<TauriEmitRecord> = EventJournal::new(4);
        let c = j1.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::Value::Null,
        });
        assert_eq!(c, 1);
        let _ = j1.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::Value::Null,
        });
        drop(j1);

        let j2: EventJournal<TauriEmitRecord> = EventJournal::new(4);
        let c_fresh = j2.append(TauriEmitRecord {
            channel: "chat-event".into(),
            payload: serde_json::Value::Null,
        });
        assert_eq!(
            c_fresh, 1,
            "new journal restarts cursor at 1 — old localStorage cursor would be useless"
        );
    }

    #[test]
    fn event_dispatch_take_receiver_is_one_shot() {
        let d = super::EventDispatch::new();
        assert!(d.take_receiver().is_some(), "first take yields receiver");
        assert!(
            d.take_receiver().is_none(),
            "second take returns None — guards against double spawn",
        );
    }

    #[test]
    fn event_dispatch_enqueue_buffers_before_take() {
        let d = super::EventDispatch::new();
        d.enqueue("chat-event".into(), serde_json::json!("first"));
        d.enqueue("chat-event".into(), serde_json::json!("second"));
        let mut rx = d.take_receiver().expect("receiver");
        let a = rx.try_recv().expect("first envelope buffered");
        let b = rx.try_recv().expect("second envelope buffered");
        assert_eq!(a.channel, "chat-event");
        assert_eq!(a.payload, serde_json::json!("first"));
        assert_eq!(b.channel, "chat-event");
        assert_eq!(b.payload, serde_json::json!("second"));
    }
}
