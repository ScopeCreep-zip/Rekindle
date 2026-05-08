# Daemon and CLI Architecture

Rekindle ships **two ways** to run the protocol stack:

1. **Tauri desktop app** (`src-tauri/`) — links Veilid in-process and
   serves the SolidJS UI. This is the primary user-facing build today.
2. **Daemon + CLI** (`rekindle-node` + `rekindle-cli`) — a long-running
   daemon owns the Veilid node and exposes it over an encrypted IPC bus.
   The first IPC client is a `clap`-based CLI with an optional
   `ratatui` TUI.

Both speak the same Veilid protocol, the same SMPL governance, the same
Signal/MEK encryption. They differ only in how the local process is
structured. This document covers the daemon track. For the desktop
shell, see [`tauri-backend.md`](tauri-backend.md).

## Why a separate daemon

A single, long-running process that owns the Veilid node enables:

- **Headless deployments** — servers, automation, bots, bridges, CI runners.
- **Multiple frontends sharing one node** — desktop UI + CLI + future
  mobile or web client all driving the same identity, presence, and
  routes.
- **Privilege isolation** — the daemon holds the Stronghold vault and
  long-term keys; each client only holds the credentials needed for
  the operations it performs.
- **Cleaner restart story** — desktop UI can crash or be force-quit
  without losing the network state, presence, or in-flight transfers.

The desktop app does not yet use the daemon — it embeds Veilid
directly. The daemon track is the chiral-network-aligned direction for
out-of-process clients and may eventually become the substrate for the
desktop shell as well.

## Three crates

```
              ┌────────────────────────────┐
              │ rekindle-cli               │   clap + ratatui
              │ (CLI / TUI client)         │
              └─────────────┬──────────────┘
                            │ IPC (Noise IK over Unix socket)
                            ▼
              ┌────────────────────────────┐
              │ rekindle-node              │   daemon — owns Veilid,
              │ (daemon)                   │   serves IPC bus,
              │  ipc/ daemon/ state/       │   manages session
              └─────────────┬──────────────┘
                            │ rekindle-transport public API
                            ▼
              ┌────────────────────────────┐
              │ rekindle-transport         │   sole Veilid boundary;
              │ (sole Veilid boundary)     │   broadcast/ + subscriptions/
              │  broadcast/ subscriptions/ │   are the only veilid_core
              │  operations/ payload/ …    │   importers.
              └─────────────┬──────────────┘
                            │
                            ▼
                       Veilid network
```

A hard rule on this track: **only `rekindle-transport::broadcast/` and
`rekindle-transport::subscriptions/` import `veilid_core`.** Every
other module — including all of `rekindle-node` and `rekindle-cli` —
talks to Veilid through the `TransportNode` / `Sender` / `Session` /
`InboundHandler` / `QueryEngine` API exposed at the crate boundary.
This keeps the Veilid version surface centralised and lets the rest of
the daemon track be tested without a running Veilid node.

## `rekindle-transport` — the unified Veilid boundary

```
crates/rekindle-transport/src/
├── broadcast/         outbound: sends, DHT writes, route management,
│                      node lifecycle (only outbound veilid_core importer)
├── subscriptions/     inbound: event dispatch, DHT watches, value-change
│                      routing (only inbound veilid_core importer)
├── operations/        per-feature wrappers (community, channel, dm,
│                      friend, voice, mek, presence, roles,
│                      moderation, invites, identity)
├── payload/           wire payload helpers, Cap'n Proto adapters
├── crypto/            pseudonym derivation, prekeys, Signal session
├── community/         per-community state types
├── session/           Session, SessionIdentity, CommunityMembership
├── gossip.rs          GossipMesh, OnlineMember, DedupCache, LamportClock
├── frame.rs           wire framing
├── handler.rs         InboundHandler trait + TransportEvent
├── query.rs           QueryEngine (read-side aggregation)
├── config.rs          TransportConfig, SafetyConfig, SafetyProfile
├── shared.rs          SharedState, AttachmentState, snapshots
└── error.rs           TransportError
```

Public re-exports include `TransportNode`, `Sender`, `RouteManager`,
`PeerRegistry`, `DhtStore`, `Session`, `QueryEngine`,
`SignalSessionManager`, plus the per-feature operation modules.

## `rekindle-node` — the daemon

