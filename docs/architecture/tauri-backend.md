# Tauri Application Shell

The `src-tauri/` directory contains the Tauri 2 application shell that bridges
the pure Rust crates to the SolidJS frontend. It manages application state,
IPC commands, event emission, background services, window lifecycle, and
system tray integration.

## Application State

`AppState` is the central state container, shared across all commands and
services as `SharedState = Arc<AppState>`. Fields use `Arc<RwLock<T>>` for
read-heavy access and `Arc<Mutex<T>>` for exclusive access. See
`src-tauri/src/state.rs` for the authoritative definition.

### Identity, Veilid, and Lifecycle

| Field | Purpose |
|-------|---------|
| `identity` | Logged-in user identity (set after Stronghold unlock) |
| `identity_secret` | Ed25519 secret bytes (for envelope signing) |
| `node` | Veilid node handle (`NodeHandle`) |
| `dht_manager` | DHT record manager (`DHTManagerHandle`) |
| `routing_manager` | Private route lifecycle (`RoutingManagerHandle`) |
| `signal_manager` | Signal session manager |
| `app_handle` | Tauri AppHandle (set during `setup()`) |
| `network_ready_tx` / `_rx` | `tokio::sync::watch` for Veilid public-internet readiness |
| `dispatch_loop_handle` | Veilid dispatch loop join handle |
| `background_handles` | Spawned background task handles (aborted on logout) |
| `pending_deep_link` | Deep link received pre-auth (replayed post-login) |

### Friends & Communities

| Field | Purpose |
|-------|---------|
| `friends` | `HashMap<pubkey, FriendState>` with presence |
| `unwatched_friends` | Friends whose DHT watch failed (sync_service polls them with `force_refresh=true`) |
| `pre_away_status` | Status before auto-away (restored on activity) |
| `communities` | `HashMap<community_id, CommunityState>` |
| `community_circuit_breakers` | Per-community circuit breaker for remote RPCs (3 fails → 30s cooldown) |
| `mek_cache` | Legacy community-level MEK cache |
| `channel_mek_cache` | Per-channel MEK cache `(community_id, channel_id) → MEK` |
| `automod_cache` | Per-community compiled AutoMod regex cache |
| `event_reminder_wake_tx` | Wake-up signal for event reminder scheduler |
| `dedup_cache` | Global gossip-mesh dedup cache |
| `channel_write_retry_tx` | Queued SMPL channel-message write handle |

### Voice, Game Detection, Files

| Field | Purpose |
|-------|---------|
| `voice_engine` | `VoiceEngineHandle` (engine + transport + send/recv/MCU/device-monitor task handles) |
| `voice_packet_tx` | Routes inbound voice packets from dispatch loop to receive loop |
| `game_detector` | Game detection state |
| `file_caches` | Per-community Lost Cargo `ChunkCache` map |
| `pinned_attachments` | Per-community pinned attachment IDs (skipped during eviction) |
| `file_cache_root` | `<app_data>/file_cache/` root path |
| `video_reassembly` | Per-community video / screen-share reassembly buffers |

### DMs, Relay, Push Relay

| Field | Purpose |
|-------|---------|
| `dm_mek_cache` | DM MEK chain (genesis + every materialized generation) |
| `relay_probe_cooldown` | Strand Relay status-probe cooldown (60s window) |
| `relay_reliability_dirty` | Mutual Aid §14.5 dirty set (flushed to SQLite every 30s) |
| `last_wake_notify_secs` | Push-relay wake-notify debounce |

### Shutdown Channels

`shutdown_tx`, `sync_shutdown_tx`, `route_refresh_shutdown_tx`,
`idle_shutdown_tx`, `heartbeat_shutdown_tx` — `mpsc::Sender<()>` for graceful
shutdown of the corresponding background service.

`parking_lot` mutexes are used for synchronous access. Guards are `!Send` —
data must be cloned out before `.await` points.

## Command Modules

Commands are the Frontend → Rust IPC mechanism. Each is a `#[tauri::command]`
function registered in `lib.rs`. Approximately 220 commands are registered.
Subsections below give the per-module breakdown — see `src-tauri/src/lib.rs`
for the canonical registration list.

### auth (6)

