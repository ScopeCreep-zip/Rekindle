# 0020 — Domain logic lives in tier crates and the Host; `src-tauri` is a frontend

- **Status:** Accepted (supersedes [0008](0008-runtime-adapter-pattern.md))
- **Date:** 2026-10-18
- **Plan step:** E1.1 in `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2); the domain ports
  E2–E6 follow it, F1 completes it.
- **Related:** [0010](0010-single-daemon-thin-frontends.md) (one Host; thin frontends; strangler migration),
  [0019](0019-intentional-actions.md) (the Host's one intent gate).

## Context and problem statement

ADR 0008 told contributors where code goes *inside* `src-tauri/src/services/`: runtimes, `Deps` adapters and
pure-logic surfaces. ADR 0010 then made `src-tauri` a thin frontend of one `rekindle_node::Host`, which the GUI
runs in-process until F1 and which `rekindled` runs alone. With 0010 in force, 0008's flowchart sends new domain
code to the wrong process: an adapter against `AppState` is code the Host cannot run, so the TUI and CLI behave
differently from the GUI, and every port has to move it again.

The question is where domain code goes now, and how the GUI keeps working while domains move one at a time.

## Decision drivers

- One behaviour for every frontend (ADR 0010): the GUI, TUI and CLI reach the same code through `HostApi`.
- No glue: nothing is written in one place to be moved later.
- The tier table (`docs/architecture/crates.md`) already says which crate owns which kind of logic.
- The migration is strangler-style (ADR 0010, Fowler 2024): seams, one domain at a time, transitional
  architecture that goes away when the migration ends.

## Considered options

### Option A — Keep 0008 (adapters in `src-tauri`)

Rejected: every adapter written there is a second implementation the daemon lacks, and is moved again by its
port.

### Option B — Move everything into the Host in one step

Rejected: about 258 desktop commands and their stages cannot move at once without a long period where neither
frontend works; ADR 0010 chose the strangler sequence for this reason.

### Option C — Tier crates and the Host own domain code; the shell keeps commands and a shrinking hook — selected

- **Domain logic** goes to the tier crate the tier table names (protocol, records, engines, `Deps` traits).
- **`Deps` impls, unlock stages and Host adapters** go to `rekindle-node::host` (`host/adapters/`, `host/*.rs`).
- **`src-tauri`** keeps Tauri commands, each a `HostApi` call (`HostSlot::api()`), plus the commands that only a
  window can perform (`SHELL_LOCAL`: opening windows, the OS opener, bundled lookups).
- **`HostApi`, `Caller` and `ConnId`** live in `rekindle-ipc` (`host_api.rs`), beside `IpcRequest`, so a frontend
  names them without the node and F1's `DaemonClient` implements the same trait.
- **The transitional seam is `host::shell_hooks::ShellHooks`**, which the GUI implements with its existing,
  not-yet-ported stages (`services/shell_session.rs`). The Host owns the lifecycle and waits on the shell only
  where it must: the shell's unported unlock stages run inside the Host's unlock scope and can abort it, and the
  shell's leave of calls runs before the scope is cancelled. This is the delegate pattern comparable core-plus-GUI
  apps use (Keybase's service-called UI protocols, matrix-rust-sdk's delegate callbacks in Element X); Tailscale's
  GUI shows the end state, where a frontend only watches the daemon's state. Each domain port deletes its part of
  the hook; F1 deletes the trait.

## Decision outcome

Chosen: **Option C**. ADR 0008 is superseded. A new feature's code goes to its tier crate and, for its
orchestration, to `rekindle-node::host`; `src-tauri` gains only the command that calls it.

## Consequences

**Positive.**

- The GUI, TUI and CLI run one implementation of each ported domain.
- Each port deletes the daemon's duplicate and the shell's part of the hook in the same change.

**Negative.**

- Until F1, the in-process GUI implements `ShellHooks`; it is the one sanctioned transitional surface, and it
  only shrinks.

## More information

- `docs/architecture/services-pattern.md`, `docs/architecture/tauri-backend.md`.
- Martin Fowler, [StranglerFigApplication](https://martinfowler.com/bliki/StranglerFigApplication.html).