```
crates/rekindle-node/src/
├── lib.rs               crate-level docs + re-exports
├── validation.rs        input validation
├── ipc/                 encrypted IPC bus
│   ├── server.rs        bus server (UnixListener)
│   ├── client.rs        client side (consumed by rekindle-cli)
│   ├── transport.rs     UCred extraction, socket-path resolution
│   ├── framing.rs       length-prefixed frames
│   ├── noise.rs         Noise IK handshake (Noise_IK_25519_ChaChaPoly_BLAKE2s)
│   ├── noise_keys.rs    OS keyring storage for the daemon long-term key
│   ├── protocol.rs      IpcRequest / IpcResponse enums (exhaustive match)
│   ├── registry.rs      connected-client registry, UCred-pinned
│   └── message.rs       Message<T> envelope, AgentType, SecurityLevel
├── daemon/              lifecycle + RPC handlers
│   ├── mod.rs           DaemonState state machine
│   ├── handler.rs       top-level request dispatch
│   ├── community_rpc.rs / governance_rpc.rs
│   ├── friend_inbox.rs  inbound friend-request queue
│   ├── event_router.rs  push events to subscribed clients
│   └── dispatch/        per-operation dispatch tables
└── state/               session, config, paths
```

### Lifecycle state machine

```
       ┌────────────┐
       │  STOPPED   │  no socket, no Veilid node
       └─────┬──────┘
             │ daemon launches
             ▼
       ┌────────────┐
       │  STARTING  │  Veilid bootstrapping; socket created; limited cmds
       └─────┬──────┘
             │ network ready
             ▼
       ┌────────────┐
       │   LOCKED   │  Stronghold not unlocked; secrets not in memory
       └─────┬──────┘
             │ Unlock(passphrase)
             ▼
       ┌────────────┐
       │  RESUMING  │  reopening DHT records, warming MEK cache
       └─────┬──────┘
             │ all subsystems ready
             ▼
       ┌────────────┐         ┌────────────┐         ┌────────────┐
       │ OPERATIONAL│ ←─────  │  DEGRADED  │ ←─────  │  DETACHED  │
       └─────┬──────┘         └────────────┘         └────────────┘
             │ Lock                  ▲                     ▲
             ▼                       │                     │
       ┌────────────┐                │                     │
       │  LOCKING   │   route died, MEK stale,        network lost,
       └─────┬──────┘   auto-recovering.              serving cached data,
             │ secrets zeroed                          queuing writes.
             ▼
       ┌────────────┐
       │   LOCKED   │
       └────────────┘

       Shutdown → SHUTTING_DOWN → STOPPED
```

The state determines which `IpcRequest` variants are available at any
moment. The `can_query()`, `can_write()`, `can_unlock()` methods on
`DaemonState` are the canonical capability checks — they are not just
discriminant comparisons.

### IPC bus (Noise IK over Unix socket)

```
Client                                    Daemon (rekindle-node)
──────                                    ──────────────────────
Connect to /run/user/<uid>/rekindle.sock
                            ───tcp───▶
                                          extract_ucred(stream) → (PID, UID)
                                          look up agent in registry
                                                   │
                                                   ▼
                                          Noise IK handshake
        ◀──── 1 round-trip handshake ────▶
                                          prologue mixes UCred:
                                          REKINDLE-IPC-v1:lo_pid:lo_uid:hi_pid:hi_uid
                                                   │
                                                   ▼
                                          forward-secret transport
                                          ChaCha20-Poly1305 + BLAKE2s
                                          frames up to 16 MiB (chunked)

IpcRequest::FriendList   ───▶  daemon::dispatch  ───▶  rekindle-transport::QueryEngine
                         ◀─── IpcResponse ──────────────
```

Pattern: **`Noise_IK_25519_ChaChaPoly_BLAKE2s`** via `snow`.

| Property | Mechanism |
|----------|-----------|
| Mutual authentication | Noise IK — initiator's static key transmitted, responder's static key pre-known |
| Forward secrecy | Noise transport uses ephemeral DH per session |
| Confidentiality | ChaCha20-Poly1305 AEAD |
| OS-binding | UCred (`SO_PEERCRED` on Linux, `LOCAL_PEERCRED` on macOS) mixed into the Noise prologue |
| Anti-confused-deputy | UCred binding cryptographically prevents a different process from MITM-ing the bus |
| Rate limit | 100 requests/sec per connection, refill bucket |
| Frame chunking | Noise's 65535-byte transport limit handled with chunk-count headers |
| Handshake DoS protection | 5-second handshake timeout |

`SO_PEERCRED` extraction is platform-gated `#[cfg(unix)]` — both Linux
and macOS get it via tokio's Unix socket support. Windows uses a named
pipe with named-pipe credentials.

The daemon's long-term Ed25519 key lives in the OS keyring (`keyring`
crate) — Apple Keychain on macOS, Windows Credential Manager on Windows,
Secret Service on Linux.

### IpcRequest dispatch

`IpcRequest` is a single Rust enum with one variant per supported
operation: `Unlock`, `Lock`, `Status`, `Shutdown`, `IdentityCreate`,
`IdentityShow`, `IdentityRotate`, `FriendAdd`, `FriendAccept`,
`FriendRemove`, `FriendList`, `CommunityCreate`, `CommunityJoin`,
`CommunityLeave`, `ChannelCreate`, `ChannelMessage`, `MEKRotate`,
`VoiceJoin`, `VoiceLeave`, …

