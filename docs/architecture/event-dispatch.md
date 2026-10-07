# Event dispatch — backend to webview

How the Tauri backend sends events to its windows. Decision records:
[`0007`](../decisions/0007-event-dispatch-single-source.md) (one queue) and
[`0013`](../decisions/0013-per-window-event-router.md) (typed events, per-window delivery).

## Pipeline

```
emitter ──► event_dispatch::emit / emit_journaled ──► one mpsc queue
                                                          │
                                       dispatch loop (one task)
                                         ├─ os_notify::notify (policy: rekindle_events::notify_policy)
                                         └─ event_router.deliver(envelope, event.audience())
                                                          │
                                  per-window tauri::ipc::Channel<OutboundEnvelope>
                                                          │
                       webview: subscriptions.ts demux ──► subscribe* handlers
```

- **`WebviewEvent`** (`src-tauri/src/event_dispatch.rs`) is every event a window can receive:
  the shared `SubscriptionEvent` vocabulary, lifecycle transitions, deep-link requests,
  profile updates, settings-tab switches, and two legacy community shapes (`ChannelChat`,
  `Community`) that step E6 deletes.
- **`audience()`** is an exhaustive match, so a new event does not compile without a routing
  row. A `SubscriptionEvent` routes by its Tier 1 `EventScope`
  (`rekindle-types/src/subscription_events/scope.rs`):

  | Scope / family | Windows |
  |---|---|
  | `Notification`, `ConversationFocusRequested` | `buddy-list` (single owner) |
  | `Call(id)` events | `buddy-list`, `call-<id12>`, every `chat-*` and `dm-*` |
  | `Device` | every post-login window |
  | `Community(id)`, `CommunityJoin(id)` | `buddy-list`, every `community-*` (a community window switches communities in place) |
  | `Peer(pk)` | `buddy-list`, `chat-<pk16>`, `profile-<pk12>` (voice also every `call-*`) |
  | `Conversation(record)` | `buddy-list`, `dm-<record20>` |
  | Lifecycle | every window (login included) |

- **`WebviewRouter`** (`src-tauri/src/event_router.rs`) keeps one channel per window label. A
  reload re-registers and replaces the dead channel (Tauri cannot detect it);
  `WindowEvent::Destroyed` removes the entry.
- **Frontend.** Each window registers its handlers, then calls `startEventStream()`
  (`src/ipc/channels/subscriptions.ts`), which opens the channel with `subscribe_events`.
  The logical channel names (`chat-event`, `community-event`, …) are kept, so handlers
  did not change shape.

## Journal and reload

`emit_journaled` also appends the event to `AppState.event_journal`
(`rekindle_events::EventJournal`, 10 000 entries, in memory) and stamps the envelope with its
sequence number. A webview remembers the last sequence it processed in its own
`sessionStorage`. On a reload it passes that number to `subscribe_events`, which sends the
journaled events after it that were addressed to this window, before any live event; the
frontend skips a sequenced envelope it has already seen. A window that just opened passes
nothing and hydrates from the database instead. Logout clears the journal.

## OS notifications

The dispatch loop gives each `SubscriptionEvent` to `os_notify::notify`. Candidates
(`NotificationEvent`, event reminders) are decided by the pure
`rekindle_events::notify_policy::os_notification_for` from the user's preferences, Do Not
Disturb, quiet hours, call state and Busy status, and shown with `notify-rust` (async on Linux,
`spawn_blocking` on macOS/Windows). Message notifications were already filtered for DND, quiet
hours and channel level when emitted.

## What does not go through the queue

- **Video frames** and the native self-view use their own `tauri::ipc::Channel`s
  (`video_channels.rs`); frame-rate traffic would bottleneck the queue.
- **Voice packets** use the dedicated `AppState.voice_packet_tx`.

## Invariants

- No Tauri `Emitter` anywhere in `src-tauri` — `cargo xtask check-no-emitter` (rule B17).
- No window holds a `core:event` grant — `tests/capability_policy.rs`.
- Every `WebviewEvent` and every `SubscriptionEvent` variant has an audience — the compiler.
- Routing rows are unit-tested in `event_dispatch.rs`; label/kind mapping in `window_labels.rs`.
