# Tauri Application Shell

The `src-tauri/` directory contains the Tauri 2 application shell that
bridges the pure Rust crates to the SolidJS frontend. It manages
application state, IPC commands, event emission, background services,
window lifecycle, and system tray integration.

This document is the overview. Two sibling documents go deeper:

- [`tauri-state.md`](tauri-state.md) — the ~47-field `AppState`
  anatomy, `state/` submodule split, and `state_helpers/` read-only
  accessor catalogue.
- [`tauri-modules.md`](tauri-modules.md) — every top-level
  module file under `src-tauri/src/` that is not a command, event,
  service, or state surface (event dispatch, audit repo, channel /
  message materialisation, deep links, tray, etc.).

## Entry Point

`src-tauri/src/lib.rs` carries `#![recursion_limit = "512"]` — required
for `veilid-core`'s deeply nested future types under the Tauri command
macros. The default value (128) is not enough.

The `run()` function:

1. Builds the Tauri runtime with `tauri::Builder::default()`.
2. Registers plugins in dependency order (see below).
3. Installs the `setup()` hook from `setup.rs` — Veilid init, DB pool,
   event-loop spawn, shortcut registration.
4. Wires `tauri::generate_handler![...]` from `invoke.rs` — **~241
   commands** registered.
5. Hands off to `app.run(|app, event| ...)` with custom handling for
   window-close (buddy list hides to tray; closing login while no buddy
   list is visible exits the app) and `RunEvent::Exit` (delegates to
   `shutdown::handle_exit`).

`AppState` is the central state container, shared as `SharedState =
Arc<AppState>`. Fields use `Arc<RwLock<T>>` for read-heavy access and
`Arc<Mutex<T>>` for exclusive access. The full field-by-field reference
lives in [`tauri-state.md`](tauri-state.md).

## Plugin Registration

Only the `login` window is declared in `tauri.conf.json`. All other
windows are created at runtime by `src-tauri/src/windows.rs`. Plugins
are registered in this order:

| Order | Plugin | Rationale |
|-------|--------|-----------|
| 1 | `single-instance` | Must be first — prevents duplicate processes; deep links re-route to existing instance |
| 2 | `notification` | System notifications |
| 3 | `store` | Persistent user preferences |
| 4 | `opener` | URL/file opening via system default handler |
| 5 | `dialog` | File dialogs (e.g. attachment uploads) |
| 6 | `process` | Process info (for updater) |
| 7 | `deep-link` | `rekindle://` URL scheme |
| 8 | `autostart` | Launch at system boot (LaunchAgent on macOS) |
| 9 | `global-shortcut` | Registered in `setup()` for state access |

Notable absences:

- `tauri-plugin-window-state` — removed due to infinite `windowDidMove`
  loop on macOS combined with `prevent_exit()`. (See tauri#11489.)
- `tauri-plugin-stronghold` — Stronghold has been replaced entirely by
  `rekindle-vault` (SQLCipher double-encryption). See
  [`decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md).
- `tauri-plugin-sql` — replaced with direct `rusqlite` for
  `veilid-core` compatibility (matched dep version).

### Global Shortcuts

Registered in `setup()` so the handler closes over `SharedState`.

| Combo | Action |
|-------|--------|
| `Ctrl+Shift+X` (`Cmd+Shift+X` on macOS) | Toggle buddy list visibility |
| `Ctrl+Shift+M` (`Cmd+Shift+M` on macOS) | Toggle voice mute |

## IPC Patterns

| Pattern | Direction | Mechanism |
|---------|-----------|-----------|
| Commands | Frontend → Rust | `invoke()` / `#[tauri::command]` |
| Events | Rust → Frontend | `event_dispatch::emit_*()` → `listen()` |

Every Rust → Frontend emit flows through
`src-tauri/src/event_dispatch.rs` (Phase 23.A single-source router).
Cross-cutting concerns — rate limiting, journalling, dedup, replay on
reconnect — live in one place. See
[`event-dispatch.md`](event-dispatch.md).

### Command Module Summary

Commands are registered in `src-tauri/src/invoke.rs` via
`tauri::generate_handler![...]`. They are organised by domain under
`src-tauri/src/commands/`:

| Module | Approx commands | Highlights |
|--------|---------|-----------|
| `auth.rs` | 6 | `create_identity`, `login`, `logout`, `list_identities`, `delete_identity`, `get_identity` |
| `chat.rs` | 5 | `prepare_chat_session`, `send_message`, `send_typing`, `get_message_history`, `mark_read` |
| `friends.rs` | 17 | Add / remove / accept / reject, groups, invites, block / unblock, presence emit |
| `dm.rs` | 6 | `list_dms`, `start_dm`, `accept_dm_invite`, `decline_dm_invite`, `send_dm_message`, `get_dm_messages` |
| `calls.rs` | 12 | `start_dm_call`, `accept_dm_call`, `decline_dm_call`, `end_dm_call`, `send_call_media_state`, `send_call_reaction`, `mute_caller_temp`, `start_group_call`, `accept_group_call`, `decline_group_call`, `end_group_call`, `get_missed_calls` |
| `voice.rs` | 12 | Join / leave channel, mute, deafen, devices, mode, server-mute, stage hand-raise |
| `community/` | ~127 across 32 submodules | CRUD, channels, channel_admin, messaging, roles, moderation, mek, invites, notifications, reactions_pins, audit, analytics, automod, polls, expressions, files, link_previews, threads, events, game_servers, unread, presence, onboarding, segments, video, background_sync, diagnostics, profile_blobs, policy |
| `status.rs` | 5 | Set status / nickname / avatar / status message / get avatar |
| `game.rs` | 3 | `get_game_status`, `get_game_name`, `launch_game_to_server` |
| `relay.rs` | 4 | Strand Relay volunteer / revoke / list (received / volunteered) |
| `search.rs` | 1 | `search_messages` — FTS5 over `messages_fts` / `thread_messages_fts` / `dm_messages_fts` |
| `sync.rs` | 10 | Personal sync record, pairing, read / write manifest / read_state / preferences / paired_devices |
| `push_relay.rs` | 3 | Register / unregister / list push relay |
| `settings.rs` | 3 | Get / set preferences, `check_for_updates` (stub — auto-updater not yet wired) |
| `vault.rs` | several | Vault unlock / lock, passphrase change, audit verify / export |
| `event.rs` | several | Event resume / cursor for Phase 10 reconnect replay |
| `window.rs` | 7 | `show_buddy_list`, `open_chat_window`, `open_dm_window`, `open_settings_window`, `open_community_window`, `open_profile_window`, `get_network_status` |

`invoke.rs` registers the union of these — the file is the canonical
source of truth for the IPC surface.

## Event Channels

Events are the Rust → Frontend push mechanism, emitted through
`event_dispatch::EventDispatch` and received via `listen()` on the
frontend. All event enums use
`#[serde(rename_all = "camelCase", tag = "type", content = "data")]`.

### `chat-event` (`channels/chat_channel.rs`)

`MessageReceived`, `TypingIndicator`, `MessageAck`, `FriendRequest`,
`FriendRequestAccepted`, `FriendRequestRejected`, `FriendAdded`,
`FriendRemoved`, `FriendRequestDelivered`, `DirectMessageInvite`,
`IncomingCall`, plus the call-state ack variants emitted by
`rekindle-calls`.

### `presence-event` (`channels/presence_channel.rs`)

`FriendOnline`, `FriendOffline`, `StatusChanged`, `GameChanged`.

### `voice-event` (`channels/voice_channel.rs`)

`UserJoined`, `UserLeft`, `UserSpeaking`, `UserMuted`,
`ConnectionQuality`, `DeviceChanged`.

### `community-event` (`channels/community_channel/`)

A directory of submodules — over 50 variants — covering membership,
governance, channel messages, threads, events, voice / stage / video,
expressions, files, link previews, audit, automod, raid detection.

### `notification-event` + `network-status`
(`channels/notification_channel.rs`)

`NotificationEvent { MessageReceived, SystemAlert, UpdateAvailable }`
plus a flat `NetworkStatusEvent` struct
(`attachmentState`, `isAttached`, `publicInternetReady`, `hasRoute`).

## Background Services

Services live under `src-tauri/src/services/` (~229 `.rs` files). The
Veilid dispatch loop is the central event router; everything else is a
domain service spawned after login. The directory uses a deliberate
**runtime / adapter / pure-logic** layering — see
[`services-pattern.md`](services-pattern.md) for the full pattern. The
high-level layout:

| Surface | Path | Responsibility |
|---|---|---|
| Veilid | `services/veilid/{lifecycle,app_message,network,dht_watch,control_*}` | Node lifecycle, dispatch loop, route refresh, status |
| Messaging | `services/message_service/`, `messaging_runtime.rs`, `chat_runtime.rs` | Inbound friend `AppMessage` ingest, outbound envelope sign / dispatch |
| Friends | `services/friend_runtime/`, `friendship.rs` | Friend list, inbox-scan coordinator wiring |
| Calls | `services/calls_adapter/`, `call_runtime.rs`, `voice_signaling_adapter.rs` | Phase 14.q `CallRegistry` impl, signaling RPCs |
| Voice | `services/voice/`, `voice_adapter/`, `voice_runtime.rs` | Send / receive / MCU loops, device monitor |
| Communities | `services/community/` (gossip, governance, presence, channels, threads, polls, reactions, video, expressions, files, link previews, automod, raid detection, segments, stage, MEK rotation, analytics, join), plus runtime files (`community_*_runtime.rs`) | Community state machine + harvest adapters |
| DMs | `services/dm/`, `dm_runtime.rs`, `dm_adapter.rs` | DM lifecycle and persistence |
| Sync | `services/cross_device_sync/`, `sync_runtime.rs`, `sync_adapter/`, `sync_communities.rs` | Pairing, record sync, gap reconciliation |
| Relay | `services/relay/`, `push_relay.rs` | Strand Relay forwarding, mobile push relay client |
| Search | `services/search/` | FTS5 query, context, threads, DM, messages |
| Auth | `services/auth_runtime.rs`, `auth_cores.rs`, `login_runtime.rs`, `login_spawn.rs` | Login flow task spawning |
| Idle | `services/idle_service.rs` | Auto-away on inactivity (restores `pre_away_status`) |
| Status | `services/status_runtime.rs` | Status transitions |
| Game | `services/game_service.rs`, `game_publisher.rs` | Game detection + DHT publish |
| Presence | `services/presence_service.rs`, `presence_adapter/` | DHT presence ValueChange dispatch |
| MEK | `services/mek_adapter.rs` | Wires `rekindle-mek-rotation` |
| DHT publish | `services/dht_publish_service.rs` | Periodic profile re-publish |
| Event resume | `services/event_resume_runtime.rs` | Phase 10 reconnect replay |
| Window | `services/window_runtime.rs` | Window lifecycle handlers |

### Veilid Dispatch Loop

The dispatch loop classifies inbound `VeilidUpdate` variants and
delegates:

- `AppMessage` → classified by content prefix
  - `b'V'` prefix → voice receive loop
  - Community envelopes → `services/community/gossip` (after dedup)
  - Otherwise → `services/message_service` (1:1 friend traffic)
- `AppCall` → DM invite handshake / community RPC handlers
- `ValueChange` → friend `presence_service` (profile records),
  `services/community/watch` (community records), or
  `services/cross_device_sync/watch` (personal sync records)
- `Attachment` → update `NodeHandle` state, emit `NetworkStatusEvent`
- `RouteChange` → re-allocate private routes via `routing_manager`

## Window Management

Windows are created via helper functions in `src-tauri/src/windows.rs`:

| Function | Window | Behavior |
|----------|--------|----------|
| `open_login()` | Login | Destroys existing, supports `?account=` preselect |
| `open_buddy_list()` | Buddy list | Destroys existing, narrow vertical (320 × 650) |
| `open_chat_window()` | Chat | Show existing or create new, label `chat-{key prefix}` |
| `open_dm_window()` | DM | Show existing or create new, label `dm-{record key prefix}` |
| `open_community_window()` | Community | Show existing or create new, label `community-{id}` |
| `open_settings_window()` | Settings | Single instance |
| `open_profile_window()` | Profile | Show existing or create new, label `profile-{key prefix}` |
| `open_call_window()` | Call | Show existing or create new, label `call-{id}` |

All windows use `decorations: false` and `transparent: true` for the
frameless Xfire look with custom titlebars.

The buddy list hides to tray on close (`api.prevent_close()`). Closing
the login window while no buddy list is visible triggers `app.exit(0)`.
Other windows close normally.

## System Tray, Deep Links, Shutdown

These three responsibilities live in single-purpose top-level modules
catalogued in [`tauri-modules.md`](tauri-modules.md):

- `tray.rs` — context menu with status controls (Online / Away / Busy /
  Invisible), show / hide buddy list, quit.
- `deep_links.rs` — `rekindle://` URL parsing. On macOS the URL is
  delivered via `app.deep_link().on_open_url(...)`; on Windows/Linux
  the `single-instance` callback re-routes. Pre-auth links are buffered
  in `AppState.pending_deep_link` and replayed on login.
- `shutdown.rs` — `handle_exit()` performs an ordered tear-down with a
  5-second timeout: close user DHT state, signal dispatch loop +
  domain shutdowns, stop game detection, stop voice engine, shut down
  Veilid node.

## Concurrency Patterns

- `parking_lot::RwLock` for read-heavy state (identity, friends,
  communities).
- `parking_lot::Mutex` for exclusive access (voice engine, game
  detector, MEK caches, dedup cache, audit chain).
- Guards are `!Send` — clone data out before `.await` points.
- `tokio_rusqlite::Connection` for the database pool (async wrapper
  over `rusqlite` on a dedicated background thread). All access goes
  through `db_helpers::{db_call, db_call_or_default, db_fire}`.
- `rusqlite` 0.37 is used (not `sqlx`) to match `veilid-core`'s
  dependency on the same version and avoid `libsqlite3-sys` build
  conflicts.
- `tokio::sync::watch` for network readiness and event-reminder wake
  signals.
- `tokio::sync::mpsc` for shutdown channels and event dispatch.
- `cpal::Stream` is `!Send` on macOS — voice capture and playback live
  on dedicated OS threads and bridge to Tokio via `mpsc`.
