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
- **Privilege isolation** — the daemon holds the vault
  (`rekindle-vault`) and long-term keys; each client only holds the
  credentials needed for the operations it performs.
- **Cleaner restart story** — desktop UI can crash or be force-quit
  without losing the network state, presence, or in-flight transfers.

The desktop app does not yet use the daemon — it embeds Veilid
directly. The daemon track is the chiral-network-aligned direction for
out-of-process clients and may eventually become the substrate for the
desktop shell as well.

## Three crates

```
              ┌────────────────────────────┐
              │ rekindle (CLI), rekindle-tui│  frontends, over
              │ via rekindle-client        │   rekindle-client
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
other module — including all of `rekindle-node` and every frontend —
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

Public re-exports include `TransportNode` (its routes via
`TransportNode::own_routes`), `Sender`,
`PeerRegistry`, `DhtStore`, `Session`, `QueryEngine`,
`SignalSessionManager`, plus the per-feature operation modules.

## `rekindle-ipc` — the bus

Tier 3, `#![forbid(unsafe_code)]`. No Veilid, no storage: every frontend
and the daemon link it.

```
crates/rekindle-ipc/src/
├── server/              bus server (accept loop, per-connection Noise, routing)
├── client.rs            BusClient (consumed by rekindle-cli; later rekindle-client)
├── transport.rs         platform listener/stream, peer credentials, socket path
├── framing.rs           u16 BE per Noise message; reader task per connection
├── noise.rs             Noise IK handshake (Noise_IK_25519_ChaChaPoly_BLAKE2s)
├── noise_keys.rs        bus key files, agent-name validation
├── protocol/            IpcRequest / IpcResponse enums (exhaustive match)
├── registry.rs          ClearanceRegistry (shared with the host's dispatch)
├── event_router.rs      push events to subscribed clients
├── event_source.rs      follow the subscription source across lock cycles
├── media_channel.rs     drop-oldest media queue
└── message.rs           Message<T> envelope, AgentType, SecurityLevel
```

**Local transport and access control** (`transport.rs`):

| Platform | Transport | Who may connect |
|---|---|---|
| Linux | `$XDG_RUNTIME_DIR/rekindle/daemon.sock` | `0700` dir, `0600` socket, and every peer's UID (`SO_PEERCRED`) must equal the daemon's |
| macOS | `~/Library/Application Support/rekindle/daemon.sock` | the same; tokio reads the peer's UID with `getpeereid` and its PID with `LOCAL_PEEREPID` |
| Windows | `\\.\pipe\rekindle-<tag of the user profile>` | the pipe's protected DACL `D:P(A;;GA;;;OW)(A;;GA;;;SY)` — the pipe's owner and LocalSystem only — checked by the OS at open; remote clients refused. Bus key files in `%LOCALAPPDATA%\rekindle\` |

The Windows side goes through `interprocess`'s safe security-descriptor
API, so the crate has no `unsafe`. Microsoft documents the descriptor as
what controls access to both ends of a pipe; the default one grants
Everyone read access, which is why it is replaced. Research:
`.claude/plans/standards-remediation/evidence/c1-windows-ipc-research.md`.

## `rekindle-node` — the daemon (`rekindled`)

```
crates/rekindle-node/src/
├── lib.rs               crate-level docs
├── bin/rekindled.rs     the `rekindled` binary
├── host/                composition root
│   ├── mod.rs           run(): lock, policy, bus key, transport, context, bus
│   ├── lock.rs          NodeLock — one rekindled per data root
│   ├── policy.rs        admin policy: one fail-closed loader (startup + PolicyReload)
│   ├── bus_key.rs       bus Noise keypair with tamper detection
│   └── watchdog.rs      sd_notify READY=1 + watchdog
├── validation.rs        input validation
├── daemon/              lifecycle + RPC handlers
│   ├── mod.rs           DaemonState state machine
│   ├── handler.rs       top-level request dispatch
│   ├── community_rpc.rs / governance_rpc.rs
│   ├── friend_inbox.rs  inbound friend-request queue
│   └── dispatch/        per-operation dispatch tables
└── state/               session, config, paths
```

**One daemon per data root.** `host::run` takes `NodeLock` on
`<state>/node.lock` before touching the vault, `veilid/` or the bus: a
second `rekindled` exits with "another rekindled is running on <root>
(pid N)" and never reaches `BusServer::bind`, so it cannot take a live
daemon's socket. The lock is `std::fs::File::try_lock` (`flock` /
`LockFileEx`), released when the process ends, crash included.

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
       │   LOCKED   │  Vault not unlocked; secrets not in memory
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

### IPC bus (Noise IK over the local socket)

```
Client                                    Daemon (rekindled)
──────                                    ──────────────────
connect to the per-user socket / pipe
                                          peer credentials → UID (PID if the OS gives one)
                                          refuse a UID other than the daemon's
        ◀──── Noise IK handshake (1 RTT) ────▶
                                          prologue: "REKINDLE-IPC-v2" ‖ lo_uid ‖ hi_uid
                                                    ‖ socket path   (no PIDs)
                                          registry lookup: Noise static key → name, clearance
                                                   │
                                                   ▼
                                          transport: ChaCha20-Poly1305 + BLAKE2s
                                          each Noise message ≤ 65535 bytes behind a
                                          u16 big-endian length (Noise §13); a frame's
                                          total length (≤ 16 MiB) is authenticated in
                                          its first message before anything is allocated
