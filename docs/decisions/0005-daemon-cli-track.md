# 0005 — Add a daemon + CLI track alongside the Tauri desktop app

- **Status:** Accepted
- **Date:** 2026-05

## Context and problem statement

The Tauri 2 desktop app embeds Veilid in-process — the GUI process
itself owns the Veilid node, the DHT records, the Signal sessions,
and the Stronghold vault. This is the right shape for the
user-facing desktop build, but it locks out:

- **Headless deployments** — servers, automation, bots, bridges, CI
  runners.
- **Multiple frontends sharing one node** — desktop UI + CLI + future
  mobile or web client all driving the same identity, presence, and
  routes.
- **Privilege isolation** — every UI touches the long-term identity
  key directly because there is no boundary between UI and
  cryptographic state.
- **Cleaner restart story** — a desktop UI crash or force-quit drops
  network state, presence, and in-flight transfers.

We also wanted a path to cleanly enforce that **only one place** in
the codebase imports `veilid_core`. With the Tauri build, that
discipline is achievable but not architecturally enforced — many
crates can and do touch Veilid types.

## Decision drivers

- **Headless protocol-stack runtime.** Need a way to run Rekindle's
  protocol stack without a GUI process.
- **Multi-frontend.** CLI / TUI / future bridges should drive the
  same node.
- **Privilege boundary.** The process that holds long-term keys
  should be smaller than the process that paints UI.
- **Architectural enforcement of the Veilid boundary.** A single
  crate should be the only `veilid_core` importer.

## Considered options

### Option A — Status quo: Tauri-only, defer headless

Continue with Tauri-only. Add headless support later when needed.

### Option B — Add a separate `rekindle-server` binary that mirrors the Tauri shell

A second binary that runs the same `src-tauri/` code without the
WebView.

### Option C — Daemon + IPC bus + CLI client (selected)

A long-running daemon (`rekindle-node`) owns Veilid via a sole-
boundary transport crate (`rekindle-transport`). Frontends
(`rekindle-cli` for now; future others) connect over a Noise IK
encrypted IPC bus.

### Option D — Library-only "headless" mode

Refactor `src-tauri/` such that the headless logic is a library
crate, and provide both a Tauri binary and a binary that runs the
library directly with a CLI front.

## Decision outcome

**Chose Option C — daemon + IPC bus + CLI client.**

The daemon track is a parallel architecture: the desktop app keeps
its in-process Veilid for now; the daemon track is for headless
deployments, automation, CLI / TUI users, and future out-of-process
clients. Both speak the same Veilid protocol, the same SMPL
governance, the same Signal/MEK encryption — they differ only in
how the local process is structured.

The boundaries are crisp:

- **`rekindle-transport`** is the **sole `veilid_core` importer** on
  this track. Every other crate (including `rekindle-node` itself)
  talks to Veilid through `TransportNode` / `Sender` / `Session` /
  `InboundHandler` / `QueryEngine`.
- **`rekindle-node`** is the daemon. It owns the `TransportNode`,
  manages persistent state, and serves the IPC bus. The daemon's
  long-term Ed25519 key lives in the OS keyring (Apple Keychain,
  Windows Credential Manager, Secret Service).
- **`rekindle-cli`** is the first IPC client. It never touches
  `TransportNode`, `Session`, or the OS keyring directly. Every
  command sends an `IpcRequest`, awaits the `IpcResponse`, renders.

The IPC bus uses Noise IK with UCred binding — see
[`../architecture/daemon-cli.md`](../architecture/daemon-cli.md) for
the cryptographic and OS-binding properties.

## Consequences

**Positive.**

- **Architecturally enforced Veilid boundary.** `rekindle-node` and
  `rekindle-cli` cannot import `veilid_core` even if they wanted
  to — the dep graph forbids it.
- **Headless deployments** work cleanly: `rekindle-node` integrates
  with `systemd --user` (or system-level), responds to `READY=1`
  via `sd_notify`, supports watchdog keepalive.
- **Privilege isolation:** the CLI runs as the user; the daemon's
  long-term key sits in the OS keyring; the bus's UCred binding
  prevents another process from MITM-ing.
- **Multi-frontend ready:** the bus is connection-multiplexing.
  Future clients (other UIs, automation, AI/LLM agents,
  bridges, filters) all connect to the same node.
- **TUI as a first-class interaction model.** `rekindle-cli`'s
  `tui` feature gives users a `ratatui`-based screen UI for
  terminal-only environments.

**Negative.**

- **More crates to maintain.** `rekindle-transport`,
  `rekindle-node`, `rekindle-cli` are 3 substantial crates.
- **Two architectures in parallel.** Until the desktop app
  migrates to the daemon model, we maintain both code paths. This
  duplicates some service-layer logic (the Tauri shell does
  presence / sync / voice in `src-tauri/services/`; the daemon
  does similar work via `rekindle-transport::operations`).
- **Bus security is a real surface.** Noise IK + UCred binding +
  rate limiting + handshake DoS protection are all required for
  the bus to be safe — see
  [`../security/threat-model.md`](../security/threat-model.md).

**Future direction.**

- The Tauri desktop app may eventually migrate to the daemon
  model — Tauri talks to `rekindle-node` over the bus instead of
  embedding Veilid. This would unify the two tracks and remove
  the dual-architecture maintenance cost. Not committed; depends
  on whether the bus security and IPC surface are robust enough
  in practice.

## Pros and cons of the options

### Status quo (Tauri-only)

- **+** Lowest current effort.
- **−** No headless story.
- **−** Veilid boundary stays informal.

### `rekindle-server` mirroring Tauri

- **+** Low net new code.
- **−** No multi-frontend.
- **−** Veilid boundary still informal.
- **−** Doesn't solve privilege isolation.

### Daemon + IPC bus + CLI (chosen)

- **+** Architectural enforcement of the Veilid boundary.
- **+** Multi-frontend.
- **+** Privilege isolation.
- **+** Headless first-class.
- **−** More crates.
- **−** Two architectures in parallel.
- **−** Bus security surface.

### Library-only refactor

- **+** Single source of truth for protocol logic.
- **−** Doesn't solve out-of-process clients (everything is in
  the same process as whatever links the library).
- **−** Doesn't solve privilege isolation.

## More information

- [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md) — full daemon + CLI architecture
- [`../architecture/crates.md`](../architecture/crates.md) — crate listing including the daemon track
- [Noise Protocol Framework](http://noiseprotocol.org/) — IPC bus handshake
- [`../security/threat-model.md`](../security/threat-model.md) — IPC bus threat model (S4, E5)
