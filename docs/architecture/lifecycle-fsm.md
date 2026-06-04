# Lifecycle FSM

`rekindle-lifecycle` is a 9-state application FSM hoisted from the
daemon (`rekindle-node::daemon::DaemonState`) so the Tauri shell can
share the same machinery. The transition table is verbatim from the
daemon — Phase 5 of the harvest plan moved the enum into a shared
crate with no behaviour change.

The FSM tracks where the application is in its boot / login /
shutdown cycle. **Capability gates** advertise which commands are
safe to run; **`TransportGuard`** is the RAII wrapper command bodies
use to assert their capability requirement at entry.

This doc is the shared contract. The crate lives at
`crates/rekindle-lifecycle/`. The Tauri-side handle is
`AppState.lifecycle: Arc<rekindle_lifecycle::AppLifecycle>` (see
[`tauri-state.md`](tauri-state.md)). The daemon-side type is
`rekindle_node::daemon::DaemonState`, which re-exports
`LifecycleState as DaemonState` to keep its existing call sites
source-compatible.

## The nine states

| State | Meaning | `can_query` | `can_write` | `can_unlock` |
|---|---|---|---|---|
| `Stopped` | Process starting; no Veilid node attached, no vault loaded | no | no | no |
| `Starting` | Veilid bootstrapping; vault file may or may not exist | no | no | no |
| `Locked` | Network ready, vault locked (waiting for passphrase) | no | no | **yes** |
| `Resuming` | Vault unlocked; warming caches, reopening DHT records | **yes** | no | no |
| `Operational` | Everything ready — all commands available | **yes** | **yes** | no |
| `Degraded` | Route died or MEK stale; auto-recovering | **yes** | **yes** | no |
| `Detached` | Network lost; serving cached reads, queuing writes | **yes** | no | no |
| `Locking` | Zeroising secrets, closing vault | no | no | no |
| `ShuttingDown` | Graceful shutdown in progress | no | no | no |

`Degraded` is deliberately writable: the protocol tolerates writes
during route recovery (envelopes go into the
`pending_envelopes` queue and replay when the route comes back). The
distinction between `Degraded` and `Detached` is whether the network
is *technically* attached — `Detached` means Veilid has lost public
internet reachability, so writes have no peer to go to.

`Stopped` is the fail-closed default for the `LifecycleState::from_u8`
deserializer: unknown atomic values resolve to `Stopped` rather than
risking an accidental enable of writes.

## Transition table

Allowed transitions (verbatim from the daemon):

```
Stopped       → Starting
Starting      → Locked        (network up, vault file exists)
Starting      → Stopped       (init failed)
Locked        → Resuming      (passphrase accepted, vault open)
Locked        → ShuttingDown  (user quits before unlock)
Resuming      → Operational   (caches warm, watches re-armed)
Resuming      → Locking       (resume failed)
Operational   → Degraded      (route died)
Operational   → Detached      (network lost)
Operational   → Locking       (logout)
Operational   → ShuttingDown  (quit)
Degraded      → Operational   (recovery succeeded)
Degraded      → Detached      (network lost during recovery)
Degraded      → Locking       (logout while degraded)
Degraded      → ShuttingDown  (quit while degraded)
Detached      → Operational   (network restored)
Detached      → Degraded      (network back but route stale)
Detached      → Locking       (logout while offline)
Detached      → ShuttingDown  (quit while offline)
Locking       → Locked        (vault closed cleanly)
ShuttingDown  → Stopped       (last task joined)
```

Every other pair is forbidden. The `AppLifecycle::transition_to`
function returns `LifecycleError::IllegalTransition { from, to }` on
violation. Tests exercise the full table to prevent silent
regressions when adding states.

## `AppLifecycle` API

```rust
pub struct AppLifecycle { /* AtomicU8 + Notify + broadcast::Sender */ }

impl AppLifecycle {
    pub fn new() -> Self;
    pub fn state(&self) -> LifecycleState;
    pub fn transition_to(&self, new_state: LifecycleState) -> Result<(), LifecycleError>;
    pub async fn wait_for(&self, target: LifecycleState) -> Result<(), LifecycleError>;
    pub fn subscribe(&self) -> broadcast::Receiver<LifecycleState>;
}
```

The `AtomicU8` lets every command sample the state without taking a
lock. `Notify` wakes the `wait_for` waiters when a transition lands.
`broadcast::Sender` lets the dispatch loop and UI subscribe to every
transition for tracing and the login window's progress UI.

## `TransportGuard`

Mutating Tauri commands acquire a `TransportGuard::write(lifecycle)`
at the top of their body. If the lifecycle is not in a `can_write`
state, construction returns `LifecycleError::CannotWrite { state }`
and the command short-circuits with the existing string-error
plumbing.

Read-only commands use `TransportGuard::read(lifecycle)` which
accepts `can_query`-true states (`Resuming`, `Operational`,
`Degraded`, `Detached`).

```rust
#[tauri::command]
pub async fn send_dm(
    state: State<'_, SharedState>,
    /* ... */
) -> Result<(), String> {
    let _guard = TransportGuard::write(&state.lifecycle)
        .map_err(|e| e.to_string())?;
    // ... actual work; _guard lives for the body
}
```

The guard does not mutate state on drop. Its sole job is the
boundary check at construction; holding it for the command body is
purely a lifetime-binding convenience so the borrow checker can
verify the check actually ran.

## Where transitions fire

| Transition | Fired by |
|---|---|
| `Stopped → Starting` | `setup::run` at app startup |
| `Starting → Locked` | `services/veilid/lifecycle/network.rs` on `Attachment::AttachedGood` |
| `Locked → Resuming` | `commands/auth.rs::login` after vault unlock |
| `Resuming → Operational` | `services/login_runtime.rs` after watch arming + cache warm |
| `Operational → Degraded` | `services/veilid/lifecycle/route_refresh.rs` on route death |
| `Operational → Detached` | dispatch loop on `Attachment::Detached` |
| `Detached → Operational` | `services/veilid/lifecycle/network.rs` on `Attachment::AttachedGood` |
| `Operational → Locking` | `commands/auth.rs::logout` |
| `Locking → Locked` | `commands/auth.rs::logout` once vault zeroised |
| `* → ShuttingDown` | `shutdown::handle_exit` on `RunEvent::Exit` |

## Why share with the daemon

The daemon's CLI/TUI track needs the same capability gating: a
remote CLI request to "send a DM" should reject when the daemon is in
`Locked` for the same reason the Tauri shell rejects it. Hoisting
the FSM into `rekindle-lifecycle` lets both frontends share one
transition table and one semantic — there is no second source of
truth for "is the app ready to write."

The Tauri shell wraps the FSM with `TransportGuard`; the daemon
wraps it with `daemon::handler::dispatch_with_capability_check` which
serves the same purpose at the IPC boundary. Both check `can_write`
on the same `AppLifecycle` enum without behaviour drift.
