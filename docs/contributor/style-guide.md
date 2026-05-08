# Style Guide

Rekindle's style rules are a mix of "what every Rust project should
do" and "what this project's threat model and architecture demand."
Most are enforced by lints; the rest are checked in code review.

This guide is a quick reference. The authoritative source for Rust
lints is the workspace `Cargo.toml`; the authoritative source for
front-end conventions is [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md).

## Rust

### Lints (denied at the workspace level)

```toml
[workspace.lints.rust]
warnings = "deny"
dead_code = "deny"
unused_imports = "deny"
unused_variables = "deny"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
pedantic = { level = "warn", priority = -1 }
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
undocumented_unsafe_blocks = "deny"
```

CI fails on any warning. `cargo clippy --workspace -- -D warnings`
must be clean.

### What `#[allow]` is and is not for

- **Use `#[allow]`** for genuinely false-positive lints with a `//
  reason: ...` comment explaining why.
- **Do not use `#[allow]`** to silence size or complexity lints.
  Refactor into smaller helpers instead.
- **Do not use `#[allow(dead_code)]`.** Wire the code fully or
  delete it.
- **Do not use `#[allow]`** to bypass `dbg_macro`, `todo`,
  `unimplemented`, or `undocumented_unsafe_blocks`. These are
  load-bearing.

### Tiers and crate boundaries

The crate hierarchy is strict — see [`../architecture/crates.md`](../architecture/crates.md).
Lower tiers know nothing about higher tiers. Do not add a dependency
that goes the wrong direction.

The crypto boundary is sacred:

- **Only `rekindle-secrets`** may import `ed25519-dalek`,
  `x25519-dalek`, `aes-gcm`, `chacha20poly1305`, `hkdf`, `argon2`.
- **Only `rekindle-transport`** (daemon track) and a small set of
  cross-cutting crates may import `veilid-core`. The daemon track
  enforces this strictly: only `broadcast/` and `subscriptions/`
  modules within `rekindle-transport` import it.
- Tier 6 (`rekindle-governance`) has no I/O, no async, no side
  effects. It is the CRDT merge engine — pure logic, pure test.

### `unsafe`

Pure-logic crates **forbid** `unsafe`:

```rust
#![forbid(unsafe_code)]
```

Cross-cutting crates that genuinely need `unsafe` (e.g., FFI to a C
library, platform-specific syscalls) must annotate every `unsafe
{ ... }` block with `// SAFETY: <reasoning>` — the
`undocumented_unsafe_blocks` lint enforces this.

### No legacy compatibility shims

Rekindle is **pre-release**. Drop replaced columns, replace fields
outright, delete old code paths. Do not add:

- "Fallback for legacy rows" branches.
- `_v1` / `_old` / `_deprecated` modules sitting alongside the new
  thing.
- Compatibility shims that translate between two versions.

The `001_init.sql` schema is edited in place and `SCHEMA_VERSION` is
bumped. On mismatch, the DB tables are dropped, Stronghold files
removed, and Veilid storage wiped (3 stores must stay in sync).

### Async + parking_lot

`parking_lot` mutex guards are `!Send`. Clone data out before
`.await` points:

```rust
// Wrong: holds the guard across await
let guard = state.foo.lock();
do_async_thing(&guard).await;   // !Send guard across await

// Right: drop the guard, then await
let foo_clone = state.foo.lock().clone();
do_async_thing(&foo_clone).await;
```

### Veilid types

`RoutingContext` and `VeilidAPI` are `Arc`-based and `Clone`. Clone
them from `NodeHandle` before async DHT/routing calls — do not hold
references across `await` points.

### `cpal::Stream` is `!Send` on macOS

Audio streams must live on dedicated OS threads. Bridge them to
Tokio via `mpsc` channels. See
[`../architecture/voice.md`](../architecture/voice.md).

### Cap'n Proto

Generated modules go at the consuming crate's root:

```rust
// in lib.rs
pub mod foo_capnp { include!(concat!(env!("OUT_DIR"), "/foo_capnp.rs")); }
```

### Argon2 in debug

Override `opt-level = 3` for the `rust-argon2` package in dev mode:

```toml
[profile.dev.package.rust-argon2]
opt-level = 3
```

### IPC type conventions

All Rust IPC structs sent to the frontend:

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendInfo { /* ... */ }
```

Channel enums (events emitted by the backend):

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum ChatEvent { /* ... */ }
```

