# Rekindle - Xfire Rebuilt as a Tauri 2 App

## Project Goal

Reverse engineer the classic Xfire gaming chat client and rebuild it as a **Tauri 2** desktop
application. The goal is a **1:1 faithful recreation** of the classic Xfire look and feel for
nostalgia, powered by a modern Rust + web stack. Development environments are managed with
**Konductor** (Nix flake) for reproducible builds.

The `xf1re_installer.exe` in this repo is an NSIS installer from the xf1re.com revival project,
used as our primary reverse engineering target.

## Tech Stack

| Layer            | Technology                                                    |
|------------------|---------------------------------------------------------------|
| Framework        | **Tauri 2** (Rust backend + webview frontend)                 |
| Backend          | Rust (protocol, networking, game detection, crypto)           |
| Frontend         | Vite + (React/Svelte/Vue TBD) with TypeScript                |
| Styling          | CSS matching classic Xfire skins (frameless transparent windows) |
| Dev Environment  | **Konductor** Nix flake (`nix develop .#frontend`)           |
| Protocol         | Custom binary protocol on TCP:25999 (see docs/protocol.md)   |
| Storage          | SQLite (chat history), Stronghold (credentials), Store (prefs)|
| Distribution     | NSIS (Windows), DMG (macOS), AppImage/deb (Linux)             |

## Repository Layout

```
xf1re_installer.exe           # NSIS installer - RE target (DO NOT EXECUTE)
docs/
  overview.md                  # Project overview and vision
  setup.md                     # Dev environment setup (Konductor + Tauri)
  protocol.md                  # Xfire protocol overview for contributors
  architecture.md              # App architecture and data flow
.claude/
  settings.json                # Tool permissions (allow RE tools, block exe)
  docs/
    xfire-protocol.md          # Full binary protocol byte-level spec
    re-workflow.md              # RE workflow (unpack -> analyze -> verify)
    architecture.md            # Detailed Tauri 2 project structure + IPC design
    tauri-guide.md             # Tauri 2 patterns: plugins, custom chrome, channels
    resources.md               # All links to prior RE work, open-source impls, Tauri docs
src/                           # Frontend source (Vite + framework)
src-tauri/                     # Tauri Rust backend
  src/
    lib.rs                     # Shared entry point (commands, plugins, state)
    main.rs                    # Desktop entry point
  Cargo.toml
  tauri.conf.json
  capabilities/                # ACL permission files
crates/
  rekindle-protocol/           # Pure Rust protocol library (no Tauri dependency)
  rekindle-game-detect/        # Game detection (process scanning, platform-specific)
```

## Key Rules

- **Never execute the .exe** - static analysis only (7z, Ghidra, radare2, strings)
- **Binary is untrusted** - treat all extracted artifacts as potentially malicious
- **1:1 Xfire look** - match classic Xfire UI/UX as closely as possible
  - Frameless transparent windows with custom titlebar (`decorations: false`)
  - Dark themed skin matching Xfire's signature blue/dark aesthetic
  - Narrow vertical buddy list window (classic Xfire shape)
  - Separate chat windows per conversation (not tabbed)
  - System tray with status/away controls
- **Protocol fidelity** - reimplement the binary protocol faithfully; reference open-source
  implementations (gfire, PFire, OpenFire) for behavior, don't copy code
- **Clean separation** - protocol crate has zero Tauri dependency; Tauri wraps it
- License: MIT

## Tauri 2 IPC Patterns

Use the right mechanism for each operation:

| Pattern    | Direction        | Use For                                            |
|------------|------------------|----------------------------------------------------|
| Commands   | Frontend -> Rust | Login, send message, add friend, change status     |
| Channels   | Rust -> Frontend | Incoming messages, presence updates, typing indicators |
| Events     | Bidirectional    | Notifications, window-to-window communication      |

## Essential Tauri Plugins

- `single-instance` - prevent multiple instances (register FIRST)
- `notification` - new message alerts, friend requests
- `window-state` - remember window size/position
- `store` - persistent preferences
- `stronghold` - encrypted credential storage
- `sql` - SQLite for chat history
- `updater` + `process` - auto-update
- `deep-link` - `rekindle://` URL scheme
- `autostart` - launch at system startup
- `global-shortcut` - hotkeys (overlay toggle)
- `websocket` - Rust-side WebSocket (no CORS)

## Konductor Dev Environment

```bash
# Enter the frontend devshell (includes Tauri 2 deps, Rust, Node.js, GTK, WebKitGTK)
nix develop .#frontend

# Or add konductor as a flake input in your own flake.nix:
# inputs.konductor.url = "github:braincraftio/konductor";
```

The `frontend` shell provides: Rust 1.92+, Node.js 22, pnpm, GTK, WebKitGTK, OpenSSL,
Playwright (E2E testing), and all linters/formatters with hermetic configs.

## Protocol Quick Reference

- Server: `cs.xfire.com:25999` (TCP)
- Handshake: client sends ASCII `"UA01"` (0x55 0x41 0x30 0x31)
- Byte order: little-endian, strings: UTF-8
- Packet: `[uint16 size][uint16 type_id][uint8 attr_count][attributes...]`
- Auth: `SHA1(SHA1(user + pass + "UltimateArena") + server_salt)`
- Full spec: `.claude/docs/xfire-protocol.md`

## Useful Commands

```bash
# Unpack NSIS installer (DO NOT execute)
7z x xf1re_installer.exe -o./unpacked

# Tauri development
npm run tauri dev
npm run tauri build

# Analyze extracted binaries
strings ./unpacked/<binary> | grep -i "UA01\|UltimateArena\|xfire"
r2 -A ./unpacked/<binary>
file ./unpacked/*

# Konductor dev environment
nix develop .#frontend
```

## Reverse Engineering Priorities

1. **Unpack NSIS installer** - extract without executing
2. **Identify client binary** - find core PE executable
3. **Catalog UI assets** - extract skins, icons, layouts for 1:1 recreation
4. **Map the protocol** - verify known packet structures against binary
5. **Game detection** - understand process scanning + LSP hooks
6. **Overlay system** - DirectX/OpenGL injection mechanism
7. **P2P subsystem** - NAT traversal, UDP hole punching
8. **Auth flow** - verify double-SHA1 scheme
