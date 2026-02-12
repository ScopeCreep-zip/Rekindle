# Development Guide

## Environment Setup

### Konductor (Recommended)

[Konductor](https://github.com/braincraftio/konductor) is a Nix flake providing
a reproducible development environment. Its `frontend` devshell includes all
dependencies for Tauri 2 development.

**Prerequisites:** Nix package manager with flakes enabled.

```bash
# Enable flakes (if not already)
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf

# Enter the dev environment
cd Rekindle
nix develop .#frontend
```

The `frontend` shell provides:

| Tool | Purpose |
|------|---------|
| Rust 1.92+ | Backend compilation (cargo, clippy, rust-analyzer) |
| Node.js 22 | Frontend tooling |
| pnpm | Package management |
| GTK / WebKitGTK | Tauri 2 Linux dependencies |
| OpenSSL | TLS support |
| Playwright | E2E testing with bundled browsers |
| 13 linters | Code quality (hermetic configs) |
| 8 formatters | Code formatting (hermetic configs) |

The Konductor `frontend` shell is Linux-only (x86_64-linux) due to
GTK/WebKitGTK dependencies. macOS users should use manual setup.

### Manual Setup (macOS / Non-Nix)

**Rust:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

**Node.js:**
```bash
# Install Node.js 22+ via nvm, fnm, or system package manager
npm install -g pnpm
```

**Tauri 2 System Dependencies:**

Linux (Debian/Ubuntu):
```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

macOS:
```bash
xcode-select --install
```

## Build Commands

```bash
# Install frontend dependencies
pnpm install

# Development (hot-reload for both frontend and Rust)
pnpm tauri dev

# Production build
pnpm tauri build
```

## Testing

### Rust Unit Tests

```bash
# All workspace crates
cargo test --workspace

# Specific crate
cargo test -p rekindle-protocol
cargo test -p rekindle-crypto
cargo test -p rekindle-game-detect
cargo test -p rekindle-voice
```

Rust backend commands extract `_core` functions (e.g., `create_identity_core`,
`login_core`) that can be tested without a Tauri `AppHandle`.

### E2E Testing

Rekindle supports two E2E testing strategies:

**Real E2E (`pnpm test:e2e`):**
- HTTP bridge server (`e2e-server`) runs the real Rust backend
- Playwright tests the real SolidJS UI in a browser
- Uses SQLite + Stronghold + Ed25519 — no mocking
- `VITE_E2E=true` → `invoke()` sends HTTP POST to `localhost:3001`
- `channels.ts` uses `safeListen()` which is a no-op (no Tauri event system)

**Mock IPC (`pnpm test:mock`):**
- `VITE_PLAYWRIGHT=true` → `mockIPC` from `@tauri-apps/api/mocks`
- Tests frontend behavior with stubbed responses
- Faster, no Rust compilation required

### Linting

```bash
# Rust
cargo clippy --workspace -- -D warnings
cargo fmt --all --check

# Frontend
pnpm tsc --noEmit
```

## Code Conventions

### Serde Attributes

All Rust IPC structs must use `#[serde(rename_all = "camelCase")]` to match
JavaScript naming conventions.

Channel event enums use adjacently tagged serialization:
```rust
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
```

The `UserStatus` enum uses lowercase serialization:
```rust
#[serde(rename_all = "lowercase")]  // → "online", "away", "busy", "offline"
```

### Zero Warnings Policy

The workspace enforces `deny(warnings)` in `Cargo.toml`:

```toml
[workspace.lints.rust]
warnings = "deny"
unused-imports = "deny"
dead-code = "deny"
unused-variables = "deny"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }
```

`#[allow(dead_code)]` is never used. All code must be fully wired.

### Tailwind CSS

Global styles only. No inline Tailwind utility classes in component JSX. All
styling is done via `@apply` in `src/styles/global.css`.

### Database Schema

The schema is defined in `src-tauri/migrations/001_init.sql`. There are no
migration files — the schema is edited directly since it is not yet deployed
to production.

A `SCHEMA_VERSION` constant in `db.rs` is incremented whenever `001_init.sql`
changes. On startup, if the stored version does not match, all SQLite tables
are dropped, Stronghold files are deleted, and Veilid local storage is wiped.
This ensures the three data stores remain synchronized.

### Concurrency

- `parking_lot` guards are `!Send` — clone data out before `.await` points
- `Veilid RoutingContext` and `VeilidAPI` are `Arc`-based and `Clone` — clone
  from `NodeHandle` before async DHT or routing calls
- `std::sync::Mutex` for `DbPool` (not `parking_lot`) since it is used with
  `spawn_blocking`
- `cpal::Stream` is `!Send` on macOS — audio streams live on dedicated OS
  threads, communicating via `mpsc` channels

### Argon2 Performance

Debug-mode Argon2 is extremely slow. The workspace optimizes crypto packages
even in dev builds:

```toml
[profile.dev.package.rust-argon2]
opt-level = 3
[profile.dev.package.iota_stronghold]
opt-level = 2
```

`iota_stronghold` uses `rust-argon2` (not the `argon2` crate) internally.

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `VITE_E2E` | Enable E2E HTTP bridge mode | unset |
| `VITE_PLAYWRIGHT` | Enable mock IPC mode | unset |

## IDE Setup

### VS Code

Recommended extensions:
- `tauri-apps.tauri-vscode` — Tauri integration
- `rust-lang.rust-analyzer` — Rust language server
- `bradlc.vscode-tailwindcss` — Tailwind CSS IntelliSense
- `biomejs.biome` — TypeScript linting

### Neovim

Konductor's devshell includes Neovim with rust-analyzer, TypeScript LSP,
Tailwind CSS LSP, and other language servers pre-configured.
