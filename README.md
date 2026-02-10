# Rekindle

**What if Xfire were reborn as a modern desktop app?**

Rekindle is a from-scratch rebuild of [Xfire](https://en.wikipedia.org/wiki/Xfire) — the legendary
gaming chat client (2004–2015) — as a [Tauri 2](https://v2.tauri.app/) application. The goal is a
**1:1 faithful recreation** of the classic Xfire look and feel, powered by a modern Rust + web
technology stack.

## Why?

Xfire was the original gaming social network: friends lists, in-game chat overlay, game time
tracking, server browsing — years before Steam, Discord, or Twitch offered the same. It peaked at
22+ million users before shutting down in 2015. The nostalgia is real.

Rekindle aims to bring it back. Not as a clone with a new coat of paint, but as a faithful
recreation that feels like opening Xfire for the first time again — skinned windows, buddy list,
game detection, the whole thing.

## Tech Stack

| Layer           | Technology                                          |
|-----------------|-----------------------------------------------------|
| App Framework   | [Tauri 2](https://v2.tauri.app/) (Rust + webview)   |
| Backend         | Rust (protocol, networking, game detection, crypto)  |
| Frontend        | Vite + TypeScript                                    |
| Styling         | CSS (classic Xfire skin, frameless transparent windows) |
| Dev Environment | [Konductor](https://github.com/braincraftio/konductor) (Nix flake) |
| Protocol        | Xfire binary protocol (TCP:25999, reverse-engineered)|
| Storage         | SQLite (history), Stronghold (credentials)           |

## Project Structure

```
xf1re_installer.exe        # RE target (Xfire/Xf1re NSIS installer — DO NOT EXECUTE)
docs/                       # Project documentation
  overview.md               # Vision and goals
  setup.md                  # Development environment setup
  protocol.md               # Xfire protocol overview
  architecture.md           # Application architecture
src/                        # Frontend (Vite + TypeScript)
src-tauri/                  # Tauri Rust backend
crates/
  rekindle-protocol/        # Pure Rust Xfire protocol library
  rekindle-game-detect/     # Game detection engine
```

## Getting Started

### Prerequisites

**Option A: Konductor (recommended)**

```bash
# Enter the frontend devshell — includes Rust, Node.js, Tauri deps, and tooling
nix develop .#frontend
```

**Option B: Manual setup**

- Rust 1.80+ (via rustup)
- Node.js 20+ with pnpm
- Tauri 2 system dependencies ([platform-specific guide](https://v2.tauri.app/start/prerequisites/))

### Development

```bash
# Install frontend dependencies
pnpm install

# Run in development mode (hot-reload)
pnpm tauri dev

# Build for production
pnpm tauri build
```

## Reverse Engineering

This repo contains `xf1re_installer.exe` from the [Xf1re](https://xf1re.com/) revival project. It
is used as a reverse engineering target to understand the Xfire client internals. The binary is
**untrusted and should never be executed** — only analyzed statically.

```bash
# Unpack the NSIS installer without executing
7z x xf1re_installer.exe -o./unpacked

# Inspect extracted files
file ./unpacked/*
```

See [docs/protocol.md](docs/protocol.md) for the Xfire protocol specification and
[docs/architecture.md](docs/architecture.md) for the application design.

## Design Goals

- **1:1 Classic Xfire UI** — frameless skinned windows, narrow buddy list, separate chat windows,
  dark blue theme, system tray with status controls
- **Protocol fidelity** — faithful reimplementation of the Xfire binary protocol
- **Modern internals** — Rust for safety and performance, async networking with tokio, Tauri 2 for
  cross-platform desktop delivery
- **Reproducible builds** — Konductor Nix flake ensures identical toolchains across all developers
  and CI

## Documentation

| Document | Description |
|----------|-------------|
| [docs/overview.md](docs/overview.md) | Project vision and goals |
| [docs/setup.md](docs/setup.md) | Development environment setup |
| [docs/protocol.md](docs/protocol.md) | Xfire protocol overview |
| [docs/architecture.md](docs/architecture.md) | Application architecture |

## License

[MIT](LICENSE)

## Acknowledgments

- The original Xfire team (Dennis "Thresh" Fong, Chris Kirmse, and team)
- [Xf1re](https://xf1re.com/) revival project
- [gfire](https://github.com/gfireproject/gfire) — Pidgin Xfire plugin (protocol reference)
- [PFire](https://github.com/darcymiranda/PFire) — Xfire server emulator
- [OpenFire](https://github.com/iainmcgin/openfire) — Protocol specification
- [Konductor](https://github.com/braincraftio/konductor) — Reproducible dev environments