`create_identity`, `login`, `get_identity`, `logout`, `list_identities`,
`delete_identity`

### chat (5)

`prepare_chat_session`, `send_message`, `send_typing`, `get_message_history`,
`mark_read`

### friends (17)

`add_friend`, `remove_friend`, `accept_request`, `reject_request`,
`get_friends`, `get_pending_requests`, `create_friend_group`,
`rename_friend_group`, `move_friend_to_group`, `generate_invite`,
`add_friend_from_invite`, `block_user`, `unblock_user`, `get_blocked_users`,
`cancel_request`, `cancel_invite`, `get_outgoing_invites`,
`emit_friends_presence`

### dm (6)

`list_dms`, `start_dm`, `accept_dm_invite`, `decline_dm_invite`,
`send_dm_message`, `get_dm_messages`

### community (~127, split across 30 modules under `commands/community/`)

The community surface is grouped by feature module. Highlights:

| Module | Commands |
|--------|----------|
| `crud.rs` | `create_community`, `join_community`, `leave_community`, `create_category`, `delete_category`, `rename_category` |
| `mod.rs` | `get_communities`, `get_community_details` |
| `channels.rs` | `create_channel`, `delete_channel`, `rename_channel`, `move_channel`, `reorder_channels`, `set_channel_topic`, `set_channel_forum_tags`, `get_channel_messages`, `get_older_channel_messages` |
| `channel_admin.rs` | `set_slowmode`, `update_community_info` |
| `messaging.rs` | `send_channel_message`, `edit_channel_message`, `delete_channel_message`, `forward_channel_message`, `admin_delete_channel_message`, `bulk_delete_channel_messages`, `send_voice_message` |
| `roles.rs` | `get_roles`, `create_role`, `edit_role`, `delete_role`, `assign_role`, `unassign_role`, `self_assign_role`, `self_unassign_role` |
| `moderation.rs` | `timeout_member`, `remove_timeout`, `ban_member`, `unban_member`, `get_ban_list`, `remove_community_member`, `set_channel_overwrite`, `delete_channel_overwrite`, `set_community_policy`, `get_community_policy` |
| `mek.rs` | `rotate_mek` |
| `invites.rs` | `create_community_invite`, `revoke_community_invite`, `list_community_invites` |
| `notifications.rs` | `set_channel_notification_level`, `set_community_default_notification_level`, `get_community_default_notification_level`, `set_notification_sound`, `get_notification_sound`, `set_do_not_disturb`, `get_do_not_disturb`, `set_quiet_hours`, `get_quiet_hours` |
| `reactions_pins.rs` | `add_reaction`, `remove_reaction`, `pin_message`, `unpin_message`, `get_channel_pins` |
| `audit.rs` / `analytics.rs` | `get_audit_log`, `get_community_analytics` |
| `automod.rs` | `list_automod_rules`, `set_automod_rule`, `delete_automod_rule` |
| `polls.rs` | `create_poll`, `vote_poll`, `close_poll`, `get_poll_results` |
| `expressions.rs` | `upload_emoji`, `upload_sticker`, `upload_soundboard_sound`, `play_soundboard`, `delete_emoji`, `list_expressions` |
| `files.rs` | `upload_attachment`, `download_attachment`, `pin_attachment` |
| `link_previews.rs` | `fetch_link_preview` |
| `threads.rs` | `create_thread`, `get_active_threads`, `get_channel_threads`, `send_thread_message`, `get_thread_messages`, `archive_thread`, `unarchive_thread` |
| `events.rs` | `create_event`, `edit_event`, `delete_event`, `cancel_event`, `rsvp_event`, `set_event_rsvp`, `list_event_attendees`, `get_events` |
| `game_servers.rs` | `add_game_server`, `remove_game_server`, `get_game_servers` |
| `unread.rs` | `mark_channel_read`, `get_unread_counts` |
| `presence.rs` | `update_community_presence`, `update_community_profile`, `send_channel_typing` |
| `onboarding.rs` | `get_onboarding_config`, `set_onboarding_config`, `get_welcome_screen`, `set_welcome_screen`, `submit_onboarding_answers` |
| `segments.rs` | `expand_community_segment` |
| `video.rs` | `derive_video_stream_id`, `default_media_capabilities`, `send_video_frame`, `notify_video_topology_change` |
| `background_sync.rs` | `run_background_sync` |
| `diagnostics.rs` | `debug_gossip_state` |