The dispatcher is an **exhaustive match** with no wildcard arm — adding
a variant forces a handler implementation in
`daemon::dispatch::dispatch()`. Variant naming is `{Domain}{Verb}`
(e.g., `ChannelCreate`, `FriendAdd`) so the match arms are
self-documenting.

Variants containing secrets (`Unlock`, `IdentityCreate`) have custom
`Debug` impls that redact sensitive fields.

### Subscription / event push

Beyond the request-response surface, clients subscribe to streaming
events (incoming chat messages, presence updates, voice events, governance
changes) via `SubscriptionFilter`. The bus server keeps a list of filters
per connection and the `daemon::event_router` fans events out to every
matching client.

`MAX_FILTERS_PER_CONNECTION` caps subscription cost per client.

## `rekindle-cli` — the first client

```
crates/rekindle-cli/src/
├── main.rs              clap entry point
├── cli/                 clap subcommand definitions
├── tui/                 ratatui interactive mode (feature `tui`)
├── views/               TUI screens
├── output/              JSON / table renderers
├── config/              config file loading (toml)
├── transport.rs         IPC client wrapper (over rekindle-node::ipc::client)
├── node_daemon.rs       (feature `daemon`) embedded daemon mode
├── identity.rs / keys.rs / network.rs / presence.rs
├── friends.rs / dm.rs / community.rs / channel.rs / governance.rs / voice.rs
├── helpers.rs / error.rs
```

### Binary name

The CLI binary is named **`rekindle-cli`** (renamed from `rekindle` to
avoid colliding with the desktop app's binary under `src-tauri/`,
which is named `rekindle` and produces `Rekindle.app` etc.).

### Default features

| Feature | Default | Adds |
|---------|---------|------|
| `tui` | yes | `ratatui`, `crossterm`, `ratatui-textarea`, `arboard` — interactive screen-based UI |
| `daemon` | yes | embeds the daemon in the same binary for solo-developer workflows; pulls in `rekindle-transport`, `snow`, `sd-notify`, `rustix`, `postcard` |

Both can be disabled for a minimal CLI-only build:

```
cargo build -p rekindle-cli --no-default-features --features=""
```

### Operating modes

```
$ rekindle-cli --help
... usage ...

# One-shot CLI commands (default)
$ rekindle-cli friend list
$ rekindle-cli community join <invite>
$ rekindle-cli channel send #general "hello"

# Interactive TUI
$ rekindle-cli tui

# Embedded daemon mode (feature=daemon, useful in dev)
$ rekindle-cli daemon start --foreground
```

Every CLI command builds an `IpcRequest`, dials the bus, sends, awaits
the `IpcResponse`, and renders. The CLI never touches `TransportNode`,
`Session`, or the OS keyring directly — that strict boundary makes the
CLI safe to run as a non-privileged user even when the daemon holds
elevated credentials.

`#![deny(clippy::print_stdout)]` is enforced in the CLI: every output
goes through the `output/` renderers (JSON or comfy-table) so machine
consumers and humans get equivalent data.

## systemd integration

On Linux, `rekindle-node` integrates with systemd:

- **`READY=1`** sent via `sd_notify` once the daemon reaches `LOCKED`
  (network attached, socket bound, ready to accept clients).
- **Watchdog keepalive** at the configured `WatchdogSec` interval —
  systemd will restart the daemon if it stops sending watchdog pings,
  catching deadlocks that don't crash the process.

This makes `rekindle-node` deployable as a per-user `systemd --user`
service or a system-level service for headless deployments.

## Where to look

| Concern | File |
|---------|------|
| Veilid public API surface | `crates/rekindle-transport/src/lib.rs` |
| Outbound Veilid I/O | `crates/rekindle-transport/src/broadcast/` |
| Inbound Veilid I/O | `crates/rekindle-transport/src/subscriptions/` |
| Per-feature operations | `crates/rekindle-transport/src/operations/{community,channel,dm,friend,voice,mek,presence,roles,moderation,invites,identity}.rs` |
| Daemon entry / lib | `crates/rekindle-node/src/lib.rs` |
| Lifecycle state machine | `crates/rekindle-node/src/daemon/mod.rs` |
| IPC server | `crates/rekindle-node/src/ipc/server.rs` |
| Noise IK handshake | `crates/rekindle-node/src/ipc/noise.rs` |
| `IpcRequest` / `IpcResponse` | `crates/rekindle-node/src/ipc/protocol.rs` |
| UCred extraction | `crates/rekindle-node/src/ipc/transport.rs` |
| Long-term key OS keyring | `crates/rekindle-node/src/ipc/noise_keys.rs` |
| CLI entry | `crates/rekindle-cli/src/main.rs` |
| TUI screens | `crates/rekindle-cli/src/tui/`, `crates/rekindle-cli/src/views/` |
| Embedded-daemon mode | `crates/rekindle-cli/src/node_daemon.rs` |
