# 0009 — Tier bumps and the eleven harvest crates

- **Status:** Accepted
- **Date:** 2026-06

## Context and problem statement

Between the May 2026 inventory and this ADR, the workspace grew
from 22 crates to 33. Eleven new crates landed:

- `rekindle-vault` — SQLCipher double-encrypted store (Tier 2).
- `rekindle-audit` — BLAKE3 keyed hash chain (Tier 2).
- `rekindle-events` — event dedup + `EventJournal` (Tier 3).
- `rekindle-idempotency` — LRU+TTL command-dedup cache (Tier 3).
- `rekindle-mek-rotation` — cascade rotator (Tier 3).
- `rekindle-analytics` — local SQL aggregations (Tier 3).
- `rekindle-lifecycle` — 9-state app FSM + `TransportGuard` (Tier 4).
- `rekindle-presence` — friend / community / idle / game
  orchestrators (Tier 5).
- `rekindle-friendship` — inbox-scan coordinator (Tier 5).
- `rekindle-governance-runtime` — async lifecycle on top of the
  pure-CRDT crate (Tier 6).
- `rekindle-channel` — channel messaging, threads, reactions,
  polls, automod, slowmode (Tier 7).

Each crate was extracted as part of the **Phase 23 services drain**
— the harvest of `src-tauri/src/services/` into tier-aligned
Tier-3-through-7 crates. The drain is documented separately as the
ongoing services-pattern work
([`0008-runtime-adapter-pattern.md`](0008-runtime-adapter-pattern.md));
this ADR records the discrete tier assignments and the principles
that drove them.

## Decision drivers

- **Tier purity.** Lower tiers must not import higher-tier
  dependencies. The CI tier-violation lint enforces this.
- **No god crates.** Each crate stays under the ≤3 000-LoC
  per-crate budget; per-file ≤500 LoC.
- **Reusability across the two frontends.** A crate that the
  desktop app uses must also be usable by the daemon track without
  carrying `tauri` or `veilid-core` as a dependency.

## Tier assignments

### Tier 2 — `rekindle-vault`, `rekindle-audit`

Both crates own key material and live alongside `rekindle-secrets`
as the cryptographic / persistent boundary. They are the only
non-`secrets` crates allowed to import crypto libraries directly.

### Tier 3 — `rekindle-events`, `rekindle-idempotency`,
`rekindle-mek-rotation`, `rekindle-analytics`

These crates carry local state with `tokio` async surface but no
DHT / Veilid dependency. `rekindle-analytics` is allowed `rusqlite`
because it is local-only (analytics never leave the device);
others are pure logic.

### Tier 4 — `rekindle-lifecycle`

The FSM has a routing-adjacent role: it gates whether routes can
be opened, whether DHT writes can happen, whether the vault can be
unlocked. It belongs at Tier 4 alongside `rekindle-route`.

### Tier 5 — `rekindle-presence`, `rekindle-friendship`

Both crates build on the gossip / route surfaces of Tier 4 and
below. Presence is naturally a mesh-overlay concern; friendship's
inbox-scan coordinator is similar in shape (one decision loop
draining multiple triggers).

### Tier 6 — `rekindle-governance-runtime`

The async lifecycle (origin, bootstrap, join, segments, apply)
layers on the pure CRDT (`rekindle-governance`). The runtime crate
is allowed `tokio` and `async-trait` for its `Deps` trait surface,
but is forbidden from importing `veilid-core`, `tauri`, `rusqlite`,
or `iota_stronghold` — every I/O surface goes through the trait.

The split keeps the pure CRDT crate (`rekindle-governance`)
runtime-free so its merge function can run synchronously over
millions of property-test inputs.

### Tier 7 — `rekindle-channel`

A self-contained feature crate on top of `rekindle-mek-rotation`,
`rekindle-gossip`, `rekindle-protocol`, and `rekindle-governance`.
Same shape as the existing Tier-7 crates (`rekindle-dm`,
`rekindle-files`, `rekindle-calls`, `rekindle-video`,
`rekindle-link-preview`).

## Decision outcome

The eleven crates ship at the tiers listed above. Architecture
rules **B1**, **B2**, **B15**, **B16**, **B17** enforce the
boundary constraints. The tier matrix in
[`../architecture/crates.md`](../architecture/crates.md) is the
ongoing source of truth.

## Consequences

**Positive.**

- Each crate is property-testable in isolation; the runtime / I/O
  concerns are at the `Deps` trait boundary.
- The two frontends share the same protocol logic without either
  one taking on the other's transport dependency.
- Adding a twelfth crate requires answering one structural
  question ("what tier?") rather than rewriting the whole
  dependency-graph diagram.

**Negative.**

- Per-crate `Cargo.toml`, lints, `lib.rs` docs are now duplicated
  eleven times. We accept the bookkeeping.
- New contributors must read [`crates.md`](../architecture/crates.md)
  and [`crates-tier-detail.md`](../architecture/crates-tier-detail.md)
  before contributing a non-trivial feature — there is no longer
  one mega-crate to grep.

**Boundaries.**

- Adding a crate at a new tier requires an ADR. Adding a crate at
  an existing tier requires only a PR.
- Adding `veilid-core` to any crate other than `rekindle-protocol`
  or `rekindle-transport` requires an ADR.

## More information

- [`../architecture/crates.md`](../architecture/crates.md) — inventory and tier diagram.
- [`../architecture/crates-tier-detail.md`](../architecture/crates-tier-detail.md) — per-crate descriptions.
- [`0008-runtime-adapter-pattern.md`](0008-runtime-adapter-pattern.md) — the services-pattern that produced these crates.
- [`../roadmap.md`](../roadmap.md) — the "Harvest crates" section tracks Phase 23 status.