### voice (12)

`join_voice_channel`, `leave_voice`, `request_to_speak`,
`get_stage_hand_raises`, `respond_to_speak_request`, `set_mute`,
`set_deafen`, `list_audio_devices`, `set_audio_devices`, `set_voice_mode`,
`server_mute_member`, `server_deafen_member`

### status (5)

`set_status`, `set_nickname`, `set_avatar`, `get_avatar`, `set_status_message`

### game (3)

`get_game_status`, `get_game_name`, `launch_game_to_server`

### relay (4) — Strand Relay

`volunteer_relay`, `revoke_relay`, `list_received_relay_offers`,
`list_volunteered_relay_friends`

### search (1)

`search_messages` — full-text across `messages_fts`, `thread_messages_fts`,
`dm_messages_fts`

### sync (10) — Cross-device sync (architecture §28.4)

`ensure_personal_sync_record`, `start_pairing_session`, `accept_pairing_code`,
`read_sync_manifest`, `write_sync_manifest`, `read_sync_read_state`,
`write_sync_read_state`, `read_sync_preferences`, `write_sync_preferences`,
`read_paired_devices`, `write_paired_devices`

### push_relay (3)

`register_with_push_relay`, `unregister_with_push_relay`,
`list_push_relay_registrations`

### settings (3)

`get_preferences`, `set_preferences`, `check_for_updates` (stub — always
returns false; auto-updater not yet wired)

### window (7)

`show_buddy_list`, `open_chat_window`, `open_dm_window`,
`open_settings_window`, `open_community_window`, `open_profile_window`,
`get_network_status`

## Event Channels

Events are the Rust → Frontend push mechanism, emitted via `app.emit()` and
received via `listen()` on the frontend. All event enums use
`#[serde(rename_all = "camelCase", tag = "type", content = "data")]`.

### ChatEvent (`chat-event`)

| Variant | Fields |
|---------|--------|
| `MessageReceived` | `from`, `body`, `decryptionFailed`, `automodBlurred`, `timestamp`, `conversationId`, optional `serverMessageId` / `replyToId` / `senderDisplayName` |
| `TypingIndicator` | `from`, `typing` |
| `MessageAck` | `messageId` |
| `FriendRequest` | `from`, `displayName`, `message` |
| `FriendRequestAccepted` | `from`, `displayName` |
| `FriendRequestRejected` | `from` |
| `FriendAdded` | `publicKey`, `displayName`, `friendshipState` |
| `FriendRemoved` | `publicKey` |
| `FriendRequestDelivered` | `to` |
| `DirectMessageInvite` | `from`, `recordKey`, `initiatorPseudonym`, `isGroup` |

### PresenceEvent (`presence-event`)

| Variant | Fields |
|---------|--------|
| `FriendOnline` | `publicKey` |
| `FriendOffline` | `publicKey` |
| `StatusChanged` | `publicKey`, `status`, `statusMessage` |
| `GameChanged` | `publicKey`, `gameName?`, `gameId?`, `elapsedSeconds?` |

### VoiceEvent (`voice-event`)

`UserJoined`, `UserLeft`, `UserSpeaking`, `UserMuted`, `ConnectionQuality`,
`DeviceChanged`

### CommunityEvent (`community-event`) — 50+ variants

Membership: `MemberJoined`, `MemberRemoved`, `MemberRolesChanged`,
`MemberTimedOut`, `MemberPresenceChanged`, `MembersRefreshed`, `Kicked`,
`JoinAccepted`, `JoinRejected`, `OnboardingComplete`

Governance: `MekRotated`, `ChannelOverwriteChanged`, `GovernanceUpdated`,
`SyncComplete`

Channel messages: `MessageEdited`, `MessageDeleted`, `ReactionAdded`,
`ReactionRemoved`, `MessagePinned`, `MessageUnpinned`,
`ChannelMessageDelivered`, `ChannelMessageDeliveryFailed`, `ChannelTyping`

