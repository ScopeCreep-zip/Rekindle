# Game Detection

Game detection is the feature that powers Xfire's signature behaviour:
your friends can see what you're playing in real time on their buddy
list, and they can join your server with one click. Rekindle reproduces
this end-to-end without any centralised game database service.

The implementation lives in **`crates/rekindle-game-detect/`** plus the
Tauri-side scan loop in **`src-tauri/src/services/game_service.rs`** and
the IPC commands in **`src-tauri/src/commands/game.rs`**.

## What it does

```
┌────────────────────────────────────────────────────────┐
│  game_service.rs (Tauri background task)                │
│                                                         │
│   loop every N seconds:                                 │
│     ▶ GameDetector::scan_once()                        │
│         ├── sysinfo::System.refresh_processes()        │
│         ├── for each process:                          │
│         │     match name against GameDatabase          │
│         └── return Option<DetectedGame>                │
│                                                         │
│     if game changed:                                   │
│       ▶ update presence record (rich_presence)         │
│       ▶ emit("presence-event")                         │
└────────────────────────────────────────────────────────┘
```

Each iteration:

1. **Refreshes the process list** via the cross-platform `sysinfo` crate.
   `sysinfo` uses `/proc` enumeration on Linux, `proc_listpids` on macOS,
   and `CreateToolhelp32Snapshot` on Windows.
2. **Looks up** each running process name against the bundled game
   database (case-insensitive, lowercase keys).
3. **Extracts rich presence** — server IP/port, map name, mode, player
   count — from the matched game's command-line arguments where
   available.
4. **Publishes a `PresenceUpdate`** when the current-game state
   transitions (start playing, stop playing, server change). The update
   is written to the user's DHT presence record and gossiped to
   subscribed friends.

Friends watching the user's presence record see the change within
60 seconds (DHT watch / inspect cycle) or instantly (gossip path). See
[`../protocol/overview.md`](../protocol/overview.md) for the presence
protocol.

## The game database

```
crates/rekindle-game-detect/src/default_games.json
```

JSON file with a flat array of `GameEntry` records:

```rust
struct GameEntry {
    id: u32,                          // Stable Xfire-style game ID
    name: String,                     // Display name
    process_names: Vec<String>,       // Match against running processes
    icon: Option<String>,             // Icon ref (matches assets in icons.dll)
    steam_app_id: Option<u32>,        // For steam://connect/ URI
    connect_template: Option<String>, // URI template for non-Steam launchers
}
```

### Origin

The original Xfire shipped a 2 MB `xfire_games.ini` with thousands of
entries. Rekindle ships a curated JSON database of the most-played
modern titles (CS2, Dota 2, League, Valorant, Fortnite, Minecraft,
WoW, GTA V, Elden Ring, Rocket League, …). The database is loaded with
`include_str!` and parsed once at startup:

```rust
let db = GameDatabase::bundled();   // ships with the binary
let detector = GameDetector::new(db, Duration::from_secs(5));
```

### Extending it

Users can supplement the bundled DB with their own JSON file (path is
configurable in settings). Custom entries override bundled ones with the
same `id`. The database is in-process — no network calls are made to
fetch or update game definitions. This is a deliberate privacy choice:
querying a remote game-recognition service would expose what the user
is playing to a third party.

## Cross-platform process enumeration

The cross-platform `sysinfo` API does most of the work, but each
platform has a small specialisation file for richer detection:

| Platform | File | Adds |
|----------|------|------|
| Linux | `platform/linux.rs` | `/proc/<pid>/exe` symlink resolution (more reliable than process name for Wine/Proton games), `/proc/<pid>/cmdline` reading for full argv |
| macOS | `platform/macos.rs` | Process executable path resolution, `.app` bundle detection (`Name.app/Contents/MacOS/`), bundle-name extraction |
| Windows | `platform/windows.rs` | Process executable path lookup, list-with-paths helper |

The Linux specialisation is the most useful — many games on Linux run
through Wine or Proton, which presents a non-native process name. The
`/proc/<pid>/exe` resolver gets the actual binary path so the matcher
can disambiguate.

