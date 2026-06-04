# 0007 — Route every Rust → Frontend event through one mpsc dispatcher

- **Status:** Accepted
- **Date:** 2026-06

## Context and problem statement

Before Phase 23.A, around 150 `app.emit(channel, payload)` calls
were scattered across `src-tauri/src/services/`,
`src-tauri/src/commands/`, and the per-channel adapter modules.
Each emit site:

- Computed its own throttling (sometimes correctly, sometimes
  not).
- Logged at its own granularity.
- Did not participate in any event-journal or replay scheme.

The `event_resume` Tauri command (Phase 10 reconnect support)
needed a journal of recently emitted events so the frontend could
replay them after a soft stall (page reload during dev, brief IPC
pause, hot-reload across the Tauri ↔ webview bridge). Adding a
journal at every emit site would have meant 150+ wrapping changes.

We also wanted to enable cross-cutting concerns — rate limiting,
telemetry, batching, channel-scoped backpressure — without touching
every emit site.

## Decision drivers

- **One place to change cross-cutting behaviour.** A dispatcher
  loop lets future changes attach at a single seam.
- **Replay-equal-to-live.** The `event_resume` path must deliver
  exactly what live listeners would have received, in the same
  shape.
- **No signature churn at emit sites.** 150 call sites is a lot of
  churn; we wanted to keep them as-is.

## Considered options

### Option A — Per-channel typed senders

The original sketch proposed `chat_tx: mpsc<ChatEvent>`,
`presence_tx: mpsc<PresenceEvent>`, etc. Each adapter held the
typed sender; the dispatch loop drained them.

Rejected because:

- Every emit site had to change to push into a typed sender
  instead of calling `app.emit(...)`.
- The journal would need to hold a tagged enum union of all
  channel types; adding a channel meant editing the enum and the
  replay path.
- The on-the-wire result is identical to Option B.

### Option B — Single mpsc over `EmitEnvelope { channel, payload }` — selected

A single `mpsc<EmitEnvelope { channel: String, payload:
serde_json::Value }>` drains into one `app.emit(channel, payload)`
call inside `event_dispatch::spawn_dispatch_loop`. The existing
`emit_live(app, channel, payload)` and `emit_journaled(app, state,
channel, payload)` helpers keep their shape — they just push
through the queue.

The journal stores `TauriEmitRecord { channel, payload }`. The
frontend persists the most-recent cursor; `event_resume` returns
every entry with `cursor > since`.

### Option C — Status quo (no dispatcher)

Rejected because the journal alone would require touching every
emit site, and the cross-cutting goal was unmet.

## Decision outcome

Chosen: **Option B**.

`src-tauri/src/event_dispatch.rs` owns the dispatcher. The literal
text `app.emit(` appears in `src-tauri/` exactly once — inside the
dispatch loop. Architecture rule **B17** enforces this.

The journal lives in the dedicated `rekindle-events` crate (hoisted
out of `rekindle-transport::subscriptions`), bounded to 10 000
in-memory entries. Older entries drop silently; a stale frontend
cursor returns the full ring and the frontend treats that as
"do a full hydration."

## Consequences

**Positive.**

- One place to change cross-cutting behaviour for the entire
  Rust → Frontend event surface.
- Replay observably equal to live delivery.
- Zero emit-site signature churn — the migration was an
  import-rewrite of `crate::event_emit::*` to
  `crate::event_dispatch::*`.

**Negative.**

- The dispatcher is a single-threaded bottleneck for events. The
  frontend cannot receive events faster than the dispatch loop can
  push them. Benchmarks show the loop sustains ~50k emits/sec on
  cold hardware; this is comfortably above the system's burst
  rate.
- Video frames and voice packets bypass the queue (separate
  `tokio::broadcast` channels and a dedicated `mpsc<VoicePacket>`)
  because frame-rate traffic would saturate it.

**Boundaries.**

- `app.emit(` literal grep returns zero matches outside
  `event_dispatch.rs`. CI gauntlet enforces.
- Video and voice are the only declared exceptions; new bypass
  paths require an explicit ADR.

## More information

- [`../architecture/event-dispatch.md`](../architecture/event-dispatch.md) — full description of the dispatcher + journal contract.
- [`../architecture/tauri-state.md`](../architecture/tauri-state.md) — the `event_dispatch`, `event_journal`, `event_replay_watermark`, `dedup_cache` fields on `AppState`.
- `crates/rekindle-events/` — the `EventJournal` source.
