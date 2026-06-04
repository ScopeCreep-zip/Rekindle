# Top-Level src-tauri Modules

This is the catalogue of every top-level file and directory under
`src-tauri/src/` that is **not** a command (`commands/`), event channel
(`channels/`), background service (`services/`), state container
(`state/`, `state_helpers/`), or keystore submodule. Those four
surfaces are covered in [`tauri-backend.md`](tauri-backend.md),
[`tauri-state.md`](tauri-state.md), and the keystore section of
[`data-layer.md`](data-layer.md).

The modules below are roughly grouped by responsibility — entry points,
persistence adapters, event plumbing, single-purpose glue.

## Entry Points and Wiring

### `lib.rs`

Application entry. Carries `#![recursion_limit = "512"]` for
`veilid-core` + Tauri macros. Builds the Tauri runtime, registers
plugins, installs `setup::run`, wires `tauri::generate_handler!` from
`invoke.rs`, and dispatches `RunEvent::Exit` to `shutdown::handle_exit`.

### `main.rs`

Desktop binary entry point. Calls `lib::run()`.

### `setup.rs`

The `.setup()` hook. Initialises the database pool, opens the vault,
starts the Veilid node, spawns the event dispatch loop, registers
global shortcuts, and installs the deep-link handler.

### `invoke.rs`

Houses `tauri::generate_handler![...]` listing every command function
registered with Tauri. Currently ~241 entries. This file is the
single source of truth for the desktop IPC surface.

### `platform.rs`

Per-OS setup glue. Linux display detection, macOS bundle entitlements
prep, Windows registry helpers.

## Persistent Stores and Repositories

### `db.rs`

SQLite pool (`tokio_rusqlite::Connection`), `SCHEMA_VERSION = 71`,
schema bootstrap from `migrations/001_init.sql`. On mismatch, drops
all tables and triggers the synchronised reset of the vault and
Veilid storage described in [`data-layer.md`](data-layer.md).

### `db_helpers.rs`

`db_call`, `db_call_or_default`, `db_fire` — the three wrappers every
command and service uses to access the pool. They standardise the
"clone-out, drop-guard, await" pattern required by the
`tokio_rusqlite` background thread.

### `friend_repo.rs`

Friend list CRUD against `friends` / `friend_groups` /
`blocked_users` / `pending_friend_requests` / `outgoing_invites`.

### `channel_repo.rs`

Community channel CRUD against `channels` / `channel_overwrites` /
`channel_pins` / `channel_slowmode_state`.

### `channel_materialize.rs`

Materialises channel state for the UI. Reads governance + channel
records + per-channel MEK and returns a denormalised snapshot the
frontend store can mirror.

### `message_repo.rs`

Message persistence against `messages` / `message_delivery` /
`thread_messages`. Handles dedup constraint enforcement and the
`messages_fts` / `thread_messages_fts` triggers.

### `message_view.rs`

Read-side message view helpers. Builds the paginated history payloads
the chat windows consume.

### `envelope_store_sqlite.rs`

SQLite-backed implementation of
`rekindle_transport::EnvelopeStore`. Provides the durable retry queue
for `pending_envelopes` and the seen-set for `seen_envelopes`.

### `friend_store_sqlite.rs`

SQLite-backed implementation of `rekindle_transport::FriendStore`.
Receive-path authority — the transport asks "is this peer a friend?"
without going through `AppState.friends`.

### `signal_stores.rs`

DB-level Signal Protocol session / pre-key persistence. Layers above
this go through `rekindle_crypto::signal::SignalSessionManager`; the
secret material itself lives in the vault under `keystore/signal.rs`.

### `audit_repo/`

Audit chain persistence. Submodules: `chain` (chain-walking helpers),
`store` (`audit_entries` table reads / writes), `tests`. The MAC key
lives in the vault. See [`audit-chain.md`](audit-chain.md).

### `audit_view.rs`

Read-only audit log queries used by `commands/community/audit.rs` and
`commands/vault.rs::audit_verify`.

### `community_loader/`

Community state restore on startup. Submodules: `assemble` (merge DB
rows into `CommunityState`), `friends` (community-friend relationships),
`restore` (top-level entry point), `rows` (column projections).

### `video_channels.rs`

Phase 11 Tier 1 high-throughput video channel registry. Used by
`active_calls` and `group_calls`. Video frames bypass `event_dispatch`
to avoid hot-path overhead — the registry exposes per-channel
`tokio::broadcast` senders that the frontend's `WebRTCSubscriber`
listens to.

## Event Plumbing

### `event_dispatch.rs`

Phase 23.A single-source emit router. Every Rust → Frontend
`app.emit()` call flows through `EventDispatch`. Centralises rate
limiting, journaling, dedup, telemetry. See
[`event-dispatch.md`](event-dispatch.md) for the full pattern and
contract; the field on `AppState` is `event_dispatch:
Arc<EventDispatch>`.

## Single-Purpose Glue

### `keystore/`

VaultStore wrapper, organised by domain:

- `keystore/signal.rs` — Signal identity / sessions / pre-keys, PQ
  secrets.
- `keystore/community_keys.rs` — community MEK, slot / registry
  keypairs, slot seed.
- `keystore/channel_mek.rs` — per-channel + per-generation MEK.
- `keystore/audit.rs` — audit MAC key + tail anchor.

All four are backed by `rekindle-vault` (SQLCipher double-encryption);
no direct `iota_stronghold` dependency remains. See
[`data-layer.md`](data-layer.md) for the vault layout and
[`decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md)
for the migration rationale.

### `windows.rs`

Window creation helpers — one function per window type
(`open_login`, `open_buddy_list`, `open_chat_window`, `open_dm_window`,
`open_community_window`, `open_settings_window`, `open_profile_window`,
`open_call_window`). Encapsulates frameless-transparent window options,
label naming conventions, and "show existing or create" idempotence.

### `tray.rs`

System tray icon + menu. Context menu provides status controls
(Online / Away / Busy / Invisible), show / hide buddy list, quit.

### `shortcuts.rs`

Global hotkey registration (`Ctrl+Shift+X` toggle buddy list,
`Ctrl+Shift+M` toggle voice mute). Installed in `setup()` so the
handler closes over `SharedState`.

### `shutdown.rs`

`handle_exit()` — ordered tear-down with a 5-second timeout. Closes
user DHT state, signals dispatch loop + per-service shutdowns, stops
game detection, stops voice engine, shuts down Veilid node.

### `deep_links.rs`

`rekindle://` URL parsing. `DeepLinkAction` enum covers the supported
URL schemes (add-friend, join-community, open-DM). On macOS the URL is
delivered via `app.deep_link().on_open_url(...)`. On Windows / Linux
the `single-instance` callback re-routes to the existing instance.
Pre-auth links are buffered in `AppState.pending_deep_link` and
replayed on login.

### `invite_helpers.rs`

Outgoing invite tracking — builds the `outgoing_invites` rows and
emits the events that the buddy-list invite tab consumes.

### `serde_helpers.rs`

Custom serde adapters used across the IPC surface. Mostly base64 /
hex / byte-array shims for fields that need to cross the
JSON boundary.
