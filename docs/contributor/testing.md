# Testing Guide

Rekindle has three test surfaces. Each catches a different class of
bug; together they let us iterate confidently on a non-trivial
protocol stack.

## At a glance

```text
┌────────────────────────────────────────────────────────────┐
│  Rust unit + integration tests   (cargo test --workspace)  │
│  ├── Pure-logic crates: deterministic, fast (<1s)          │
│  ├── Tier 6 governance: property tests for CRDT merge      │
│  └── Netsim scenarios: several real Hosts, seeded faults   │
├────────────────────────────────────────────────────────────┤
│  Real-app suite (WebdriverIO)     (pnpm test:app)          │
│  ├── The real desktop app, its real webview and Host       │
│  ├── One-user specs, then two-user (alice/bob) specs       │
│  ├── Headless peers: rekindled + rekindle CLI (fleet.ts)   │
│  └── Geometry checks in the real windows                   │
│      (+ pnpm test:app:devices for camera/mic specs)        │
├────────────────────────────────────────────────────────────┤
│  Real-app row walk                (pnpm walk)              │
│  └── The older walk; its rows move into specs (E7.14 S3)   │
└────────────────────────────────────────────────────────────┘
```

There is no mocked IPC layer and no HTTP bridge in front of the
frontend: every UI test drives the app users run.

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

### Wall-clock reports (`cargo xtask perf`)

A time bound fails under machine load, so no gating test measures wall
time (FLK3): the gate counts operations (e.g.
`crates/rekindle-skin/src/serve_tests.rs`), and the time bound is an
`#[ignore]`d test whose reason names `cargo xtask perf` (e.g.
`crates/rekindle-skin/tests/serve_budget.rs`). Run them by hand on an
idle machine:

```bash
cargo xtask perf
```

It runs each one in release, alone, with
`cargo test --release -p <crate> --test <target> -- --ignored --nocapture --exact <test>`,
prints its libtest lines (the result line carries the time; a failed
bound names what it took) and exits non-zero if any fails. It is not part
of `cargo xtask check` or CI. A new wall-clock test goes in the table
`REPORTS` in `xtask/src/perf.rs`; `cargo test -p xtask` fails until the
table names exactly the ignored tests whose reason mentions
`cargo xtask perf`.

### Where to put new tests

| Test target | Location |
| ------------- | ---------- |
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

## Real-app suite (`pnpm test:app`)

```bash
pnpm test:app                           # build, then every spec
REKINDLE_APP_NO_BUILD=1 pnpm test:app   # reuse the last build
pnpm exec wdio run e2e/app/wdio.conf.ts --spec e2e/app/specs/solo/identity.e2e.ts
pnpm test:app:devices                   # camera/mic specs (see below)
```

WebdriverIO's test runner drives the real desktop app through a
WebDriver server embedded in it. The mechanism:

1. The app is built once per run as a debug build with the cargo
   feature `webdriver-test` (`pnpm tauri build --debug --no-bundle
   --features webdriver-test`, `e2e/walk/launch.ts`). The feature pulls in
   the vendored `crates/vendor/tauri-plugin-wdio-webdriver`; a release
   build with it is a compile error (`src-tauri/src/lib.rs`) and
   `cargo xtask check-webdriver-release` keeps it out of release
   defaults. `REKINDLE_APP_NO_BUILD=1` skips the build and runs the last
   one; `CARGO_TARGET_DIR` is honoured when finding it.
2. The launcher service (`e2e/app/launcher.ts`) starts one app per
   worker (or per multiremote instance), each on its own temp data root,
   loopback port and per-run bearer token. The token reaches the worker
   over the fork's IPC channel, never argv or stdout.
3. The driver's request guard
   (`crates/vendor/tauri-plugin-wdio-webdriver/src/server/auth.rs`)
   refuses any request whose `Host` is not `127.0.0.1:<port>`, that
   carries an `Origin`, or that lacks the run's token (compared in
   constant time).
4. Specs run in the real webview (WKWebView on macOS, WebKitGTK on
   Linux) against the real Host, vault, SQLCipher database and Veilid
   network.

`pnpm test:app` runs two configs in sequence:

| Config | Specs | What runs |
| --- | --- | --- |
| `e2e/app/wdio.conf.ts` | `e2e/app/specs/solo/` | One user: identity, window layout, the friends list |
| `e2e/app/wdio.peers.conf.ts` | `e2e/app/specs/peers/` | Multiremote `alice` and `bob`, two apps talking to each other |

