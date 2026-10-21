# Testing Guide

Rekindle has four distinct test surfaces. Each catches different
classes of bug; together they let us iterate confidently on a
non-trivial protocol stack.

## At a glance

```
┌──────────────────────────────────────────────────────────┐
│  Rust unit + integration tests   (cargo test --workspace)  │
│  ├── Pure-logic crates: deterministic, fast (<1s)         │
│  ├── Tier 6 governance: property tests for CRDT merge     │
│  └── Cross-cutting integration: real I/O, slower          │
├──────────────────────────────────────────────────────────┤
│  Playwright E2E (real backend)    (pnpm test:e2e)         │
│  ├── Real SQLite + Vault + Veilid bootstrap               │
│  ├── HTTP IPC via rekindle-e2e-server                     │
│  └── Catches integration bugs the unit tests miss         │
├──────────────────────────────────────────────────────────┤
│  Playwright Mock IPC              (pnpm test:mock)        │
│  ├── In-browser, no backend                               │
│  ├── Fast UI regression tests                             │
│  └── Catches frontend rendering bugs                      │
├──────────────────────────────────────────────────────────┤
│  Manual visual / interaction      (`pnpm tauri dev`)      │
│  └── Frontend feature work, voice, multi-window flows     │
└──────────────────────────────────────────────────────────┘
```

## Rust tests

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

All three commands must pass before requesting review. CI runs them
on every PR.

### Per-crate testing

```bash
cargo test -p rekindle-types
cargo test -p rekindle-secrets
cargo test -p rekindle-codec
cargo test -p rekindle-records
cargo test -p rekindle-gossip
cargo test -p rekindle-governance
# ...
```

### Property tests for CRDT merge

