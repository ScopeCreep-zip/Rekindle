# Phase H Audit: Gaming Community Features (Discord Parity)

## Approach

This plan targets **Discord parity**, not legacy Xfire. Discord's 2025/2026 game features are:

1. **Game Detection** — process-name scanning against a ~1,800 game database (undocumented `GET /api/v10/applications/detectable`). No hooks, no hashing, just executable name matching.
2. **Rich Presence** — games push structured activity data (`state`, `details`, `timestamps`, `party`, `assets`, `secrets`) over a local IPC socket (`discord-ipc-{0..9}`). The Discord Social SDK (March 2025, replaces deprecated Game SDK) wraps this.
3. **Join Game** — games publish a `joinSecret` in presence; clicking "Join" on a profile dispatches `ACTIVITY_JOIN` back to the running game, which handles the connection.
4. **Scheduled Events** — `SCHEDULED → ACTIVE → COMPLETED/CANCELED` lifecycle, three entity types (Voice, Stage, External), "Interested" RSVP, recurrence rules, max 100 per guild.
5. **No "Game Server Favorites"** — Discord doesn't have this. The equivalent is the "Join" button on profiles when a game publishes a `joinSecret`.

**What Rekindle already has that maps to Discord:**
- Game detection via process scanning + `default_games.json` (51 games) → matches Discord's approach exactly
- `RichPresence` struct with server_ip/port/map/player_count → maps to Discord's Activity fields
- `GameInfo` in presence DHT subkey 4 → maps to Discord's Activity status display
- Community events with RSVP → maps to Discord's Scheduled Events (missing lifecycle states + recurrence)

**What's broken or missing:**
- `server_address` is always `None` in game detection — presence data pipeline is incomplete
- `GameServerList` component exists but is **never rendered** (no navigation path)
- No "Join Game" button anywhere
- Community members don't receive game presence updates (handler gap)
- Events lack Discord's lifecycle states and recurrence
- `rpc/events.rs` is a kitchen-sink module mixing audit/typing/presence/events

---

## Issue 1: Game Presence Data Pipeline Is Broken

**Discord equivalent:** When you're playing CS2 on a server, Discord shows "Playing Counter-Strike 2 — Competitive on de_dust2" with a Join button.

**Our state:** `game_service.rs:44-46` hardcodes `server_info: None` and `server_address: None`. The `RichPresence` struct exists but is never populated. Community members never see game status updates.

### Fix 1a: Wire `RichPresence` into `DetectedGame`

The scanner already detects games by process name. On Linux, `platform::linux::read_cmdline(pid)` already reads `/proc/{pid}/cmdline`. Many games (Source engine, Quake, etc.) include `+connect ip:port` in launch args. This is the simplest cross-platform approach that matches Discord's level of game awareness.

**Files to modify:**

`crates/rekindle-game-detect/src/scanner.rs` — Add `rich_presence` field to `DetectedGame`, populate from cmdline:
```rust
// Current:
pub struct DetectedGame {
    pub game_id: u32,
    pub game_name: String,
    pub process_name: String,
    pub started_at_epoch_ms: u64,
}

// After:
pub struct DetectedGame {
    pub game_id: u32,
    pub game_name: String,
    pub process_name: String,
    pub started_at_epoch_ms: u64,
    pub rich_presence: Option<RichPresence>,
}
```

In `scan_once()`, after matching a game, call a new helper to extract presence from the process. The scanner already has `self.system` with refreshed process data — iterate to find the matched PID, then extract cmdline args.

`crates/rekindle-game-detect/src/scanner.rs` — New private helper `extract_rich_presence()`:
- Gets the matched process's PID from `self.system.processes()`
- Calls platform-specific cmdline reader
- Parses for known connect patterns: `+connect`, `-connect`, `--connect`, `connect://`
- Returns `Option<RichPresence>` with `server_ip` and `server_port` populated

`crates/rekindle-game-detect/src/platform/mod.rs` — New public function `read_process_cmdline(pid: u32) -> Option<Vec<String>>`:
- Linux: already exists as `linux::read_cmdline(pid)` — re-export it
- macOS: use `sysinfo::Process::cmd()` (sysinfo 0.32 exposes this cross-platform)
- Windows: use `sysinfo::Process::cmd()` (same)
- This replaces the need for 3 separate platform implementations

Actually, `sysinfo::Process` already provides `.cmd()` which returns `&[String]` — the command-line arguments. The scanner already has a refreshed `System` with all processes. No platform-specific code needed.

