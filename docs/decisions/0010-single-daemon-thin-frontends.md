# 0010 — One daemon owns the node; GUI, TUI and CLI are thin frontends

- **Status:** Accepted (gated: the GUI switches to a bus client only after the IPC hardening step
  C2 of the standards-remediation plan lands, the condition ADR-0005 set)
- **Date:** 2026-10-04
- **Supersedes:** the "Future direction" section of [0005](0005-daemon-cli-track.md)

## Context and problem statement

[0005](0005-daemon-cli-track.md) added a daemon + CLI track **next to** the Tauri app, and left
open whether "the Tauri desktop app may eventually migrate to the daemon model". Its decision
drivers already named the goal: "Multiple frontends sharing one node" and "the process that holds
long-term keys should be smaller than the process that paints UI".

Verification on 2026-10-04 (`.claude/plans/standards-remediation/evidence/frontend-decoupling.md`
§1) found the two tracks have become two separate products:

- The GUI embeds about 60.5k LoC of backend behind 258 Tauri commands. The daemon has about 35k
  LoC behind 65 `IpcRequest` variants.
- Running the GUI and the TUI together starts **two Veilid nodes, two storage roots and two
  unrelated identities**.
- The daemon exists only inside the `rekindle-cli` binary, and `rekindle-node` has no `[[bin]]`.
  Every CLI build compiles `veilid-core`.
- `BusServer::bind` unlinks a live daemon's socket, so a second daemon silently takes it over.
- The IPC layer uses `UnixListener` unconditionally, so the daemon cannot run on Windows.
- Release builds publish only the GUI.
- Most of the daemon's protocol paths are scaffolding:
  - no production PQXDH or ratchet;
  - write-only prekey, MEK and governance persistence;
  - a no-op voice path.

  Every cross-cutting fix therefore has to be written twice, or the tracks drift further apart.

The owner's requirement is that users pick which frontends to install (GUI only, TUI only, CLI
only, or any combination) and that every frontend behaves identically.

## Decision drivers

- **One identity, one node per machine.** Frontends share state; they never fork it.
- **Privilege boundary.** Long-term keys stay out of the process that hosts WebKit and
  webview-reachable commands (ADR-0005 driver).
- **Fix once.** Protocol and storage fixes land in one implementation.
- **Install choice.** Each frontend is an independent package.
- **No fallback paths.** At any moment exactly one process hosts the node.

## Considered options

### Option A — One daemon, every frontend is a client (selected)

`rekindled` owns Veilid, the vault and the per-identity database, behind an exclusive lock on
the data root. The GUI, TUI and CLI talk to it over the Noise IPC bus.

### Option B — Every frontend embeds the core library

The GUI and TUI each link the backend. When both run, one must host and the other attach, which
needs a "who hosts" election; if the hosting GUI quits, the TUI loses its node.

### Option C — Keep two tracks (status quo)

## Decision outcome

**Chose Option A.**

The precedents that share one identity across frontends all use a single daemon with a data-root
lock:

- **Tailscale:** `tailscaled` plus CLI and GUI clients.
- **Syncthing:** a `flock` on the data root (`cmd/syncthing/main.go:407-420`).
- **Docker:** `dockerd` plus the CLI.
- **mpd:** "the separate client and server design allows users to choose a user interface …
  independently of the underlying daemon".
- **signal-cli:** daemon mode plus a file lock.

The embedded-core precedents (Matrix SDK clients, librespot, Transmission) work only because each
frontend is a separate device or keeps separate state, which contradicts the requirement.
Distribution follows the same split:
- Debian ships one package per frontend that depends on a core or daemon package, plus a
  metapackage (transmission, weechat, docker);
- Homebrew uses a formula for the daemon and CLI plus a cask for the app.

Sources: `evidence/frontend-decoupling.md` §2 and `evidence/external-facts.md`, with raw copies
in `evidence/raw/`.

### Shape

- **`rekindle-ipc`** (Tier 3) holds the protocol, Noise, framing, client, server, `EventRouter`
  and media channel. It depends only on `rekindle-types`.