`UserStatus` is the established convention for lowercase variants:

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus { Online, Away, Busy, Offline }
```

### Testing patterns

- Pure-logic crates: unit tests in-file (`#[cfg(test)] mod tests`).
- CRDT merge: property tests with `proptest`; pin regressions.
- Cross-module integration: in `tests/` or `services/<feature>/tests.rs`.
- Security-sensitive changes: regression test mandatory — see
  [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md).

### Database access

```rust
// Async wrapper over rusqlite on a dedicated background thread.
pub type DbPool = tokio_rusqlite::Connection;

// Use db_helpers::* for all DB access:
db_call(pool, |conn| { /* sync rusqlite */ }).await?
db_call_or_default(pool, |conn| { /* sync rusqlite */ }, default).await
db_fire(pool, |conn| { /* sync rusqlite, fire-and-forget */ })
```

For read-only state access, use `state_helpers::*` accessors —
they hold the lock briefly, clone, and return.

## TypeScript / SolidJS

### Frontend is thin

All business logic lives in the Rust backend. The frontend renders
state, forwards user actions, and emits events. No business logic
in stores, components, or handlers.

### Tailwind 4 — global styles only

Tailwind utilities live in `src/styles/`. Components compose
semantic class names defined in `xfire-theme.css`:

```tsx
// Good
<div class="buddy-item">{name}</div>

// Wrong — inline utilities
<div class="bg-xfire-bg-panel text-xfire-text-primary p-2 …">{name}</div>
```

### Window roots are flex columns

Every window root is `display: flex; flex-direction: column;` with
one `flex: 1` child holding the footer down. Wrappers must
preserve the flex contract or use `display: contents`. See
[`../architecture/ui-skin.md`](../architecture/ui-skin.md).

### IPC types mirror Rust

Frontend types in `src/ipc/commands.ts` and `src/ipc/channels.ts`
mirror the Rust structs. Keep camelCase casing on both sides.
Use the typed `invoke()` wrappers, not raw `tauri.invoke`.

### No emojis in app chrome

Use Nerd Font glyphs (loaded from
`src/assets/fonts/SymbolsNerdFontMono-Regular.woff2`) via
`.nf-icon`. User-content emoji rendering (reactions, message
bodies) is fine — that's user-driven content, not app chrome.

### Stores

SolidJS stores follow a per-feature split: `auth.store.ts`,
`chat.store.ts`, `friends.store.ts`, `community.store.ts`, etc.
Stores hold reactive signals, expose action functions, and call
`invoke` for side effects. They do not hold business logic.

### Handlers

Backend events are subscribed via `safeListen()` in
`src/handlers/<feature>.ts`. Each handler file is named for the
event channel it owns and exports a setup function called once at
startup.

## Tailwind

Stylesheets are split:

| File | Purpose |
|------|---------|
| `src/styles/global.css` | Tailwind imports, theme tokens, font-faces, root styles |
| `src/styles/xfire-theme.css` | Component classes (`.buddy-item`, `.message-bubble`, …) |
| `src/styles/animations.css` | Motion library (respects `prefers-reduced-motion`) |
| `src/styles/scrollbar.css` | Custom scrollbar styling |

The theme tokens in `global.css` are the **single source of truth**
for colour, spacing, and typography. Do not hardcode hex codes,
pixel values, or font names in components.

## Commit messages

Follow the existing repo style:

- One-line summary in imperative mood ("Add MEK rotation cascade").
- Optional body explaining the *why*.
- No "WIP:" prefixes on PR-merge commits.

For long-lived branches, the user manages commits themselves —
contributors do not squash, commit, or push uninvited.

## Pull requests

See [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md) "Pull request
process" for the canonical checklist. The
`.github/PULL_REQUEST_TEMPLATE.md` enforces it.

## Documentation style

- One-line headlines ("# Voice Architecture", not "## A Comprehensive
  Look at the Voice System").
- Code samples use language tags (`` ```rust ``, `` ```ts ``,
  `` ```bash ``).
- Cross-references between `/docs/` files use markdown links with the
  full relative path (`[`../architecture/communities.md`](../architecture/communities.md)`).
- No emojis in technical docs.
- Long docs are split by section heading (`## 1. Title`) for easy
  navigation.