**Revised approach (simpler):** In `scan_once()`, when a game is matched by process name, look up the `sysinfo::Process` from `self.system`, call `.cmd()` to get args, pass to a `parse_connect_args(cmd: &[String]) -> Option<(String, u16)>` helper in `rich_presence.rs`.

`crates/rekindle-game-detect/src/rich_presence.rs` — New function:
```rust
/// Parse server connection info from game process command-line arguments.
/// Recognizes patterns: +connect ip:port, -connect ip:port, --server ip:port
pub fn parse_connect_args(args: &[String]) -> Option<(String, u16)> {
    // Iterate args looking for connect flags, parse the following arg as ip:port
}
```

This keeps platform code untouched and uses sysinfo's cross-platform `.cmd()`.

`src-tauri/src/services/game_service.rs:41-47` — Wire the new field:
```rust
// Current (always None):
let game_info = detected.as_ref().map(|g| GameInfoState {
    server_info: None,
    server_address: None,
    ...
});

// After (populated from detection):
let game_info = detected.as_ref().map(|g| GameInfoState {
    server_info: g.rich_presence.as_ref().and_then(|rp| rp.details.clone()),
    server_address: g.rich_presence.as_ref().and_then(|rp| rp.server_address()),
    ...
});
```

### Fix 1b: Auto-publish community presence on game change

**Discord equivalent:** When your game status changes, all servers you're in see it instantly.

**Our state:** `game_service.rs` publishes to DHT profile subkey 4 (friends see it) but does NOT call `UpdatePresence` for any joined communities. Community members' game status is never updated.

**Two gaps:**
1. The game service doesn't forward to communities
2. `subscribeCommunityPresenceEvents()` in `presence-events.handlers.ts` doesn't handle `gameChanged` for community members

**Fix — backend approach** (keeps it in one place, avoids frontend round-trip):

`src-tauri/src/services/game_service.rs` — After the DHT publish block (~line 69), add community presence fan-out:
```rust
// After publishing to DHT...
// Fan out to all joined communities
let community_ids: Vec<String> = {
    let communities = state.communities.read();
    communities.keys().cloned().collect()
};
for community_id in community_ids {
    let request = CommunityRequest::UpdatePresence {
        status: if game_info.is_some() { "online".into() } else { state_helpers::current_status_string(&state) },
        game_name: game_info.as_ref().map(|g| g.game_name.clone()),
        game_id: game_info.as_ref().map(|g| g.game_id),
        elapsed_seconds: game_info.as_ref().map(|g| g.elapsed_seconds),
        server_address: game_info.as_ref().and_then(|g| g.server_address.clone()),
    };
    // Fire-and-forget — don't block detection loop on RPC responses
    let s = Arc::clone(&state);
    let pool = pool_handle.clone();
    tokio::spawn(async move {
        let _ = send_community_rpc(&s, &pool, &community_id, request).await;
    });
}
```

This requires `game_service.rs` to take a `DbPool` handle (currently it only takes `AppHandle` and `Arc<AppState>`). Add `pool: DbPool` parameter to `start_game_detection()`.

`src-tauri/src/lib.rs` — Update the `start_game_detection()` call site to pass pool.

