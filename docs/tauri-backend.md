# Tauri Application Shell

The `src-tauri/` directory contains the Tauri 2 application shell that bridges
the pure Rust crates to the SolidJS frontend. It manages application state,
IPC commands, event emission, background services, window lifecycle, and
system tray integration.

## Application State

`AppState` is the central state container, shared across all commands and
services as `SharedState = Arc<AppState>`. Fields use `Arc<RwLock<T>>` for
read-heavy access and `Arc<Mutex<T>>` for exclusive access.

| Field | Type | Description |
|-------|------|-------------|
| `identity` | `Arc<RwLock<Option<IdentityState>>>` | Logged-in user's identity |
| `friends` | `Arc<RwLock<HashMap<String, FriendState>>>` | Friends with presence |
| `communities` | `Arc<RwLock<HashMap<String, CommunityState>>>` | Joined communities |
| `node` | `Arc<RwLock<Option<NodeHandle>>>` | Veilid node handle |
| `dht_manager` | `Arc<RwLock<Option<DHTManagerHandle>>>` | DHT record manager |
| `routing_manager` | `Arc<RwLock<Option<RoutingManagerHandle>>>` | Private route lifecycle |
| `signal_manager` | `Arc<Mutex<Option<SignalManagerHandle>>>` | Signal session manager |
| `game_detector` | `Arc<Mutex<Option<GameDetectorHandle>>>` | Game detection state |
| `voice_engine` | `Arc<Mutex<Option<VoiceEngineHandle>>>` | Voice engine state |
| `shutdown_tx` | `Arc<RwLock<Option<mpsc::Sender<()>>>>` | Dispatch loop shutdown |
| `sync_shutdown_tx` | `Arc<RwLock<Option<mpsc::Sender<()>>>>` | Sync service shutdown |
| `network_ready_tx/rx` | `watch::Sender<bool>` / `watch::Receiver<bool>` | DHT readiness flag |
| `identity_secret` | `Mutex<Option<[u8; 32]>>` | Ed25519 secret for signing |
| `background_handles` | `Mutex<Vec<JoinHandle<()>>>` | Spawned task handles |

`parking_lot` mutexes are used for synchronous access. Guards are `!Send` —
data must be cloned out before `.await` points.

## Command Modules

Commands are the Frontend → Rust IPC mechanism. Each is a `#[tauri::command]`
function registered in `lib.rs`.

### auth (6 commands)

| Command | Description |
|---------|-------------|
| `create_identity` | Generate Ed25519 keypair, create Stronghold, publish DHT profile |
| `login` | Unlock Stronghold, load identity, start background services |
| `get_identity` | Return current identity state |
| `logout` | Clean up DHT records, stop services, lock Stronghold |
| `list_identities` | List all identity files on disk |
| `delete_identity` | Remove identity from DB and delete Stronghold file |

### chat (4 commands)

| Command | Description |
|---------|-------------|
| `send_message` | Encrypt and send 1:1 message to peer |
| `send_typing` | Send typing indicator (ephemeral, not queued) |
| `get_message_history` | Query SQLite for conversation messages |
| `mark_read` | Mark messages as read for a conversation |

### friends (9 commands)

| Command | Description |
|---------|-------------|
| `add_friend` | Send friend request via Veilid |
| `remove_friend` | Remove friend from list and DHT |
| `accept_request` | Accept incoming friend request |
| `reject_request` | Reject incoming friend request |
| `get_friends` | Return full friend list with presence |
| `get_pending_requests` | Return incoming friend requests |
| `create_friend_group` | Create a new buddy list group |
| `rename_friend_group` | Rename an existing group |
| `move_friend_to_group` | Move friend to a different group |

### community (11 commands)

| Command | Description |
|---------|-------------|
| `create_community` | Create community, generate DHT records |
| `join_community` | Join by invite code |
| `create_channel` | Add text or voice channel |
| `send_channel_message` | Send to channel (plaintext — MEK integration pending) |
| `get_channel_messages` | Query channel message history |
| `get_communities` | List joined communities |
| `get_community_details` | Full community info |
| `get_community_members` | Member list with roles |
| `remove_community_member` | Remove member from local DB (no DHT propagation) |
| `update_member_role` | Change member's role locally (no DHT propagation) |
| `leave_community` | Leave and clean up local state |

### voice (4 commands)

| Command | Description |
|---------|-------------|
| `join_voice_channel` | Start capture, connect transport, spawn send loop |
| `leave_voice` | Stop capture/playback, disconnect transport |
| `set_mute` | Toggle microphone mute |
| `set_deafen` | Toggle audio output deafen |

### status (5 commands)

| Command | Description |
|---------|-------------|
| `set_status` | Update online/away/busy status, publish to DHT |
| `set_nickname` | Set display name |
| `set_avatar` | Upload avatar (WebP), publish to DHT |
| `get_avatar` | Retrieve avatar for a peer |
| `set_status_message` | Update status message text |

### game (1 command)

| Command | Description |
|---------|-------------|
| `get_game_status` | Return current detected game info |

### settings (3 commands)

| Command | Description |
|---------|-------------|
| `get_preferences` | Load preferences from Tauri Store |
| `set_preferences` | Save preferences to Tauri Store |
| `check_for_updates` | Stub — always returns false (updater not wired) |

### window (6 commands)

| Command | Description |
|---------|-------------|
| `show_buddy_list` | Open or focus buddy list window |
| `open_chat_window` | Open chat window for a specific peer |
| `open_settings_window` | Open settings window |
| `open_community_window` | Open community window |
| `open_profile_window` | Open profile viewer for a peer |
| `get_network_status` | Return current Veilid attachment state and DHT readiness |