```

Pattern: **`Noise_IK_25519_ChaChaPoly_BLAKE2s`** via `snow`.

| Property | Mechanism |
|----------|-----------|
| Mutual authentication | Noise IK: the initiator's static key is transmitted, the responder's is known in advance |
| Forward secrecy | Ephemeral DH per session |
| Confidentiality | ChaCha20-Poly1305 AEAD |
| Local binding | Both UIDs and the socket path are in the prologue, so a handshake relayed to another user's or another path's bus fails. PIDs are not: a PID-namespaced client has none to give |
| Cancel safety | A reader task per connection owns the socket's read half and hands whole ciphertext messages over a channel; snow advances the receive nonce only on a successful decrypt |
| Sender stamping | The server overwrites `verified_sender_name` and `verified_sender_key` (the connection's Noise static key) on every routed frame; `WIRE_VERSION` 2, other versions are dropped |
| Response routing | A correlated frame is accepted only from the daemon's own connection, so a client cannot answer another client's request |
| Rate limit | 100 requests/sec per connection, refill bucket |
| Handshake DoS protection | 5-second handshake timeout |

The daemon's long-term Ed25519 key lives in the OS keyring (`keyring`
crate) until plan D1 moves it into the vault.

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

**`MEKRotate` answers `{ "queued": true, … }`, not a generation.** MEK
distribution is a per-recipient `app_call` run by the rotation worker
(`daemon::mek_rotation`), so a synchronous reply would either block the
handler on the slowest peer or report a generation that had not reached
anybody. Clients read the outcome from `CryptoEvent::MekRotated`, the
same event a departure-triggered rotation emits. Any frontend that
waited on the response for a generation number needs to subscribe
instead.

Variants containing secrets (`Unlock`, `IdentityCreate`) have custom
`Debug` impls that redact sensitive fields. The audit log records only
`IpcRequest::name()`, never a field.

**Validation.** `rekindle-node/src/validation.rs::validate_request` checks
every field of every variant once, in `router::dispatch`, before any
handler runs (exhaustive, no wildcard). Keys use
`rekindle_types::key_format`; free text has caps taken from documented
platform limits (message 2000, topic 1024, reason/note 512, description
255, status 100 — sources in
`.claude/plans/standards-remediation/evidence/c2-ipc-robustness-research.md`).
A malformed key is a 400, never a panic: `clippy::string_slice` is denied
in every crate on this path.

**Concurrency.** Every request runs as its own task. `IpcRequest::lane()`
puts it in one of three lanes: `Query` (no lock), `Write` (shared) or
`Exclusive` (Unlock, Lock, Shutdown, Identity Create/Rotate/Destroy/Wipe),
which waits for in-flight writes and holds the lane alone. tokio's
`RwLock` is FIFO-fair, so a queued `Exclusive` holds back later writes.

**A panicking handler** answers its own request 500, then the daemon
shuts down gracefully and exits 70 (`EX_SOFTWARE`) for its supervisor to
restart it. Shared state behind a lock whose holder panicked "is likely
tainted" (Rust `std::sync::poison`), and the daemon's locks do not
poison, so continuing would serve from half-mutated state.

**Caller context.** Handlers receive `CallerContext { verified_name,
static_key, level }`, all server-stamped. `AgentRegister` registers the
caller's own Noise key (effective on its next connection).

### Subscription / event push

Beyond the request-response surface, clients subscribe to streaming
events (incoming chat messages, presence updates, voice events, governance
changes) via `SubscriptionFilter`. The bus server keeps a list of filters
per connection and the `daemon::event_router` fans events out to every
matching client.

`MAX_FILTERS_PER_CONNECTION` caps subscription cost per client.

Both the bus's event delivery and the daemon's tier-3 inbox consumer
follow the event source with `rekindle_ipc::event_source::follow`: unlock
publishes the `SubscriptionManager` broadcast, lock clears it, and each
follower re-subscribes after every unlock.

### Lock, unlock and shutdown

`teardown_unlocked` releases everything an unlock created (presence polls,
subscription loops, the broadcast mesh, cached channel keys, the event
source, then the signing key) and is shared by Lock, a failed Unlock,
Destroy, Wipe and process exit. Lock while locked answers 409; an Unlock
whose resume fails returns to Locked and answers 503.

Shutdown (IPC `Shutdown`, SIGTERM, SIGINT, or a handler panic) runs in
this order, within systemd's `TimeoutStopSec=15`:

1. `STOPPING=1`;
2. the bus subscriber stops reading and answers every request in flight
   (5 s; anything still running is answered 503);
3. the bus server stops accepting, waits for the daemon's connection to
   close, then closes each client once its queued replies are written
   (2 s) — this is how `node stop` gets its answer;
4. the workers stop at their next await (2 s);
5. `teardown_unlocked`, then the transport;
6. after a Wipe, the Veilid storage is deleted, now that the transport
   no longer holds it open.

| Exit | Meaning | systemd |
|---|---|---|
| 0 | stop requested | not restarted (clean exit) |
| 1 | startup failed: transport, audit log, node lock | restarted (`on-failure`) |
| 70 `EX_SOFTWARE` | a handler panicked | restarted from fresh state |
| 75 `EX_TEMPFAIL` | identity destroyed or data wiped | restarted empty (`RestartForceExitStatus=75`) |
| 78 `EX_CONFIG` | unparsable policy; incomplete, malformed or tampered bus key | not restarted (`RestartPreventExitStatus=78`) |

## The frontends: `rekindle-client`, `rekindle` and `rekindle-tui`

Plan C3 split the old CLI-with-a-TUI-feature into three crates with no
features. `cargo xtask check-frontend-boundaries` walks each one's
resolved normal-dependency graph and fails if any reaches `veilid-core`,
`rekindle-transport`, `rekindle-protocol`, `rekindle-vault`,
`rekindle-db`, `rekindle-node` or `rekindle-desktop`.

| Crate | Binary | Holds |
|---|---|---|
| `rekindle-client` | — | `DaemonClient` (requests, `subscribe`, the typed event stream), `spawn::{connect_or_spawn, start_detached, run_foreground}`, `config::load`, `fmt` formatters, `term::use_unicode`, `log::init` (scrubbed file log), `ClientError`, `CLI_BIN`/`cli_cmd!` |
| `rekindle-cli` | `rekindle` | clap commands, `output/` (text, JSON, JSONL), `watch.rs`, the confirmation and passphrase prompts |
| `rekindle-tui` | `rekindle-tui` | the ratatui dashboard and views, `[tui]` theming and keybindings (`keymap/`), color-eyre |

The desktop app is the `rekindle-desktop` package (product name
"Rekindle"); it joins the frontends at plan F1.

### Starting the daemon on demand

A frontend that needs `rekindled` starts it, as gpg starts gpg-agent
("automatically started on demand … no reason to start it manually",
GnuPG manual). `connect_or_spawn` connects if a daemon answers, otherwise
runs the `rekindled` beside the frontend's own executable and waits for a
`Status` answer. Until the daemon's own subscriber connects, the bus
answers every request 503 "daemon starting" rather than dropping it, and
`READY=1` is sent only once the subscriber is connected. If two frontends
start the daemon at once, the node lock lets one run; the other's
`rekindled` exits and that frontend waits for the winner.

The CLI starts the daemon for every command except `status` (which
reports a stopped daemon honestly) and `node stop`.

```
$ rekindle friend list                 # starts rekindled if needed
$ rekindle node start                  # detached; returns once it answers
$ rekindle node start --foreground     # becomes `rekindled` in this terminal
$ rekindle channel watch -c Dev -C general   # stream until Ctrl-C
$ rekindle --format jsonl dm watch     # one JSON event per line
$ rekindle-tui                         # the terminal UI
$ rekindled                            # or run the host directly
```

`channel watch`, `dm watch` and `voice join --watch` subscribe on the bus
and print each matching event until Ctrl-C. Peer-controlled text goes
through `sanitize_for_display` before it reaches the terminal.

### Configuration

One `config.toml` schema (`rekindle_types::config::user::UserConfig`)
serves every reader: `rekindled` takes `[network]` (the transport
configuration itself), `rekindle-tui` takes `[tui]`, and `rekindle config
validate` checks all of it. Every table rejects unknown keys.

Layers, lowest precedence first (`rekindle_utils::config_layers`):
`/etc/rekindle/config.toml` (`%ProgramData%\rekindle` on Windows), its
`config.d/*.toml`, the user file in the data root's config dir
(`DataRoot::config`, `<config dir>/com.rekindle.app`, the folder the desktop
uses), its
`config.d/*.toml`, `$REKINDLE_CONFIG`, then `--config`. Layers merge by key
presence, like systemd drop-ins: a later layer can set a value back to its
default, and a key it leaves out is untouched. An invalid config stops
`rekindled` with exit 78. `policy.toml` lives in the same system and user
dirs.

Every frontend command builds an `IpcRequest`, sends it, and renders the
response. No frontend touches `TransportNode`, `Session` or the OS
keyring.

Input follows clig.dev:

- prompts appear only when stdin is a terminal and `--no-input` was not
  passed; otherwise the command fails and names the flag to use;
- `unlock` and `init` read the passphrase from a no-echo prompt or from
  `--passphrase-file <PATH>` (`-` is stdin), never from a flag value or
  the environment;
- severe actions (`identity destroy`, `init --wipe-all-data`) are
  confirmed by typing the phrase, or with `--confirm="<phrase>"`; the
  daemon checks the phrase;
- other confirmations are skipped with `--force`.

Exit codes: 0 success, 1 error, 2 timeout, 3 auth, 4 daemon state (409,
503), 69 (`EX_UNAVAILABLE`) for a command that is not implemented yet.
With `--format json|jsonl` an error is printed on stdout as
`{"error": {"message", "daemon_code", "remediation", "exit_code"}}`.
Hints name the binary as built (`env!("CARGO_BIN_NAME")`); the daemon's
own remediation names an action, not a command, since the GUI reads it
too.

`#![deny(clippy::print_stdout)]` is enforced in the CLI: every output
goes through the `output/` renderers (JSON or comfy-table) so machine
consumers and humans get equivalent data.

## systemd integration

`crates/rekindle-node/contrib/systemd/rekindled.service` (the Docker test
harness ships the same unit under `.config/docker/`):

- `Type=notify`, `ExecStart=rekindled` in the foreground — with
  `WatchdogSec=` set, `NotifyAccess` defaults to `main`, so the notifying
  process must be the one systemd started;
- no `ExecStop=`: systemd's SIGTERM triggers the drain above;
- **`READY=1`** once the bus is bound and the daemon is Locked;
- **watchdog** pings every `$WATCHDOG_USEC / 2` (`sd_watchdog_enabled(3)`)
  while the bus subscriber's heartbeat is fresh; a stalled subscriber
  sends `WATCHDOG=trigger`;
- `RestartForceExitStatus=75`, `RestartPreventExitStatus=78`, and
  `StartLimitBurst=5` per 300 s bound a crash loop.

## Where to look

| Concern | File |
|---------|------|
| Veilid public API surface | `crates/rekindle-transport/src/lib.rs` |
| Outbound Veilid I/O | `crates/rekindle-transport/src/broadcast/` |
| Inbound Veilid I/O | `crates/rekindle-transport/src/subscriptions/` |
| Per-feature operations | `crates/rekindle-transport/src/operations/` — `{channel,dm,friend,voice,mek,presence,roles,moderation,invites,identity}.rs` plus the `community/` and `calls/` directories |
| Channel messages on the wire | `crates/rekindle-transport/src/broadcast/dht/channel_smpl.rs` — one SMPL record per `(channel, segment)`, each member writing their own slot subkey. Replaced a per-member DFLT `DhtLog` announced through the registry member index |
| Channel lifecycle | `crates/rekindle-governance-runtime/src/channels.rs` — record + `ChannelCreated` in one call, shared by daemon and desktop |
| Daemon entry / lib | `crates/rekindle-node/src/lib.rs` |
| Lifecycle state machine | `crates/rekindle-node/src/daemon/mod.rs` |
| Host composition root | `crates/rekindle-node/src/host/mod.rs` |
| Node lock | `crates/rekindle-node/src/host/lock.rs` |
| Admin policy | `crates/rekindle-node/src/host/policy.rs` |
| IPC server | `crates/rekindle-ipc/src/server/` |
| Noise IK handshake | `crates/rekindle-ipc/src/noise.rs` |
| `IpcRequest` / `IpcResponse` | `crates/rekindle-ipc/src/protocol/` |
| Local transport, peer credentials | `crates/rekindle-ipc/src/transport.rs` |
| Bus key files | `crates/rekindle-ipc/src/noise_keys.rs`, `crates/rekindle-node/src/host/bus_key.rs` |
| Request validation | `crates/rekindle-node/src/validation.rs` |
| Per-request tasks, panic policy | `crates/rekindle-node/src/daemon/dispatch/{in_flight,context}.rs` |
| Shutdown signal, exit reasons | `crates/rekindle-node/src/daemon/shutdown.rs`, `host/signals.rs` |
| Watchdog | `crates/rekindle-node/src/host/watchdog.rs`, `daemon/heartbeat.rs` |
| systemd unit | `crates/rekindle-node/contrib/systemd/rekindled.service` |
| CLI entry | `crates/rekindle-cli/src/main.rs` |
| TUI | `crates/rekindle-tui/src/` |
| Daemon connection, on-demand start | `crates/rekindle-client/src/{client,spawn}.rs` |
| Config schema and layers | `crates/rekindle-types/src/config/`, `crates/rekindle-utils/src/config_layers.rs` |
| Frontend boundary gate | `xtask/src/frontend_boundaries.rs` |
