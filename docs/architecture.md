# Application Architecture

## Overview

Rekindle is a Tauri 2 desktop application with a Rust backend and web frontend. The architecture
is designed to cleanly separate the Xfire protocol implementation from the UI layer.

```
┌─────────────────────────────────────────────────┐
│  Frontend (Webview)                             │
│  TypeScript + Framework + Classic Xfire CSS     │
│                                                 │
│  ┌──────────┐ ┌───────────┐ ┌───────────────┐  │
│  │ Login    │ │ Buddy     │ │ Chat Windows  │  │
│  │ Screen   │ │ List      │ │ (per friend)  │  │
│  └────┬─────┘ └─────┬─────┘ └──────┬────────┘  │
│       └─────────────┼──────────────┘            │
│              Tauri IPC                          │
└──────────────┬──────────────────────────────────┘
               │
┌──────────────┼──────────────────────────────────┐
│  Rust Backend│                                  │
│  ┌───────────┴────────────────────────────────┐ │
│  │  src-tauri (Tauri Commands + State)        │ │
│  │  - commands/auth.rs    (login, logout)     │ │
│  │  - commands/chat.rs    (messages, typing)  │ │
│  │  - commands/friends.rs (add, remove, list) │ │
│  │  - commands/status.rs  (status, nickname)  │ │
│  │  - state.rs            (session, friends)  │ │
│  │  - tray.rs             (system tray)       │ │
│  │  - windows.rs          (multi-window mgmt) │ │
│  └──────┬─────────────────┬───────────────────┘ │
│         │                 │                     │
│  ┌──────┴──────────┐  ┌──┴──────────────────┐  │
│  │ rekindle-       │  │ rekindle-game-      │  │
│  │ protocol        │  │ detect              │  │
│  │ (pure Rust,     │  │ (process scanning,  │  │
│  │  no Tauri dep)  │  │  platform-specific) │  │
│  └────────┬────────┘  └─────────────────────┘  │
└───────────┼─────────────────────────────────────┘
            │ TCP:25999
     ┌──────┴──────┐
     │ Xfire Server │
     └─────────────┘
```

## Crate Separation

### `rekindle-protocol`
Pure Rust library. Zero Tauri dependency. Handles:
- Binary packet parsing and serialization
- Attribute type system (string, int32, SID, list, map)
- All message types (login, chat, friends, game, status, groups)
- tokio codec for framed TCP I/O
- SHA-1 authentication (double-hash with "UltimateArena" constant)

Can be used independently of Tauri — in a CLI tool, a server, tests, or any Rust project.

### `rekindle-game-detect`
Cross-platform game detection. Handles:
- Process enumeration (Windows: CreateToolhelp32Snapshot, Linux: /proc, macOS: sysinfo)
- Game database (game ID <-> process name mapping)
- Periodic scanning with configurable interval

### `src-tauri`
The Tauri 2 app shell. Imports both crates and exposes them to the frontend via:
- **Commands**: Request-response operations (login, send message, add friend)
- **Channels**: Streaming data (incoming messages, presence updates, typing indicators)
- **Events**: Broadcast notifications (theme changes, window-to-window communication)

## Window Architecture

Classic Xfire used separate windows, not tabs. Rekindle recreates this:

| Window | Purpose | Created |
|--------|---------|---------|
| `login` | Login screen | At launch (if no saved session) |
| `buddy-list` | Main friends list | After successful login |
| `chat-{userid}` | Chat conversation | On double-click friend or incoming message |
| `settings` | Preferences | From menu, single instance |

All windows use `decorations: false` + `transparent: true` for the classic Xfire skinned look with
a custom CSS titlebar and `data-tauri-drag-region` for native window dragging.

## IPC Pattern Summary

| Operation | Mechanism | Direction | Example |
|-----------|-----------|-----------|---------|
| Login | Command | FE -> Rust | `invoke('login', { username, password })` |
| Send message | Command | FE -> Rust | `invoke('send_message', { to, body })` |
| Add friend | Command | FE -> Rust | `invoke('add_friend', { username, msg })` |
| Get friends | Command | FE -> Rust | `invoke('get_friends')` |
| Incoming messages | Channel | Rust -> FE | `Channel<ChatEvent>` streaming |
| Presence updates | Channel | Rust -> FE | `Channel<ChatEvent>::PresenceUpdate` |
| Typing indicator | Channel | Rust -> FE | `Channel<ChatEvent>::TypingIndicator` |
| Window notification | Event | Broadcast | `app.emit_to("buddy-list", "refresh", ())` |

## Data Storage

| Data | Storage | Crate |
|------|---------|-------|
| Chat history | SQLite | `tauri-plugin-sql` |
| Credentials | Stronghold (encrypted) | `tauri-plugin-stronghold` |
| User preferences | JSON key-value store | `tauri-plugin-store` |
| Window positions | Auto-saved state | `tauri-plugin-window-state` |

## Build & Distribution

Built with Tauri's bundler. Output per platform:

| Platform | Format | Notes |
|----------|--------|-------|
| Windows | NSIS setup.exe | Code signing recommended |
| macOS | .dmg + .app | Requires notarization for distribution |
| Linux | .deb, .rpm, .AppImage | Broad format support |

CI/CD via GitHub Actions using `tauri-apps/tauri-action`.

## Dev Environment

Konductor `frontend` devshell provides all build dependencies:

```bash
nix develop .#frontend   # Linux: Rust, Node, GTK, WebKitGTK, Playwright
```

For macOS, manual setup of Rust + Node + Xcode tools is required (Konductor's frontend shell is
Linux-only).
