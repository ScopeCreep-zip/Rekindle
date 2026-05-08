# Development Guide

## Environment Setup

### Konductor (Recommended on Linux)

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
| Cap'n Proto compiler | Build-time schema generation |
| Playwright | E2E testing with bundled browsers |
| 13 linters | Code quality (hermetic configs) |
| 8 formatters | Code formatting (hermetic configs) |

The Konductor `frontend` shell is Linux-only (x86_64-linux) due to
GTK/WebKitGTK dependencies. macOS and Windows users should use manual setup.

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

**Cap'n Proto compiler** (`capnp`):
```bash
# macOS
brew install capnp

# Debian/Ubuntu
sudo apt install capnproto

# Windows: download release and set capnp on PATH
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

### Windows Setup

**Prerequisites:**
- [Visual Studio 2026](https://visualstudio.microsoft.com/downloads/) (or 2022)
  with the **"Desktop development with C++"** workload
- [CMake](https://cmake.org/download/) (`winget install Kitware.CMake`)
- [Rust](https://rustup.rs/) (`winget install Rustlang.Rustup`)
- [Node.js](https://nodejs.org/) LTS (`winget install OpenJS.NodeJS.LTS`)
- [pnpm](https://pnpm.io/) (`npm install -g pnpm`)
- [Cap'n Proto](https://capnproto.org/install.html) — download the Windows
  release and extract to `%LOCALAPPDATA%\capnproto\`. The build system will
  automatically find it.

The "Desktop development with C++" workload is required — it registers the
Windows SDK library paths that CMake needs to detect the MSVC compiler.

**Building on Windows:**

```powershell
pnpm install
pnpm tauri dev
```

CMake auto-detects the Visual Studio installation and MSVC compiler.

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
# All workspace crates (22 members)
cargo test --workspace

# Specific crate examples
cargo test -p rekindle-types
cargo test -p rekindle-secrets
cargo test -p rekindle-codec
cargo test -p rekindle-records
cargo test -p rekindle-gossip
cargo test -p rekindle-governance
cargo test -p rekindle-dm
cargo test -p rekindle-files
cargo test -p rekindle-link-preview
cargo test -p rekindle-video
cargo test -p rekindle-sync
cargo test -p rekindle-protocol
cargo test -p rekindle-crypto
cargo test -p rekindle-game-detect
cargo test -p rekindle-voice
```

`rekindle-governance` ships property-based tests with proptest regression
seeds in `crates/rekindle-governance/proptest-regressions/merge.txt` —
do not delete that file.

Rust backend commands extract `_core` functions (e.g., `create_identity_core`,
`login_core`) that can be tested without a Tauri `AppHandle`.

### E2E Testing

Rekindle supports two E2E testing strategies:

**Real E2E (`pnpm test:e2e`):**
- The `rekindle-e2e-server` crate exposes Tauri commands over HTTP at
  `http://127.0.0.1:3001/invoke`
- Playwright drives the real SolidJS UI in a browser
- Uses real SQLite + Stronghold + Ed25519 — no mocking
- Frontend code path: `VITE_E2E=true` → `invoke()` sends HTTP POST
- `channels.ts` uses `safeListen()` which is a no-op in this mode

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
#[serde(rename_all = "lowercase")]  // → "online", "away", "busy", "offline", "invisible"
```

### Zero Warnings Policy

The workspace enforces `deny(warnings)` plus restriction lints:

```toml
[workspace.lints.rust]
warnings = "deny"
unused-imports = "deny"
dead-code = "deny"
unused-variables = "deny"
unsafe-op-in-unsafe-fn = "warn"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }

# Restriction lints — enforce code quality
dbg-macro = "deny"
todo = "deny"
unimplemented = "deny"
undocumented-unsafe-blocks = "deny"
print-stdout = "warn"
print-stderr = "warn"
```

`#[allow(dead_code)]` is never used. All code must be fully wired. Don't
silence size/complexity lints — refactor into smaller helpers instead.

### Tailwind CSS

Global styles only. No inline Tailwind utility classes in component JSX. All
styling is done via `@apply` in `src/styles/global.css` and the Xfire-theme
CSS files.

### Database Schema

The schema is defined in `src-tauri/migrations/001_init.sql`. There are no
migration files — the schema is edited directly since it is not yet deployed
to production.

A `SCHEMA_VERSION` constant in `src-tauri/src/db.rs` (currently **56**) is
incremented whenever `001_init.sql` changes. On startup, if the stored
version does not match, all SQLite tables are dropped, Stronghold files are
deleted, the Veilid local storage is wiped, and the Lost Cargo file cache is
removed. This ensures the four data stores remain synchronized.

### Concurrency

- `parking_lot` guards are `!Send` — clone data out before `.await` points
- `Veilid RoutingContext` and `VeilidAPI` are `Arc`-based and `Clone` — clone
  from `NodeHandle` before async DHT or routing calls
- `tokio_rusqlite::Connection` for the DbPool — async wrapper around
  `rusqlite` on a dedicated background thread. Use the `db_helpers`
  module (`db_call`, `db_call_or_default`, `db_fire`)
- `cpal::Stream` is `!Send` on macOS — audio streams live on dedicated OS
  threads, communicating via `mpsc` channels

### Crate Tier Hierarchy

The Rust crates form a strict dependency hierarchy. **Lower tiers know
nothing about higher tiers.** Crypto operations are confined to
`rekindle-secrets` (Tier 2); no other crate may import `ed25519-dalek`,
`x25519-dalek`, `aes-gcm`, or `hkdf` directly. The CRDT merge in
`rekindle-governance` (Tier 6) is a pure function — no I/O, no async, no
side effects. See [`../architecture/crates.md`](../architecture/crates.md) for the full layout.

### Argon2 Performance

Debug-mode Argon2 is extremely slow. The workspace optimizes crypto packages
even in dev builds:

```toml
[profile.dev.package.argon2]
opt-level = 3
[profile.dev.package.rust-argon2]
opt-level = 3
[profile.dev.package.iota_stronghold]
opt-level = 2
[profile.dev.package.iota-crypto]
opt-level = 3
[profile.dev.package.scrypt]
opt-level = 3
```

`iota_stronghold` uses `rust-argon2` (not the `argon2` crate) internally.

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `VITE_E2E` | Enable E2E HTTP bridge mode | unset |
| `VITE_PLAYWRIGHT` | Enable mock IPC mode | unset |
| `RUST_LOG` | Tracing filter (default: `info,veilid_api=warn,veilid_core=warn`) | unset |

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