## Event Types

Events are the Rust → Frontend push mechanism, emitted via `app.emit()` and
received via `listen()` on the frontend.

### ChatEvent (`chat-event`)

| Variant | Fields |
|---------|--------|
| `MessageReceived` | `from`, `body`, `timestamp`, `conversationId` |
| `TypingIndicator` | `from`, `typing` |
| `MessageAck` | `messageId` |
| `FriendRequest` | `from`, `displayName`, `message` |
| `FriendRequestAccepted` | `from`, `displayName` |
| `FriendRequestRejected` | `from` |
| `FriendAdded` | `publicKey`, `displayName` |

### PresenceEvent (`presence-event`)

| Variant | Fields |
|---------|--------|
| `FriendOnline` | `publicKey` |
| `FriendOffline` | `publicKey` |
| `StatusChanged` | `publicKey`, `status`, `statusMessage` |
| `GameChanged` | `publicKey`, `gameName: Option<String>`, `gameId: Option<u32>`, `elapsedSeconds: Option<u32>` |

### VoiceEvent (`voice-event`)

| Variant | Fields |
|---------|--------|
| `UserJoined` | `publicKey`, `displayName` |
| `UserLeft` | `publicKey` |
| `UserSpeaking` | `publicKey`, `speaking` |
| `UserMuted` | `publicKey`, `muted` |
| `ConnectionQuality` | `quality` |

### NotificationEvent (`notification-event`)

| Variant | Fields |
|---------|--------|
| `SystemAlert` | `title`, `body` |
| `UpdateAvailable` | `version` |

### NetworkStatusEvent (`network-status`)

Not a tagged enum. Flat struct pushed whenever Veilid connection state changes.

| Field | Type | Description |
|-------|------|-------------|
| `attachmentState` | `String` | Raw Veilid state (e.g., `attached_good`) |
| `isAttached` | `bool` | Whether node is attached |
| `publicInternetReady` | `bool` | Whether DHT operations are available |
| `hasRoute` | `bool` | Whether a private route is allocated |

All event enums use `#[serde(rename_all = "camelCase", tag = "type", content = "data")]`.

## Background Services

Six services run as spawned Tokio tasks after login.

| Service | File | Responsibility |
|---------|------|---------------|
| `veilid_service` | `veilid_service.rs` | Node lifecycle, Veilid update dispatch loop |
| `message_service` | `message_service.rs` | Process incoming `AppMessage` payloads |
| `presence_service` | `presence_service.rs` | Handle DHT `ValueChange` for friend presence |
| `sync_service` | `sync_service.rs` | Retry pending messages every 30s (max 20 retries) |
| `community_service` | `community_service.rs` | Sync community DHT records |
| `game_service` | `game_service.rs` | Periodic game detection, publish to DHT |

The `veilid_service` dispatch loop is the central event router. It receives
`VeilidUpdate` variants and delegates to the appropriate service:
- `AppMessage` → `message_service`
- `ValueChange` → `presence_service` (profile records) or `community_service`
- `Attachment` → update `NodeHandle` state, emit `NetworkStatusEvent`
- `RouteChange` → re-allocate private routes via `routing_manager`

## Plugin Registration

Plugins are registered in `lib.rs` in dependency order:

| Order | Plugin | Rationale |
|-------|--------|-----------|
| 1 | `single-instance` | Must be first — prevents duplicate processes |
| 2 | `notification` | System notifications for messages and alerts |
| 3 | `store` | Persistent user preferences |
| 4 | `process` | Process information (for updater) |
| 5 | `deep-link` | `rekindle://` URL scheme handling |
| 6 | `autostart` | Launch at system boot (LaunchAgent on macOS) |
| 7 | `global-shortcut` | Registered in `setup()` for state access |

Notable absences:
- `tauri-plugin-window-state` — removed due to infinite `windowDidMove` event loop on macOS
- `tauri-plugin-stronghold` — replaced with direct `iota_stronghold` usage for per-identity files

## Window Management

Windows are created via helper functions in `windows.rs`:

| Function | Window | Behavior |
|----------|--------|----------|
| `open_login()` | Login | Destroys existing, supports `?account=` preselect |
| `open_buddy_list()` | Buddy list | Destroys existing, narrow vertical (320x650) |
| `open_chat_window()` | Chat | Show existing or create new, label = `chat-{key prefix}` |

All windows use `decorations: false` and `transparent: true` for frameless
appearance with custom titlebars.

The buddy list hides to tray on close (`api.prevent_close()`). Closing the
login window while no buddy list is visible triggers `app.exit(0)`. Other
windows close normally.

## System Tray

The system tray is configured in `tray.rs` with a context menu providing:
- Status controls (Online, Away, Busy)
- Show/hide buddy list
- Quit application

## Graceful Shutdown

On `RunEvent::Exit`, the application performs an ordered shutdown with a
5-second timeout:

1. Clean up user DHT state (close records, release private route)
2. Signal dispatch loop shutdown
3. Signal sync service shutdown
4. Signal game detection shutdown
5. Stop voice engine (capture + playback)
6. Shut down Veilid node

## Concurrency Patterns

- `parking_lot::RwLock` for read-heavy state (identity, friends, node)
- `parking_lot::Mutex` for exclusive access (voice engine, game detector)
- Guards are `!Send` — clone data out before `.await` points
- `std::sync::Mutex` for `DbPool` (used with `spawn_blocking`)
- `tokio::sync::watch` for network readiness signaling
- `tokio::sync::mpsc` for shutdown channels
