# 0013 — Typed webview events delivered per window, no Tauri `Emitter`

- **Status:** Accepted
- **Date:** 2026-10-04
- **Supersedes:** the delivery mechanism and replay contract of
  [0007](0007-event-dispatch-single-source.md) (one global `app.emit` per envelope,
  `event_resume` broadcast replay). 0007's single dispatch queue stands.

## Context and problem statement

0007 funnelled every event through one queue that ended in `app.emit(channel, payload)`. Verified
against Tauri 2.10/2.12 source (`.claude/plans/standards-remediation/evidence/steps-03-09.md` step 5):

- A JS `listen()` registers `EventTarget::Any`, and the backend filters on the listener's target,
  so even `emit_to(label, …)` reaches every webview that listened. Every window received every
  event: a chat window saw other peers' DMs, every window ran its own call UI (N rings, N OS
  notifications, N camera pipelines during a video call).
- `event_resume` replayed the journal through the same global emit, so one window's reload
  re-delivered the backlog to all windows; the cursor lived in `localStorage`, shared by all.
- The queue carried untyped `(String, Value)`, so nothing could say which windows an event was for.
- Rule B17 ("`app.emit(` only in `event_dispatch.rs`") was a text grep that missed `window.emit`
  and `app.handle().emit`, and was never wired into CI.

## Decision drivers

- An event reaches exactly the windows that show its subject.
- Device-wide UI (ring, OS notification, inbox, session-reset prompt) has one owner.
- Routing follows the Tier 1 scope every frontend shares (backend owns policy).
- Enforcement is structural, not a grep.

## Considered options

- **Keep the global emit, filter in each window.** Every webview still receives everything,
  including other conversations' plaintext; rejected.
- **`emit_to` per label.** Does not isolate (see above).
- **One `tauri::ipc::Channel` per window, routed in Rust (selected).** A `Channel` argument is
  bound to the invoking webview, delivers in order (indexed reorder buffer), and only that webview
  receives it.

## Decision outcome

- Emitters push a typed `event_dispatch::WebviewEvent` onto the 0007 queue. `audience()` is an
  exhaustive match; for a `SubscriptionEvent` it follows `SubscriptionEvent::scope()` (Tier 1
  `EventScope`): device-wide → every post-login window; community → the buddy list and every
  community window; peer → the buddy list, that peer's chat and profile windows; DHT conversation →
  its DM window; call → the buddy list, that call's window and the chat/DM windows.
  Notifications and focus requests go to the buddy list only.
- Each window calls `subscribe_events(channel, last_seq)` once its handlers are registered.
  `event_router::WebviewRouter` keeps the latest channel per label (a reload replaces it) and drops
  it on `WindowEvent::Destroyed`.
- Replay happens only for a webview that reloads: it passes the last journal sequence number it
  saw (kept in its own `sessionStorage`) and receives the journaled events after it that were
  addressed to it, sent before any live event. A new window replays nothing; it hydrates from
  the database.
- OS notifications are decided once, in the dispatch loop, by the pure
  `rekindle_events::notify_policy` and shown by `os_notify`; `tauri-plugin-notification` is gone.
- No code in `src-tauri` uses Tauri's `Emitter`; `cargo xtask check-no-emitter` (a syn walk)
  enforces it (rule B17). No window holds a `core:event` grant.

## Consequences

**Positive.** Per-window isolation of conversations; one ring, one notification, one camera
pipeline per call; replay without cross-window duplicates; the routing table is shared with the
daemon router and TUI through `EventScope`.

**Negative.** A new window kind or event needs an audience row (compile-checked). Community
windows switch communities in place, so community events go to every community window.

## More information

- `src-tauri/src/event_dispatch.rs`, `src-tauri/src/event_router.rs`, `src-tauri/src/os_notify.rs`
- `crates/rekindle-types/src/subscription_events/scope.rs`,
  `crates/rekindle-events/src/notify_policy.rs`
- [Tauri — calling the frontend from Rust](https://v2.tauri.app/develop/calling-frontend/)
- [0007](0007-event-dispatch-single-source.md), [0010](0010-single-daemon-thin-frontends.md)