- **`rekindle-node`** is the only backend host: library plus `[[bin]] rekindled`.
  - `host::NodeLock` uses `flock` (`LockFileEx` on Windows) on the data root before the bus binds.
  - Named-pipe transport with a peer-SID check on Windows.
- **`rekindle-client`** holds `DaemonClient` and `connect_or_spawn`, which spawns the `rekindled`
  next to the frontend binary.
- **Frontends:** `rekindle-cli` (bin `rekindle`), `rekindle-tui` (bin `rekindle-tui`) and
  `rekindle-desktop` (Tauri, product "Rekindle"). No frontend crate may depend on `veilid-core`,
  `rekindle-transport`, `rekindle-protocol`, `rekindle-vault` or `rekindle-db`; xtask
  `check-frontend-boundaries` enforces this.
- **No Cargo features select frontends.** A variant is the set of installed packages.
- **Version coupling is strict.** The handshake carries `(WIRE_VERSION, build_id)`, and a mismatch
  is rejected with remediation text. There is no compatibility path (pre-release).

### Migration (strangler fig)

The migration is incremental, following Fowler's StranglerFigApplication (2024-08-22): identify
seams, move behaviour piece by piece, and accept transitional architecture "that will go away once
the modernization is complete".

1. The daemon's composition root becomes `rekindle_node::Host`.
2. The GUI runs that same `Host` in-process, holding the lock and serving the bus, so the TUI and
   CLI attach to the GUI's node.
3. Domains move from `src-tauri/src/services/` into tier crates and Host dispatch one at a time.
   Each move deletes the daemon's weaker duplicate.
4. Finally the GUI drops the in-process Host, connects to a spawned `rekindled`, and loses its
   backend dependencies.

At every point exactly one process hosts the node.

The plan steps are C1–C3, D3, E1–E6 and F1–F3 in
`.claude/plans/standards-remediation/00-integration-plan.md`.

## Consequences

**Positive.**

- One identity, one node and one event stream for every frontend, and identical behaviour across
  them.
- Long-term keys leave the WebKit process.
- Protocol and storage fixes are written once; the "edit both adapters" tax disappears.
- Users install exactly the frontends they want.

**Negative.**

- About 258 desktop commands have to move behind `IpcRequest`. The same move would be needed under
  Option B.
- Media frames take one more local hop (daemon → GUI host → webview Channel). It is a Unix socket
  plus ChaCha20, negligible next to VP9 frames.
- Open items to verify at the flip:
  - macOS camera/mic (TCC) attribution when `Rekindle.app` spawns `rekindled`;
  - notarisation of the sidecar.

## Packaging

| Variant | Debian / RPM | macOS | Windows | Nix |
|---|---|---|---|---|
| Daemon | `rekindle-daemon` (`rekindled` + systemd user unit, `Type=notify`) | formula `rekindle-daemon` | inside every installer | `rekindled` |
| CLI | `rekindle-cli`, Depends `rekindle-daemon (= ver)` | formula, `depends_on "rekindle-daemon"` | dist msi/zip with `rekindled` | `rekindle-cli` |
| TUI | `rekindle-tui`, Depends `rekindle-daemon (= ver)` | formula | dist msi/zip | `rekindle-tui` |
| GUI | Tauri deb/rpm, Depends `rekindle-daemon (= ver)` | cask; `rekindled` sidecar in `Contents/MacOS` | NSIS with sidecar | `rekindle-desktop` |
| All | metapackage `rekindle` | cask + formulae | NSIS + msi | `default` |

Formats without a dependency resolver (dmg, AppImage, NSIS) bundle the sidecar. Formats with a
resolver depend on the daemon package. A second daemon copy on one machine is harmless, because
the lock admits only one and the version check rejects mismatches.

## More information

- [0005](0005-daemon-cli-track.md) — the daemon track this completes
- [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md)
- `.claude/plans/standards-remediation/evidence/frontend-decoupling.md`
- [Fowler — Strangler Fig](https://martinfowler.com/bliki/StranglerFigApplication.html)