Threads & events: `ThreadCreated`, `ThreadMessageReceived`, `ThreadArchived`,
`EventCreated`, `EventUpdated`, `EventDeleted`, `EventRsvpChanged`,
`EventReminder`

Voice / stage / video: `VoiceJoin`, `VoiceLeave`, `VoiceModeSwitch`,
`StageUpdate`, `SpeakRequest`, `SpeakResponse`, `SoundboardPlay`,
`VideoFrame`, `VideoFrameAck`, `VideoKeyframeRequest`,
`VideoBandwidthEstimate`, `VideoMediaCapabilities`, `VideoTopologyChange`

Other: `LinkPreviewReceived`, `AttachmentDownloaded`, `GameServerAdded`,
`GameServerRemoved`, `RaidDetected`, `RaidAlert`, `ChannelLockdown`,
`SystemMessage`, `AutoModAlert`

### NotificationEvent (`notification-event`)

`MessageReceived`, `SystemAlert`, `UpdateAvailable`

### NetworkStatusEvent (`network-status`)

Flat struct (not a tagged enum):

| Field | Description |
|-------|-------------|
| `attachmentState` | Raw Veilid state (e.g. `attached_good`) |
| `isAttached` | Whether the node is attached |
| `publicInternetReady` | Whether DHT operations are available |
| `hasRoute` | Whether a private route is allocated |

## Background Services

Services live under `src-tauri/src/services/`. The Veilid dispatch loop is
the central event router; everything else is a domain service spawned after
login.

### Core

| Service | Path | Responsibility |
|---------|------|----------------|
| `veilid` | `services/veilid/{lifecycle,app_message,network,dht_watch,control_*}` | Node lifecycle, dispatch loop, route refresh, status tracking, DHT watch dispatch, control event handling |
| `message_service` | `services/message_service.rs` | Process incoming friend `AppMessage` payloads, sign outbound envelopes |
| `presence_service` | `services/presence_service.rs` | Process DHT `ValueChange` for friend presence |
| `sync_service` | `services/sync_service.rs` | Retry pending messages, poll unwatched friends |
| `dht_publish_service` | `services/dht_publish_service.rs` | Periodic profile re-publish |
| `idle_service` | `services/idle_service.rs` | Auto-away on inactivity (restores `pre_away_status`) |
| `game_service` | `services/game_service.rs` | Periodic game detection, publish to DHT |

### Communities

`services/community/` is a large feature surface:

- `gossip.rs` — D-fanout broadcast, dedup, forwarding
- `governance.rs` — Apply CRDT merge, persist `GovernanceState`
- `watch.rs` / `inspect.rs` — DHT watch + `inspect_dht_record` polling
- `presence/{poll,registry,sync}.rs` — Member registry presence loop
- `keepalive.rs` — DHT record keepalive
- `bootstrap.rs` / `join/{flow,bootstrap,helpers,history,rejoin,state}.rs` — Join pipeline
- `channel_messages.rs` — Channel message ingest, sequence tracking, gap detection
- `channel_reactions.rs`, `channel_polls.rs` — Reactions, polls
- `threads.rs`, `threads_store.rs` — Forum threads
- `event_reminders.rs` — Event reminder scheduler
- `expressions.rs` — Custom emoji / sticker / soundboard
- `files.rs` — Lost Cargo file delivery integration
- `link_previews.rs` — OpenGraph preview broadcasts
- `notifications.rs`, `message_notifications.rs` — Notification dispatch
- `mentions.rs` — `@member` parsing
- `automod.rs` — AutoMod evaluation
- `raid_detection.rs` — Architecture §20.6 join-rate watcher
- `mek_rotation.rs`, `mek_rotation_support.rs` — MEK rotation pipeline
- `segments.rs` — Plate Gate segment expansion
- `stage.rs` — Stage channel (listener / speaker / hand-raise)
- `video.rs` — Video / screen-share fragmentation, reassembly, topology
- `analytics/` — Activity-by-hour, growth, member/channel metrics

### Voice

`services/voice/` runs the voice loops on dedicated tokio tasks bridged to
`cpal` threads:

