# 0008 — Runtime / adapter / pure-logic layering in `src-tauri/src/services/`

- **Status:** Accepted
- **Date:** 2026-06

## Context and problem statement

`src-tauri/src/services/` is the largest directory in the Tauri
shell — around 229 `.rs` files. Without a structural convention,
the same logic could land in any of three places:

- As a command handler in `commands/`.
- As a `services/*.rs` file alongside the rest of the v1 community
  state machine.
- As an inline closure inside the Veilid dispatch loop.

Reviewers spent time arguing about where new code belonged. The
churn between "should this be a command? a service? a crate? an
adapter?" was a recurring source of review-cycle delay, especially
once the eleven harvest crates added a fourth option ("should this
be in a Tier-7 crate?").

The harvest pattern (a Tier-7 crate parameterised over a `Deps`
trait, with the trait implementation in `src-tauri/`) also kept
producing adapter files that grew past 500 LoC into single
god-method-impl blocks. We needed a convention for splitting them.

## Decision drivers

- **Where does new code go?** Must be a one-question flowchart.
- **No god modules.** Each adapter must respect the project's
  500-LoC-per-file budget.
- **Discoverability.** Reviewers reading a `*_runtime.rs` file
  should be able to predict what's in it without grepping.

## Considered options

### Option A — Free-form services (status quo)

Reject — the recurring review churn that motivated this ADR.

### Option B — Single-file adapter per Tier-7 crate

Every `Deps` trait impl in a single `*_adapter.rs` file. Rejected
because adapters grow past 500 LoC quickly (see Phase 14.r's
`channel_adapter` early drafts at ~1 200 LoC).

### Option C — Runtime / adapter / pure-logic layering — selected

Three categories with explicit rules:

1. **Runtime (`*_runtime.rs`).** Tauri-runtime glue that touches
   multiple stores (DHT + governance + AppState + SQLite + events).
   Permission check → AppState read → pure logic call →
   persistence → event emit. No protocol decisions in the body.
2. **Adapter (`*_adapter/`).** `Deps` trait implementation against
   the live `AppState` + `AppHandle` + `DbPool`. Module-dir pattern
   with submodules `deps_impl`, `state_reads`, `state_mutations`,
   `dht`, `persist`, `events`, `misc`.
3. **Pure-logic surfaces (`services/community/*.rs`, `services/dm/`,
   …).** The original community state machine; Tauri-aware but
   contains domain logic awaiting harvest into Tier-7 crates.

The choice flowchart:

- Pure logic (no `AppState`, no `AppHandle`, no `veilid_core`) →
  Tier-7 crate, define a `Deps` trait.
- Tauri-runtime glue (touches both `AppState` and the DHT / SQLite)
  → `*_runtime.rs`.
- Schwarzschild bridge → `*_adapter/`.

## Decision outcome

Chosen: **Option C**.

Adapter directories follow the Phase 14.r / Phase 23.D module-dir
pattern, capping `deps_impl.rs` at the project's 500-LoC budget.
Existing adapters: `calls_adapter/`, `channel_adapter/`,
`files_adapter/`, `governance_adapter/`, `gossip_adapter/`,
`presence_adapter/`, `sync_adapter/`, `voice_adapter/`,
`voice_signaling_adapter.rs` (single file, still under the cap).

## Consequences

**Positive.**

- One question to decide where code goes ("does this own
  `AppState`?").
- Adapter shape is predictable; reviewers know where to find
  `dht.rs` vs. `persist.rs` vs. `events.rs`.
- LoC budget naturally enforced — splitting is forced when an
  adapter section grows.

**Negative.**

- More files per surface. A new feature touches three or four
  files where before it might have touched one.
- The runtime vs. pure-logic boundary is occasionally ambiguous —
  some `services/community/*.rs` files are still Tauri-aware but
  carry domain logic that should eventually move to crates. The
  Phase 23 harvest is ongoing.

**Boundaries.**

- The Veilid dispatch loop (`services/veilid/lifecycle/dispatch.rs`)
  is the one explicit exception — it does not fit the
  runtime / adapter / pure-logic categorisation because it is the
  inbound event source for everyone else.
- The background-service category (`dht_publish_service`,
  `game_service`, `idle_service`, `presence_service`,
  `sync_service`, `push_relay`, `message_service/`) is the second
  exception — these are long-running tokio tasks, not
  command-driven flows. They have their own `shutdown_tx`
  companion in `AppState`.

## More information

- [`../architecture/services-pattern.md`](../architecture/services-pattern.md) — the full pattern, with the per-adapter split and the where-to-put-it flowchart.
- [`../architecture/tauri-backend.md`](../architecture/tauri-backend.md) — the services-layer overview.