## Rich presence

```rust
struct RichPresence {
    game_id: u32,
    details:      Option<String>,    // freeform "what they're doing"
    state:        Option<String>,    // freeform "where they are"
    server_ip:    Option<String>,
    server_port:  Option<u16>,
    map_name:     Option<String>,
    player_count: Option<u32>,
    max_players:  Option<u32>,
}
```

For supported games, the scanner extracts these fields from the running
process's command-line arguments (`+connect 1.2.3.4:27015` for Source
games, Steam launch arguments, etc.). Less-supported games get
`RichPresence::basic(game_id)` — just the ID, with display name resolved
client-side from the database.

The combined server address (`server_ip:server_port`) is what powers the
"Join Game" button on a friend's buddy list entry.

## Joining a friend's server

```
┌────────────────┐                ┌────────────────────┐
│ Friend's buddy │  click "Join"  │ commands::game::    │
│ list entry     │ ────────────▶  │ launch_game_to_     │
│ (rich presence │                │ server              │
│  available)    │                └─────────┬──────────┘
└────────────────┘                          │
                                            ▼
                                  ┌──────────────────────┐
                                  │ launcher::launch_    │
                                  │ to_server(db,        │
                                  │   game_id, addr)     │
                                  └──────────┬───────────┘
                                             │
                          ┌──────────────────┼──────────────────┐
                          ▼                  ▼                  ▼
              connect_template?      steam_app_id?         neither
              "scheme://join/{addr}" "steam://connect/    Error: no
                                       {addr}"             launch method
                          │                  │
                          └──────────┬───────┘
                                     ▼
                          open::that(url)  → OS handles URI scheme
```

The `open` crate dispatches the URI to the OS default handler. For
Steam URIs, Steam intercepts and joins the game. For custom schemes,
whatever app is registered handles it. No `Process::spawn` of game
binaries — the launcher integration is purely URI-scheme based, which
keeps Rekindle out of the way of anti-cheat systems.

## Privacy posture

Game detection is **opt-in per identity**. Disabling it stops the scan
loop and removes the `game_info` field from the user's presence
writes. The user can also restrict which friends see their game status
via the standard friend-group privacy controls (groups marked "limit
visibility" do not receive `PresenceUpdate` gossips with `game_info`).

The database is **bundled** — no network requests are made to identify
games. The scanner runs **locally** — no process list ever leaves the
machine. The only thing that goes over the wire is the matched
`{game_id, server, map, …}` tuple, encrypted to the user's friends.

## Where to look

| Concern | File |
|---------|------|
| `GameDetector` scan API | `crates/rekindle-game-detect/src/scanner.rs` |
| `GameDatabase` (process-name → entry) | `crates/rekindle-game-detect/src/database.rs` |
| Bundled game definitions | `crates/rekindle-game-detect/src/default_games.json` |
| `RichPresence` struct + helpers | `crates/rekindle-game-detect/src/rich_presence.rs` |
| URI-scheme launcher | `crates/rekindle-game-detect/src/launcher.rs` |
| Linux `/proc` extras | `crates/rekindle-game-detect/src/platform/linux.rs` |
| macOS `.app` bundle helpers | `crates/rekindle-game-detect/src/platform/macos.rs` |
| Windows path lookup | `crates/rekindle-game-detect/src/platform/windows.rs` |
| Background scan loop | `src-tauri/src/services/game_service.rs` |
| IPC commands | `src-tauri/src/commands/game.rs` |
| Frontend store / display | `src/stores/`, friend list rich-presence rendering |

## Open work

- Game-time tracking (elapsed minutes per game per identity, persisted
  in SQLite). Skeleton exists in `rich_presence.rs::started_at_epoch_ms`;
  not yet aggregated.
- Server info polling for games that expose a query protocol (Source
  Engine, Quake, Minecraft) — would populate `map_name`, `player_count`
  without parsing argv.
- Per-friend-group game-visibility filters in the presence service.

These are tracked in [`../roadmap.md`](../roadmap.md).
