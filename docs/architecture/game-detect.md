# Game Detection

Game detection is the feature that powers Xfire's signature behaviour:
your friends can see what you're playing in real time on their buddy
list, and they can join your server with one click. Rekindle reproduces
this end-to-end without any centralised game database service.

The implementation lives in **`crates/rekindle-game-detect/`** plus the
Tauri-side scan loop in **`src-tauri/src/services/game_service.rs`** and
the IPC commands in **`src-tauri/src/commands/game.rs`**.

## What it does

```text
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

```text
crates/rekindle-game-detect/src/default_games.json
```

JSON file with a flat array of `GameEntry` records:

```rust
struct GameEntry {
    id: u32,                          // Stable Xfire-style game ID
    name: String,                     // Display name
    process_names: Vec<String>,       // Match against running processes
    icon: Option<String>,             // Icon ref (matches assets in icons.dll)
    steam_app_id: Option<u32>,        // With network_args: steam://run/<id>//<args>/
    connect_template: Option<String>, // URI template, tried first ("{addr}")
    network_args: Option<String>,     // Xfire's LauncherNetworkArgs ("+connect {addr}")
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

```text
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
              connect_template?      steam_app_id +        neither
              "scheme://join/{addr}" network_args?         CannotLaunch
                                     "steam://run/<id>//
                                      <args>/"
                          │                  │
                          └──────────┬───────┘
                                     ▼
                          open::that(url)  → OS handles URI scheme
```

There is no blanket `steam://connect` fallback: a Steam game reaches a server
only through its `connect_template` or, failing that, its Steam app id with
Xfire's `LauncherNetworkArgs` (`network_args`, percent-encoded into the
`steam://run` command line). A game with neither is `CannotLaunch`.

The `open` crate dispatches the URI to the OS default handler. For
Steam URIs, Steam intercepts and joins the game. For custom schemes,
whatever app is registered handles it. No `Process::spawn` of game
binaries — the launcher integration is purely URI-scheme based, which
keeps Rekindle out of the way of anti-cheat systems.

## Privacy posture

Detection and sharing are two settings (plan E7.2, prod research batch 7):

- **`game_detection_enabled`** (on by default, local only): while on, the
  Host's game stage (`crates/rekindle-node/src/host/game_stage.rs`) scans
  the process table at `game_scan_interval_secs` and records each session
  in `game_sessions` for the local game-time history. Off, no scanner task
  exists and the process table is never read. `SetGameDetection` (U1
  `SetPreferences`, Routine) starts or stops the stage at once, with no
  restart; turning it off clears a game the detector was showing. The bulk
  preference save never writes it, so only the Host changes it.
- **`share_current_game`** (off by default): only while on does the detected
  game become our shown game, written sealed under our friend-presence key
  to profile subkey 4 (and so seen only by friends holding that key).
  `SetShareCurrentGame` is TrustChanging (U1 `ShareCurrentGame`); turning it
  off clears the shown game at once. A game set by hand
  (`GamePresenceSet`) is an explicit act and is not gated.
- **`hidden_games(owner_key, game_id)`**: a hidden game is never our shown
  game and is left out of the `GameTotals` frames friends receive (hiding
  one re-sends the totals while totals sharing is on, so friends' copies,
  which each frame replaces, drop it); it is still recorded locally.
  `GameHide` is Routine (it narrows exposure), `GameUnhide` TrustChanging.
  Settings → Privacy lists the hidden games and hides any game played.
- **Game totals** go to friends only while `share_game_totals` is on (see
  the plan's E7.2 sharing slices).
- A per-community `share_activity` flag has nothing to gate yet: the Host
  sends the game to friends only (profile subkey 4); no community fan-out
  of the current game exists. The flag is built with that fan-out.

The database is **bundled** — no network requests are made to identify
games. The scanner runs **locally** — no process list ever leaves the
machine; a command line is read only for an already matched process.

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
| Background scan loop | `crates/rekindle-game-detect/src/runtime.rs`, run by `crates/rekindle-node/src/host/game.rs` and switched by `host/game_stage.rs` |
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