`rekindle-governance` uses [`proptest`](https://docs.rs/proptest/) to
verify the CRDT merge engine's three properties:

- **Convergence:** `merge(entries.shuffled()) == merge(entries)`
- **Idempotence:** `merge(merge(entries) ∪ entries) == merge(entries)`
- **Commutativity:** `merge(a; b) == merge(b; a)`

Failing seeds are pinned in
`crates/rekindle-governance/proptest-regressions/merge.txt`. **Do
not delete this file** — it captures the inputs that caught past
bugs and is part of the regression test suite.

### Network simulation (`rekindle-netsim`)

Multi-node behaviour runs on a seeded simulation of the network (plan
C9.3): `rekindle-netsim` implements the record, route and app-message
ports of `rekindle_records::port` in memory, with Veilid's record rules
(publish on first write, schema writers, local seq + 1, supersede,
per-node watches), a seeded delivery order, a `SimClock` and a
`SimRandom` per node, and faults (try again, offline, stale reads,
partitions and heal, dropped messages, unanswered calls, clock skew,
watch and route death, a modified client's write, a same-seq conflict).

The netsim models Veilid's latency by default, and nothing turns it off
(plan E2.B): every DHT call (create, open, get, set, inspect, watch) and
every app message or call costs simulated time drawn from a seeded,
per-node generator. A set waits for consensus (~13.5 s, measured live), a
read that finds no value runs to the fanout's 10 s deadline, a read of a
held value takes 1–3 s, a message 0.2–0.8 s one way. Calls on one
`(record, subkey)` serialize, calls on different subkeys overlap, and a
node runs at most `VeilidStartupOptions::max_concurrent_dht_operations`
DHT calls at once. An offline DHT fails at once, as Veilid's
`dht_is_online()` check does. Scenarios run on paused tokio time, so the
waits cost no wall time; a scenario's `eventually` limit is simulated
time and must allow what a real network needs. The calibration table,
each number's source (a pinned `~/veilid` constant with its file and line,
or a live log's timestamps) and how to recalibrate from a live log are in
the module documentation of `crates/rekindle-netsim/src/latency.rs`.

Scenarios live in `crates/rekindle-node/tests/netsim/`: each node is a
real `Host::start` with `TransportSource::Ports` on a temp data root, and
requests go through `HostApi::call` (a held request is proved as the CLI
proves it). Run them with
`cargo test -p rekindle-node --test netsim` (add `-- --include-ignored`
to run the scenarios a later plan step fixes; `RUST_LOG=... -- --nocapture`
shows the Hosts' logs). One seed gives one trace (`Trace::hash`).

### Where to put new tests

| Test target | Location |
|-------------|----------|
| Pure-logic primitives | Unit tests in the same file (`#[cfg(test)] mod tests`) |
| Cross-module integration within a crate | `tests/` directory of the crate |
| Multi-crate integration | `src-tauri/src/services/<feature>/tests.rs` or similar |
| Property tests for CRDT-style code | Use `proptest`, pin regressions |
| Round-trip tests for wire formats | `crates/rekindle-codec/src/envelope.rs::tests`, similar |
| Several nodes, faults, ordering | A netsim scenario in `crates/rekindle-node/tests/netsim/` |

### Argon2 in debug builds

Argon2 is intentionally slow. Debug builds make it painfully so. The
workspace overrides this for the `rust-argon2` crate:

```toml
[profile.dev.package.argon2]
opt-level = 3
```

If you add a test that exercises vault unlock, expect it to be slow
without this override.

## Playwright E2E (real backend)

```bash
pnpm test:e2e          # full suite
pnpm test:e2e -- --headed   # see the browser
pnpm test:e2e -- --debug    # step through
```

E2E tests drive the real Rust backend over HTTP. The mechanism:

1. `pnpm test:e2e` sets `VITE_E2E=true`.
2. The `rekindle-e2e-server` crate spins up an HTTP IPC bridge on
   `localhost:3001` that mirrors every Tauri command.
3. The frontend's `src/ipc/invoke.ts` detects `VITE_E2E` and routes
   through HTTP instead of Tauri's IPC.
4. Playwright opens the SolidJS app in a real browser and exercises
   real flows.

This catches the class of bugs unit tests miss: state-management
issues, real cryptographic round trips, Veilid network bootstrap,
SQLite schema conflicts.

### Test fixtures

Test fixtures live at `e2e/fixtures/`. They include pre-baked
identities, communities, and friend states so tests can start from
known scenarios.

## Playwright Mock IPC (no backend)

```bash
pnpm test:mock
```

Sets `VITE_PLAYWRIGHT=true`, which makes `src/ipc/invoke.ts` use a
mock IPC layer that returns stubbed responses. Useful for:

- Pure UI regression tests (snapshot rendering, interaction flows).
- Component-level tests that don't need backend behaviour.
- Quick iteration on the frontend without spinning up Veilid.

`startEventStream()` (`src/ipc/channels/subscriptions.ts`) is a no-op
under `VITE_E2E=true` (no Tauri event transport in the browser).

### When to use which

| Scenario | Use |
|----------|-----|
| Backend logic | `cargo test --workspace` |
| Backend + frontend wiring | `pnpm test:e2e` |
| Frontend rendering / interaction | `pnpm test:mock` |
| End-to-end feature flow | `pnpm test:e2e` |
| New UI component | `pnpm test:mock` |
| Cryptographic round trip | `cargo test --workspace` |

`pnpm test:all` runs both Playwright suites in sequence.

## E2E vs production: the `_core` pattern

To keep the Rust backend testable without Tauri, security-sensitive
logic is split into `_core` functions that take plain inputs and
produce plain outputs:

```rust
// In src-tauri/src/commands/auth.rs
#[tauri::command]
pub async fn create_identity(
    state: State<'_, SharedState>,
    display_name: String,
) -> Result<IdentityCreated, String> {
    create_identity_core(&state, display_name).await
}

pub async fn create_identity_core(
    state: &SharedState,
    display_name: String,
) -> Result<IdentityCreated, String> {
    // ... real implementation
}
```

Tests can call `create_identity_core` directly with a `SharedState`
fixture, bypassing the Tauri command machinery. Adopting this
pattern for new commands is encouraged — see existing `_core`
functions for examples.

## Hermetic linter / formatter configs

The Konductor `frontend` Nix shell ships every linter and formatter
with hermetic configurations: `cargo fmt`, `cargo clippy`, `rustfmt`,
`prettier`, `eslint`, `taplo`, `nixpkgs-fmt`, `shfmt`, etc. Running
them in the shell guarantees consistent results across contributors.

For non-Nix users, install matching versions manually and run
formatters before pushing. CI will reject any change that breaks
`cargo fmt --all -- --check` or the equivalent JS/TS check.

## Logs during tests

Set `RUST_LOG=rekindle=debug` (or `=trace` for more) to see structured
backend logging. Frontend logs go to the browser console; in headed
Playwright runs you can see them via DevTools.

## Test-driven contributions

For a non-trivial change, write the failing test first, then the fix.
This is especially important for:

- **Wire-format changes** — write a round-trip test against a fixed
  byte fixture.
- **CRDT merge changes** — add a property test scenario that
  reproduces the bug.
- **Permissions changes** — add a unit test for the new permission
  check that fails before your fix.
- **Security-sensitive changes** — see
  [`../../CONTRIBUTING.md`](../../CONTRIBUTING.md) "Security-sensitive
  changes" — every such change should land with a regression test.
