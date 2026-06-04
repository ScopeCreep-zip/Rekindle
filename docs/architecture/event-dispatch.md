# Event Dispatch and Journal

The desktop app's Rust → Frontend event surface is a single mpsc
queue. Every `app.emit()` call in `src-tauri/` goes through one
dispatch loop in `src-tauri/src/event_dispatch.rs` (Phase 23.A). The
same module holds the cursor-based replay journal that lets the
frontend recover from soft stalls without losing events.

This is the architectural contract for events between Rust and the
SolidJS frontend. It cross-references
[`tauri-backend.md`](tauri-backend.md) (IPC patterns),
[`tauri-state.md`](tauri-state.md) (the `event_dispatch`,
`event_journal`, `event_replay_watermark` fields), and
[`sync.md`](sync.md) (cross-device sync uses the same journal for the
multi-device read-state stream).

## Why a single queue

Before Phase 23.A, ~150 ad-hoc `app.emit(channel, payload)` sites
were scattered across services, commands, and adapters. The original
Phase 23 sketch proposed per-channel typed senders
(`chat_tx: mpsc<ChatEvent>`, `presence_tx: mpsc<PresenceEvent>`, …),
which would have forced every emit site to change signatures and
forced `event_resume` to dispatch typed enums by tag. That is
mechanical churn for the same on-the-wire result.

Routing through a single `mpsc<EmitEnvelope { channel, payload }>`
gives the architectural win — one `app.emit()` callsite at the tail
of the loop — with zero signature changes at emit sites. The existing
`emit_live` / `emit_journaled` helpers keep their shape and just
route payloads through the queue.

## What the queue buys

- **One emission site.** The literal text `app.emit(` appears in
  `src-tauri/` exactly once, in `event_dispatch::spawn_dispatch_loop`.
  Any future occurrence is a regression and is caught by CI grep.
- **Cross-cutting concerns attach at one place.** Rate limiting,
  telemetry, tracing, batching, channel-scoped backpressure can all
  be added to the dispatch loop body without touching emit sites.
- **Replay sees the same events live listeners see.** The
  `event_resume` Tauri command pushes replayed events through the
  same queue, so future listeners (dev-tools, logging) treat replays
  identically to live events.

## Wire shape

`event_dispatch.rs` defines three public types:

```rust
pub struct TauriEmitRecord {
    pub channel: String,
    pub payload: serde_json::Value,
}

pub struct CursorTick {
    pub cursor: u64,
}

pub struct EventDispatch { /* mpsc tx + rx holder */ }
```

`TauriEmitRecord` is the wire shape persisted in the journal. The
`channel` lets the frontend route the payload to the same listener
that would have received it live; `payload` is the original
`ChatEvent` / `NotificationEvent` / etc. already serialized to JSON.

`CursorTick` fires on a dedicated `cursor-tick` channel alongside
every journaled emit. The frontend writes the latest cursor to
`localStorage`. The tick is decoupled from payload schemas so adding
new event types does not change the cursor protocol.

`EventDispatch` is shared on `AppState.event_dispatch:
Arc<EventDispatch>`. The `tx` half is cloned freely; the `rx` half is
consumed exactly once by `spawn_dispatch_loop()` at app setup.

## Helpers

Two helper functions cover every emit:

- `emit_live(app, channel, payload)` — fire-and-forget. Used for
  events the frontend can afford to miss across reconnect (typing
  indicators, presence updates while online, voice connection
  quality).
- `emit_journaled(app, state, channel, payload)` — push through the
  queue **and** append to the journal. Used for events the frontend
  must not miss across soft stalls (incoming messages, friend
  requests, governance state changes).

The two helpers were the same `event_emit::*` functions that lived
under `src-tauri/src/event_emit.rs` before Phase 23.A; that file is
deleted and every `crate::event_emit::*` import is now
`crate::event_dispatch::*`.

## Journal: `EventJournal` from `rekindle-events`

The journal lives in `crates/rekindle-events/src/journal.rs` and is
re-exported as `rekindle_events::{EventJournal, JournalCursor,
JournalEntry}`. The `AppState.event_journal` field is a typed
`EventJournal<TauriEmitRecord>`.

Every emitted event gets a unique `JournalCursor` (monotonic
`AtomicU64`, starts at `1` per process). The frontend persists the
most-recent cursor to `localStorage`. On soft stalls (page reload
during dev, brief IPC pause, hot-reload across the Tauri ↔ webview
bridge) it calls the `event_resume` Tauri command, which returns
`EventJournal::replay_since(cursor)` — every entry with
`cursor > since`, in arrival order.

The journal is **in-memory only**. The cursor counter resets to `1`
on every process start and the ring buffer is empty after `kill -9` +
restart. Cold-start recovery for persisted state (DMs, community
messages, friend requests) goes through SQLite via the existing
message-history Tauri commands. The journal handles the gap between
"the event fired" and "the frontend's live listener was installed"
**within a single process lifetime**.

The journal is bounded to 10 000 entries. Older entries are dropped
silently. If the frontend's saved cursor is older than the oldest
entry, `replay_since` returns the full ring buffer; the frontend
treats that as "soft stall too long, do a full hydration."

## Reconnect / replay flow

```
Frontend (on listener install):
    saved = localStorage.getItem("event-cursor") or 0
    backlog = invoke("event_resume", { sinceCursor: saved })
    for entry in backlog:
        dispatch(entry.payload) to listener(entry.channel)
    listen("cursor-tick", e => localStorage.setItem("event-cursor", e.cursor))
    listen("chat-event"|"presence-event"|... live)
```

The `event_resume` command lives at
`src-tauri/src/commands/event.rs` and reads the high-water mark from
`AppState.event_replay_watermark`. Each call moves the watermark
forward, so a flaky frontend never re-receives an event it has
already acknowledged.

## What does NOT go through the queue

Two surfaces bypass `event_dispatch` deliberately:

1. **Video frames** — `src-tauri/src/video_channels.rs` exposes
   per-channel `tokio::broadcast` senders. Frames are 28 KB or
   more and arrive at video frame rate; routing through the single
   mpsc would bottleneck. The frontend subscribes to those channels
   directly via Tauri's `Channel<T>` mechanism.
2. **Voice packets** — same reason. `AppState.voice_packet_tx` is a
   dedicated `mpsc<VoicePacket>` consumed by the voice receive loop.

Everything else — chat, presence, notifications, community events,
calls, file transfer progress, governance — flows through
`EventDispatch`. The contract is enforced by grepping for
`app.emit(` outside `event_dispatch.rs` in the doc-budget verification
gauntlet.

## CI invariants

- `grep -n 'app\.emit(' src-tauri/src/**/*.rs | grep -v event_dispatch.rs`
  must return zero lines.
- `grep -n 'tx\.send\|tx\.try_send' src-tauri/src/event_dispatch.rs`
  must return exactly the dispatch loop's internal sender + the
  helpers — the queue must not be sent into from outside.

See [`decisions/0007-event-dispatch-single-source.md`](../decisions/0007-event-dispatch-single-source.md)
for the rationale and the rejected per-channel-typed alternative.