- `send_loop.rs`, `receive_loop.rs` — Encode/decode and transport
- `mcu_loop.rs` — Group voice host's mix-and-relay loop
- `signaling.rs` — Voice signaling RPCs
- `device_monitor.rs` — Audio device hot-swap detection
- `session.rs` — VoiceSession state
- `shutdown.rs` — Graceful tear-down

### DMs, Relay, Search, Sync, Push Relay

- `services/dm/{accept,create,ingest,messages,store}.rs` — DM lifecycle
- `services/relay/{forward,offer,pool,presence,send}.rs` — Strand Relay
- `services/search/{context,query,messages,dm,threads}.rs` — FTS5 search
- `services/cross_device_sync/{merge,pairing,record,subkey_io,watch}.rs` — Multi-device
- `services/push_relay.rs` — Mobile push relay client

### Veilid Dispatch Loop

The dispatch loop is the central event router. It receives `VeilidUpdate`
variants and delegates:

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

## Plugin Registration

Only the `login` window is declared in `tauri.conf.json`. All other windows
are created at runtime by `src-tauri/src/windows.rs`. Plugins are registered
in `lib.rs` in dependency order:

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
- `tauri-plugin-stronghold` — replaced with direct `iota_stronghold` for
  per-identity files and configurable Argon2 params.
- `tauri-plugin-sql` — replaced with direct `rusqlite` for
  veilid-core compatibility (matched dep version).

### Global Shortcuts

Registered in `setup()` so the handler closes over `SharedState`.

| Combo | Action |
|-------|--------|
| `Ctrl+Shift+X` (`Cmd+Shift+X` on macOS) | Toggle buddy list visibility |
| `Ctrl+Shift+M` (`Cmd+Shift+M` on macOS) | Toggle voice mute |

## Window Management

Windows are created via helper functions in `src-tauri/src/windows.rs`:

| Function | Window | Behavior |
|----------|--------|----------|
| `open_login()` | Login | Destroys existing, supports `?account=` preselect |
| `open_buddy_list()` | Buddy list | Destroys existing, narrow vertical (320 x 650) |
| `open_chat_window()` | Chat | Show existing or create new, label `chat-{key prefix}` |
| `open_dm_window()` | DM | Show existing or create new, label `dm-{record key prefix}` |
| `open_community_window()` | Community | Show existing or create new, label `community-{id}` |
| `open_settings_window()` | Settings | Single instance |
| `open_profile_window()` | Profile | Show existing or create new, label `profile-{key prefix}` |

All windows use `decorations: false` and `transparent: true` for the
frameless Xfire look with custom titlebars.

The buddy list hides to tray on close (`api.prevent_close()`). Closing the
login window while no buddy list is visible triggers `app.exit(0)`. Other
windows close normally.

## System Tray

The system tray is configured in `tray.rs` with a context menu providing:
- Status controls (Online, Away, Busy, Invisible)
- Show/hide buddy list
- Quit application

## Deep Links

`rekindle://` URLs are handled in `deep_links.rs`. On macOS the URL is
delivered via `app.deep_link().on_open_url(...)` registered in `setup()`.
On Windows/Linux the OS re-launches the binary with the URL as an
argument, and the `single-instance` plugin callback routes it.

If a deep link arrives before the user is authenticated, it is buffered in
`AppState.pending_deep_link` and replayed on successful login.

## Graceful Shutdown

On `RunEvent::Exit`, the application performs an ordered shutdown with a
5-second timeout:

1. Clean up user DHT state (close records, release private route)
2. Signal dispatch loop shutdown
3. Signal sync / heartbeat / route-refresh / idle service shutdowns
4. Stop game detection
5. Stop voice engine (capture + playback + send/recv/MCU)
6. Shut down Veilid node

## Concurrency Patterns

- `parking_lot::RwLock` for read-heavy state (identity, friends, communities)
- `parking_lot::Mutex` for exclusive access (voice engine, game detector,
  MEK caches, dedup cache)
- Guards are `!Send` — clone data out before `.await` points
- `tokio_rusqlite::Connection` for the database pool (async wrapper over
  `rusqlite` on a dedicated background thread). Use the `db_helpers`
  helpers (`db_call`, `db_call_or_default`, `db_fire`)
- `tokio::sync::watch` for network readiness signaling
- `tokio::sync::mpsc` for shutdown channels