Lists whose rows are other people use headless real peers
(`e2e/app/fleet.ts`): each is a `rekindled` with its own HOME, driven by
the `rekindle` CLI, started one at a time and removed at the end of the
run. Shared journeys (create an identity, add a friend, create a
community) live in `e2e/app/journeys.ts` and `e2e/app/social.ts`.

Layout is checked in the real windows: `e2e/app/geometry.ts` runs the
page-side checks of `e2e/app/geometry-page.ts` (overlap, overflow,
clipping, spacing scale, target size, focus indicators, the flex
contract) through WebDriver's execute and reports every finding of a
window at once; `e2e/app/windows.ts` opens each window.

### Devices (`pnpm test:app:devices`)

`e2e/app/wdio.devices.conf.ts` runs the specs in
`e2e/app/specs/devices/` that need a camera or microphone (calls). It
runs on Linux, and on macOS only from the owner's own terminal: macOS
TCC grants device access to the launching app and refuses an app a
Claude session launched. The suite never raises a permission prompt,
and a refused device fails the spec loudly; it is never skipped.

### Not in CI

The real-app suite needs a desktop session (a window server and a
webview), so CI does not run it. The gate runs `pnpm test:app` on both
target machines (macOS and Linux).

### The row walk (`pnpm walk`)

`e2e/walk/` is the older real-app walk: a list of rows driven against
the same debug build and driver. It stays until plan step E7.14 S3
ports its rows into specs.

### Why not Playwright

Playwright drives webviews over the Chrome DevTools Protocol, which
only WebView2 (Windows) exposes; it cannot drive WKWebView (macOS) or
WebKitGTK (Linux) (<https://playwright.dev/docs/webview2>). Its own
WebKit build is not the engine users run, and a page in a browser
needed either mocked IPC or an HTTP bridge in place of the real Tauri
IPC. Plan E7.14 S8 removed the Playwright suites, the mocked IPC and
the E2E bridge. Where their scenarios went: about 27 of the 45 deleted
spec files are covered (wholly or in part) by real-app specs or walk
rows; the rest are backlog rows of E7.14 S3 (flows, 19), S5
(accessibility, keyboard, skins, 13) and S6 (CSP, isolation, XSS in the
real app, 4). Nine mock-only boundary cases (a malformed answer no real
Host sends) go to S3 to place, and two scenarios are not ported:
window-focus (a test window does not get system focus) and
forced-colors (never active in WebKit).

### When to use which

| Scenario | Use |
| ---------- | ----- |
| Backend logic | `cargo test --workspace` |
| Several nodes, faults, ordering | `cargo test -p rekindle-node --test netsim` |
| Cryptographic round trip | `cargo test --workspace` |
| UI wiring, rendering, interaction | `pnpm test:app` |
| Two users, end-to-end feature flow | `pnpm test:app` (peers config) |
| Window layout | `pnpm test:app` (geometry checks) |
| Calls, camera, microphone | `pnpm test:app:devices` |

## Testing without Tauri: the `_core` pattern

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
backend logging. Frontend logs go to the webview console; open the
webview inspector in a `pnpm tauri dev` window to see them.

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

## Canonical test fixtures

Owner ruling 2026-11-26: the test identities and items a secret scanner could read as
credentials are static, defined once in `crates/rekindle-test-fixtures` (a dev-dependency only),
and referenced by path (`rekindle_test_fixtures::HEX_KEY_64`), never copied.

- **Marker.** Each secret-shaped fixture is built from a ramp, the in-order run of its alphabet:
  hex values repeat `abcdef1234567890` (`HEX_RAMP`; a random 32-byte key repeats its first 8 bytes
  four times with probability 2^-192), and base32 tokens start `abcdefghijklmnopqrstuvwx`
  (`BASE32_RAMP`, 2^-120). The shared passphrases (`PASSPHRASE`, `PASSPHRASE_SPOKEN`) live there too.
- **Detection.** `.gitleaks.toml` allowlists exactly these values (anchored regexes, AND crate
  source/tests/goldens paths); any other secret-shaped value in a test is reported.
- **Gate.** `cargo xtask check-test-fixtures` fails on a secret-shaped literal in Rust or
  TypeScript test code (a high-entropy value bound to a credential name, mirroring gitleaks
  `generic-api-key`, or a provider token / PEM private-key header). Add a new value to the
  crate, carrying the marker, and to the gitleaks allowlist; the frontend suites generate
  passphrases at run time (`e2e/app/user.ts`) and keep no static ones.