`src/handlers/presence-events.handlers.ts` — The `memberPresenceChanged` community event already works (it's in `community.handlers.ts`, not `presence-events.handlers.ts`). The server broadcasts `MemberPresenceChanged` which the frontend already handles. No frontend change needed — once the backend sends `UpdatePresence`, the server broadcasts it and other members receive it through the existing `CommunityEvent` dispatch.

### Fix 1c: Community member game display

**Discord equivalent:** Server member list shows game name under each member.

`src/components/community/MemberList.tsx` — Already shows `member.gameInfo.gameName`. Verify it also shows `serverAddress` when present. Add the server address display:
```tsx
<Show when={info().serverAddress}>
  <span class="member-game-server">on {info().serverAddress}</span>
</Show>
```

`src/styles/xfire-theme.css` — Add `.member-game-server` class (10px, monospace, dim color).

---

## Issue 2: No "Join Game" Flow

**Discord equivalent:** Profile shows "Join" button when `joinSecret` is in presence. Clicking it dispatches to the running game which handles the connection.

**Our approach:** We don't have an IPC socket to the running game. Instead, we launch the game with connect args (like `steam://connect/ip:port`) or spawn the process directly. This is what Discord does on the *receiving* side when a non-running game needs to be launched.

### Fix 2a: Game launch capability

`crates/rekindle-game-detect/src/database.rs` — Extend `GameEntry`:
```rust
pub struct GameEntry {
    pub id: u32,
    pub name: String,
    pub process_names: Vec<String>,
    pub icon: Option<String>,
    pub steam_app_id: Option<u32>,          // NEW
    pub connect_template: Option<String>,   // NEW: e.g., "steam://connect/{addr}"
}
```

`crates/rekindle-game-detect/src/default_games.json` — Add launch metadata to entries that support it:
```json
{"id": 4181, "name": "Counter-Strike 2", "process_names": ["cs2.exe", "cs2"],
 "icon": "cs2", "steam_app_id": 730, "connect_template": "steam://connect/{addr}"}
```

Most popular multiplayer games use `steam://connect/{addr}`. Non-Steam games get `null` — we can't launch what we don't know how to.

`crates/rekindle-game-detect/src/launcher.rs` — NEW module:
```rust
use crate::database::GameDatabase;
use crate::error::GameDetectError;

/// Launch a game and connect to the given server address.
///
/// Uses the game's `connect_template` from the database. Falls back to
/// `steam://connect/{addr}` if the game has a `steam_app_id`.
pub fn launch_to_server(
    db: &GameDatabase,
    game_id: u32,
    server_address: &str,
) -> Result<(), GameDetectError> {
    let entry = db.lookup_by_id(game_id)
        .ok_or_else(|| GameDetectError::DatabaseError("unknown game_id".into()))?;

    let url = if let Some(ref template) = entry.connect_template {
        template.replace("{addr}", server_address)
    } else if let Some(steam_id) = entry.steam_app_id {
        format!("steam://connect/{server_address}")
    } else {
        return Err(GameDetectError::DatabaseError("no launch method for this game".into()));
    };

    open::that(&url).map_err(|e| GameDetectError::Io(e))
}
```

Uses the `open` crate (already in Tauri's dependency tree) to invoke `steam://connect/` URIs cross-platform. Steam handles the rest — launching the game, connecting to the server.

`crates/rekindle-game-detect/src/database.rs` — Add `lookup_by_id()` method:
```rust
pub fn lookup_by_id(&self, game_id: u32) -> Option<&GameEntry> {
    self.by_process.values().find(|e| e.id == game_id)
}
```

`crates/rekindle-game-detect/src/lib.rs` — Add module:
```rust
pub mod launcher;
```

`crates/rekindle-game-detect/Cargo.toml` — Add dependency:
```toml
open = "5"
```

### Fix 2b: Tauri command for game launch

`src-tauri/src/commands/game.rs` — Add command:
```rust
#[tauri::command]
pub async fn launch_game_to_server(
    game_id: u32,
    server_address: String,
) -> Result<(), String> {
    let db = rekindle_game_detect::GameDatabase::bundled();
    rekindle_game_detect::launcher::launch_to_server(&db, game_id, &server_address)
        .map_err(|e| e.to_string())
}
```

`src-tauri/src/lib.rs` — Register in `generate_handler![]`.

`src/ipc/commands.ts` — Add:
```typescript
launchGameToServer: (gameId: number, address: string) =>
  invoke<void>("launch_game_to_server", { gameId, address }),
```

### Fix 2c: "Join Game" button in MemberProfilePopup

**Discord equivalent:** Profile card shows "Join Game" button when friend has a joinable activity.

`src/components/community/MemberProfilePopup.tsx:59-67` — Add button after server address:
```tsx
<Show when={props.member.gameInfo}>
  {(info) => (
    <div class="profile-popup-game">
      <span class="profile-popup-game-name">{info().gameName}</span>
      <Show when={info().serverAddress}>
        <span class="profile-popup-game-server">{info().serverAddress}</span>
        <button
          class="profile-popup-join-btn"
          onClick={() => commands.launchGameToServer(info().gameId!, info().serverAddress!)}
        >
          Join Game
        </button>
      </Show>
    </div>
  )}
</Show>
```

`src/styles/xfire-theme.css` — Add `.profile-popup-join-btn` (small green button matching Discord's Join style).

`src/components/buddy-list/BuddyItem.tsx` — Also add "Join Game" to friend context menu when `serverAddress` is present. This matches Discord where you can join from the friends list.

---

## Issue 3: GameServerList Is Disconnected From UI

**Discord context:** Discord doesn't have "game server favorites." This is a Rekindle/Xfire-unique feature. However, the component is fully built but never rendered. We should wire it up.

### Fix 3a: Add navigation to GameServerList

`src/windows/CommunityWindow.tsx` — Add a "Servers" toggle button alongside the existing "Events" button in the channel header area (near line 335 where `ICON_CALENDAR` toggle exists):
```tsx
<button
  class="community-header-btn"
  onClick={() => setShowServers(s => !s)}
  title="Game Servers"
>
  <span class="nf-icon">{ICON_SERVER}</span>
</button>
```

Add signal: `const [showServers, setShowServers] = createSignal(false);`

In the main content area (line ~395), add conditional rendering:
```tsx
<Show when={showServers()}>
  <GameServerList
    servers={gameServers()}
    communityId={selectedCommunityId()!}
    canManage={canManageCommunity()}
    onRemove={handleRemoveGameServer}
  />
</Show>
```

Import `GameServerList` and wire `handleLoadGameServers` on community select.

### Fix 3b: Add "Add Server" form to GameServerList

`src/components/community/GameServerList.tsx` — Add an inline "Add Server" form at the bottom when `canManage` is true:
```tsx
<Show when={props.canManage}>
  <div class="game-server-add-form">
    <input class="form-input" placeholder="Game name or ID" ... />
    <input class="form-input" placeholder="Server label" ... />
    <input class="form-input" placeholder="ip:port" ... />
    <button class="form-btn-primary" onClick={handleAdd}>Add</button>
  </div>
</Show>
```

Wire to existing `handleAddGameServer()` handler.

`src/styles/xfire-theme.css` — Add `.game-server-add-form` class (flex row, gap, bottom border).

### Fix 3c: Add "Join" button per server row

`src/components/community/GameServerList.tsx` — In each `game-server-row`, add a Join button:
```tsx
<button
  class="game-server-join"
  onClick={() => commands.launchGameToServer(parseInt(server.gameId), server.address)}
  title="Connect to server"
>
  Join
</button>
```

`src/styles/xfire-theme.css` — `.game-server-join` already exists in the CSS class family (defined in community-features-plan.md).

---

## Issue 4: Events Lack Discord Lifecycle + Permissions

**Discord equivalent:** Events have `SCHEDULED → ACTIVE → COMPLETED/CANCELED` states. `MANAGE_EVENTS` permission. Auto-activate at start time. Auto-complete at end time. Max 100 per guild.

### Fix 4a: Add `MANAGE_EVENTS` permission bit

`crates/rekindle-protocol/src/dht/community.rs` — Add constant:
```rust
pub const MANAGE_EVENTS: u64 = 1 << 14;  // next available bit after existing constants
```

`crates/rekindle-server/src/community_host.rs` — In `default_roles()`, add `MANAGE_EVENTS` to Moderator role permissions (alongside existing `KICK_MEMBERS` etc.).

`crates/rekindle-server/src/rpc/events.rs:218,293` — Change `permissions::MANAGE_COMMUNITY` to `permissions::MANAGE_EVENTS` in `handle_create_event` and `handle_edit_event`.

### Fix 4b: Event lifecycle states

**Discord model:** `SCHEDULED(1) → ACTIVE(2) → COMPLETED(3)` or `SCHEDULED(1) → CANCELED(4)`.

`crates/rekindle-protocol/src/messaging/envelope.rs` — Add `status` field to `EventDto`:
```rust
pub struct EventDto {
    // ... existing fields ...
    pub status: String,  // "scheduled", "active", "completed", "canceled"
}
```

`crates/rekindle-server/src/db.rs` — Add `status TEXT NOT NULL DEFAULT 'scheduled'` to `server_events` table. Bump `SERVER_SCHEMA_VERSION`.

`crates/rekindle-server/src/tasks.rs` — Extend `check_event_reminders()` to also:
1. Auto-activate events whose `start_time` has passed and status is "scheduled"
2. Auto-complete events whose `end_time` has passed and status is "active"
3. Auto-cancel events that are still "scheduled" 1 hour after their `start_time`

This matches Discord's behavior for External events.

`crates/rekindle-server/src/rpc/events.rs` — Add `handle_cancel_event` handler (sets status to "canceled"). Wire `CommunityRequest::CancelEvent { event_id }` variant.

`src/stores/community.store.ts` — Add `status` to `CommunityEvent` interface.

`src/components/community/EventsPanel.tsx` — Show status badge on event cards. Filter/sort: active first, then scheduled, hide completed/canceled by default.

### Fix 4c: Event audit logging (consistency fix)

`crates/rekindle-server/src/rpc/events.rs` — Add `audit::log_action()` calls in `handle_create_event`, `handle_edit_event`, `handle_delete_event` (game_servers.rs already does this, events.rs doesn't — inconsistent).

`crates/rekindle-server/src/audit.rs` — Add `CreateEvent`, `EditEvent`, `DeleteEvent`, `CancelEvent` to `AuditAction` enum.

### Fix 4d: Auto-cleanup past events

`crates/rekindle-server/src/tasks.rs` — New function `auto_cleanup_past_events(state)`:
- Delete events with status "completed" or "canceled" that are older than 7 days
- Run alongside the existing reminder check every 300s

`crates/rekindle-server/src/main.rs` — Call in the reminder task block (~line 199).

---

## Issue 5: `rpc/events.rs` Is a Kitchen-Sink Module

**Problem:** 656 lines mixing 3 unrelated concerns: audit log queries, typing/presence forwarding, and event CRUD. The module name "events" is ambiguous. This violates the refactoring goal of reducing inline function sprawl.

### Fix: Split into focused modules

Create two new files and shrink `events.rs`:

`crates/rekindle-server/src/rpc/audit_log.rs` — NEW (move `handle_get_audit_log` here):
```rust
// Move from events.rs: handle_get_audit_log (lines 18-104)
pub(super) fn handle_get_audit_log(...) -> CommunityResponse { ... }
```

`crates/rekindle-server/src/rpc/presence.rs` — NEW (move typing + presence here):
```rust
// Move from events.rs: handle_channel_typing (lines 110-143)
// Move from events.rs: handle_update_presence (lines 145-189)
pub(super) fn handle_channel_typing(...) -> CommunityResponse { ... }
pub(super) fn handle_update_presence(...) -> CommunityResponse { ... }
```

`crates/rekindle-server/src/rpc/events.rs` — Retains only event CRUD (handle_create_event, handle_edit_event, handle_delete_event, handle_rsvp_event, handle_get_events, load helpers). ~460 lines → focused on one concern.

`crates/rekindle-server/src/rpc/mod.rs` — Update module declarations and dispatch:
```rust
mod audit_log;   // NEW
mod events;      // slimmed
mod presence;    // NEW

// In dispatch_request():
CommunityRequest::GetAuditLog { .. } => audit_log::handle_get_audit_log(...)
CommunityRequest::ChannelTyping { .. } => presence::handle_channel_typing(...)
CommunityRequest::UpdatePresence { .. } => presence::handle_update_presence(...)
```

---

## Implementation Order

Each step is independently shippable and testable.

### Step 1: Split events.rs (Issue 5)
Low risk refactor, clears the path for clean event lifecycle changes.
- Create `rpc/audit_log.rs`, `rpc/presence.rs`
- Move functions, update `rpc/mod.rs` dispatch
- `cargo clippy --workspace` to verify

### Step 2: Wire rich presence from game detection (Issue 1a)
Core data pipeline fix.
- Add `rich_presence` to `DetectedGame`
- Add `parse_connect_args()` to `rich_presence.rs`
- Use `sysinfo::Process::cmd()` in scanner
- Wire into `game_service.rs` `GameInfoState`
- `cargo test --workspace`

### Step 3: Community presence fan-out (Issue 1b + 1c)
Makes game status visible in communities.
- Add community RPC fan-out in `game_service.rs`
- Pass `DbPool` to `start_game_detection()`
- Add server address display in `MemberList.tsx`
- Manual test: detect a game, verify community members see it

### Step 4: Game launch + Join buttons (Issue 2)
The Discord "Join" equivalent.
- Create `launcher.rs` module
- Add `steam_app_id` + `connect_template` to `GameEntry` + `default_games.json`
- Add `lookup_by_id()` to `GameDatabase`
- Add `launch_game_to_server` Tauri command
- Add "Join Game" button to `MemberProfilePopup.tsx`
- Add "Join" to `BuddyItem.tsx` context menu
- Manual test: click Join, verify Steam launches

### Step 5: Wire GameServerList into UI (Issue 3)
Connects the disconnected component.
- Add toggle button in `CommunityWindow.tsx`
- Add "Add Server" form to `GameServerList.tsx`
- Add "Join" button per server row
- Load servers on community select

### Step 6: Event lifecycle + permissions (Issue 4)
Discord parity for scheduled events.
- Add `MANAGE_EVENTS` permission bit
- Add `status` field to events (schema bump)
- Auto-activate/complete/cancel in tasks.rs
- Add audit logging for event operations
- Add `CancelEvent` request variant
- Update `EventsPanel.tsx` with status display
- Add past-event cleanup task

---

## Files Changed Summary

### New Files (4)
| File | Purpose |
|------|---------|
| `crates/rekindle-game-detect/src/launcher.rs` | Game launch via `steam://connect` or `open` crate |
| `crates/rekindle-server/src/rpc/audit_log.rs` | `handle_get_audit_log` (extracted from events.rs) |
| `crates/rekindle-server/src/rpc/presence.rs` | `handle_channel_typing` + `handle_update_presence` (extracted) |
| *(none in frontend — all changes to existing files)* | |

### Modified Files — Backend (12)
| File | Change |
|------|--------|
| `crates/rekindle-game-detect/src/scanner.rs` | Add `rich_presence` to `DetectedGame`, extract from `Process::cmd()` |
| `crates/rekindle-game-detect/src/rich_presence.rs` | Add `parse_connect_args()` function |
| `crates/rekindle-game-detect/src/database.rs` | Add `steam_app_id`, `connect_template` to `GameEntry`, add `lookup_by_id()` |
| `crates/rekindle-game-detect/src/default_games.json` | Add `steam_app_id` + `connect_template` to multiplayer entries |
| `crates/rekindle-game-detect/src/lib.rs` | Add `pub mod launcher;` |
| `crates/rekindle-game-detect/Cargo.toml` | Add `open = "5"` dependency |
| `crates/rekindle-server/src/rpc/mod.rs` | Add `mod audit_log; mod presence;`, update dispatch |
| `crates/rekindle-server/src/rpc/events.rs` | Remove audit/typing/presence handlers, add lifecycle, audit calls |
| `crates/rekindle-server/src/tasks.rs` | Add event auto-activate/complete/cancel/cleanup |
| `crates/rekindle-server/src/audit.rs` | Add event audit action variants |
| `crates/rekindle-protocol/src/dht/community.rs` | Add `MANAGE_EVENTS` permission constant |
| `crates/rekindle-protocol/src/messaging/envelope.rs` | Add `status` to `EventDto`, add `CancelEvent` variant |

### Modified Files — Tauri Shell (4)
| File | Change |
|------|--------|
| `src-tauri/src/services/game_service.rs` | Wire `rich_presence` → `GameInfoState`, add community fan-out |
| `src-tauri/src/commands/game.rs` | Add `launch_game_to_server` command |
| `src-tauri/src/lib.rs` | Register new command, pass pool to game service |
| `src-tauri/src/commands/community.rs` | Add `cancel_event` command if needed |

### Modified Files — Frontend (8)
| File | Change |
|------|--------|
| `src/components/community/MemberProfilePopup.tsx` | Add "Join Game" button |
| `src/components/community/MemberList.tsx` | Show server address next to game name |
| `src/components/community/GameServerList.tsx` | Add "Join" button per row, "Add Server" form |
| `src/components/community/EventsPanel.tsx` | Event status badges, filter by status |
| `src/components/buddy-list/BuddyItem.tsx` | Add "Join Game" to context menu |
| `src/windows/CommunityWindow.tsx` | Add Servers toggle, render GameServerList |
| `src/ipc/commands.ts` | Add `launchGameToServer`, `cancelEvent` |
| `src/styles/xfire-theme.css` | Add `.profile-popup-join-btn`, `.member-game-server`, `.game-server-add-form` |

### Modified Files — Stores/Types (2)
| File | Change |
|------|--------|
| `src/stores/community.store.ts` | Add `status` to `CommunityEvent` |
| `src/handlers/community.handlers.ts` | Add `handleCancelEvent` handler |

**Total: 4 new files, 26 modified files across 6 steps.**

---

## What This Does NOT Include (Future Work)

- **Discord IPC bridge** — Reading games' Rich Presence from Discord's IPC socket to import richer activity data. Would require implementing the Discord RPC handshake protocol. Rust crates exist (`discord-rich-presence`, `rpresence`). Stretch goal.
- **Live server querying** — Using `a2s` or `gamedig` crate to show player count/map on game servers. Enhancement, not parity.
- **Event recurrence** — Discord supports weekly/monthly recurrence via iCalendar subset. Can be added later.
- **Event images** — Discord allows event cover images. Low priority.
- **Platform network inspection** — Reading game process TCP connections to auto-detect servers (like Xfire's LSP). Platform-specific and complex. The cmdline parsing approach covers the common case.
