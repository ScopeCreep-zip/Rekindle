# Rust Crate Reference

Rekindle's business logic is split across **33 workspace crates** organised
into a strict tier hierarchy: lower tiers know nothing about higher tiers,
and the lowest tiers (`rekindle-types`, `rekindle-secrets`) contain zero I/O,
zero async, and zero side effects.

Two frontends sit on top of this stack:

- The **desktop app** (`src-tauri/`) links `rekindle-protocol` directly and
  runs the Veilid node in-process. This is the primary user-facing build.
- The **daemon + CLI** track (`rekindle-node` + `rekindle-cli`) funnels every
  Veilid call through `rekindle-transport` and exposes it over a Noise-IK
  encrypted IPC bus.

Both speak the same protocol on the wire. The difference is process layout.

Per-crate detail lives in [`crates-tier-detail.md`](crates-tier-detail.md).
This file is the inventory and the tier diagram.

## Workspace Members

The list below mirrors `[workspace] members` in the repository root
`Cargo.toml`. Counts are `.rs` files under each crate's `src/`.

```
crates/
# ── Tier 1 — Vocabulary ───────────────────────────────────────────────
├── rekindle-types/                 (35) shared IDs, enums, error taxonomy

# ── Tier 2 — Cryptographic boundary ───────────────────────────────────
├── rekindle-secrets/                (9) keys, MEK, signing — sole crypto boundary
├── rekindle-vault/                 (10) SQLCipher double-encrypted store + typed helpers [NEW]
├── rekindle-audit/                  (2) BLAKE3 keyed hash chain [NEW]

# ── Tier 3 — Wire format, records, local state ────────────────────────
├── rekindle-codec/                     wire types + encodings: Cap'n Proto schemas/codecs, envelopes, record pages, dedup
├── rekindle-records/                (4) DHT record lifecycle, SMPL schema
├── rekindle-db/                    (17) schema, Db handle, repositories, node lock [NEW]
├── rekindle-events/                 (5) dedup + SubscriptionState + EventJournal [NEW]
├── rekindle-idempotency/            (2) LRU+TTL command-dedup cache [NEW]
├── rekindle-mek-rotation/           (9) cascade election + MEK distribute [NEW]
├── rekindle-analytics/              (8) local-only SQL aggregations [NEW]

# ── Tier 4 — Routing ──────────────────────────────────────────────────
├── rekindle-route/                  (6) private-route lifecycle, peer cache
├── rekindle-lifecycle/              (4) 9-state app FSM + TransportGuard [NEW]

# ── Tier 5 — Gossip mesh + presence ───────────────────────────────────
├── rekindle-gossip/                 (9) gossip mesh primitives
├── rekindle-presence/              (24) presence primitives + orchestrators [NEW]
├── rekindle-friendship/             (4) inbox-scan coordinator [NEW]

# ── Tier 6 — Governance ───────────────────────────────────────────────
├── rekindle-governance/            (24) pure CRDT merge, permissions
├── rekindle-governance-runtime/    (23) async lifecycle (origin/bootstrap/join) [NEW]

# ── Tier 7 — Self-contained features ──────────────────────────────────
├── rekindle-channel/               (15) channel messaging, threads, reactions [NEW]
├── rekindle-dm/                    (13) DM / group DM logic
├── rekindle-calls/                 (14) direct-call key derivation + signaling state
├── rekindle-files/                 (16) chunked P2P file delivery (Lost Cargo)
├── rekindle-link-preview/           (1) OpenGraph fetcher (sandboxed)
├── rekindle-video/                 (10) video / screen-share fragmentation

# ── Cross-cutting integration crates ──────────────────────────────────
├── rekindle-sync/                  (14) cross-device sync orchestration
├── rekindle-protocol/              (53) Veilid + Cap'n Proto — used by the desktop app
├── rekindle-crypto/                (22) identity, Signal Protocol, DHT record keys
├── rekindle-game-detect/           (11) cross-platform game detection
├── rekindle-voice/                 (32) Opus codec, audio I/O, jitter, mixer
├── rekindle-e2e-server/             (1) HTTP IPC bridge for Playwright E2E tests

# ── Daemon / CLI track ────────────────────────────────────────────────
├── rekindle-transport/             (87) the daemon's Veilid adapter (over rekindle-protocol)
├── rekindle-ipc/                   (14) IPC bus: protocol, Noise, client, server
├── rekindle-node/                  (32) `rekindled`: owns transport, serves the bus
├── rekindle-client/                (8) what every frontend shares: bus client, on-demand start, config, formatters
├── rekindle-cli/                   (28) CLI frontend, binary `rekindle`
└── rekindle-tui/                   (36) terminal UI frontend, binary `rekindle-tui`

# ── Utilities (no tier) ───────────────────────────────────────────────
└── rekindle-utils/                  (9) time, hashing, retry, log scrub, config layers, data root
```

Tag legend: `[NEW]` marks crates introduced after the May 2026 snapshot
documented in earlier revisions of this file — the eleven harvest crates
that produced this rewrite. See
[`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md).

Workspace-level dependencies declared in `Cargo.toml`:
`serde`, `serde_json`, `tokio`, `tracing`, `tracing-subscriber`, `thiserror`,
`anyhow`, `bytes`, `futures`, `parking_lot`, `capnp`, `capnpc`, `hex`, `rand`.

Workspace lints enforce `deny(warnings)`, `deny(dead_code)`,
`deny(unused-imports)`, `deny(unused-variables)`, `clippy::all = deny`,
`clippy::pedantic = warn`, plus restriction lints `dbg-macro = deny`,
`todo = deny`, `unimplemented = deny`, `undocumented-unsafe-blocks = deny`.

## Tier Invariants

The tier hierarchy is a contract. The CI gauntlet greps every crate's
`src/` for forbidden imports and fails the build on violation.

| Tier | Crates | Allowed | Forbidden |
|------|--------|---------|-----------|
| 1 | `types` | `serde`, `thiserror` | `tokio`, async, `veilid-core`, `tauri`, any crypto lib |
| 2 | `secrets`, `vault`, `audit` | crypto libs (only here) | `veilid-core`, `tauri`, network I/O |
| 3 | `codec`, `records`, `events`, `idempotency`, `mek-rotation`, `analytics` | `tokio`, `async-trait`, in-process DB (`rusqlite`) for `analytics` only | `veilid-core`, `tauri` |
| 4 | `route`, `lifecycle` | `tokio` | `veilid-core`, `tauri` |
| 5 | `gossip`, `presence`, `friendship` | `tokio`, `async-trait` (deps traits) | `veilid-core`, `tauri` |
| 6 | `governance`, `governance-runtime` | `tokio` (runtime only), `async-trait` | `veilid-core`, `tauri`, `rusqlite`, `iota_stronghold` |
| 7 | `channel`, `dm`, `calls`, `files`, `link-preview`, `video` | `tokio`, `async-trait`, file I/O (`files` cache) | `veilid-core`, `tauri` |

The Veilid boundary is **transitive linkage**
([ADR 0014](../decisions/0014-veilid-boundary-is-transitive.md)). Only these may
link `veilid-core`:

- `rekindle-protocol` — every Veilid call: the record pool (`dht::pool`),
  `RouteImports`, `OwnRoutes`, node startup. Rule B22
  (`cargo xtask check-veilid-dht-calls`) keeps each call in its owner.
- `rekindle-transport` — the daemon's adapter over `rekindle-protocol`.
- `rekindle-node` — the host, through `rekindle-transport`.
- `rekindle-desktop` — until F1 makes it a thin client.

The tier crates reach Veilid only through a `Deps` trait, so their logic stays
portable, and since plan step C8 they cannot link it: the wire types live in
`rekindle-codec` (encodings) and `rekindle-types` (layouts and constants), and
`cargo xtask check-boundaries` walks the resolved dependency graph, failing any
crate outside this list from which `veilid-core` is reachable.

`rekindle-crypto`, `rekindle-voice`, `rekindle-game-detect`, `rekindle-sync`,
and `rekindle-utils` are **cross-cutting**: they sit alongside the tier
column rather than inside it, and are consumed by whichever frontend needs
them. They respect the no-`tauri` rule but may take a `veilid-core`
dependency where it is unavoidable (`rekindle-sync` uses Veilid's
`watch_dht_values` orchestration; `rekindle-crypto` does not).

## Visual Tier Diagram

```
   ┌─────────────────┐                  ┌──────────────────┐
   │ src-tauri       │                  │ rekindle, -tui   │
   │ (desktop app)   │                  │ (rekindle-client)│
   └────────┬────────┘                  └─────────┬────────┘
            │                                     │ IPC (Noise IK)
            │                                     ▼
            │                            ┌──────────────────┐
            │                            │ rekindle-node    │
            │                            │ (daemon)         │
            │                            └─────────┬────────┘
            │                                      │
            ▼                                      ▼
   ┌──────────────────┐                  ┌────────────────────┐
   │ rekindle-protocol│                  │ rekindle-transport │
   │  Veilid calls    │◄─────────────────│  daemon adapter    │
   │  (pool, routes)  │                  │                    │
   └────────┬─────────┘                  └─────────┬──────────┘
            │                                      │
            └──────────────────┬───────────────────┘
                               ▼
              ┌──────────────────────────────────────────┐
              │ Tier 7  channel, dm, calls, files,       │
              │         video, link-preview              │
              │ Tier 6  governance + governance-runtime  │
              │ Tier 5  gossip, presence, friendship     │
              │ Tier 4  route, lifecycle                 │
              │ Tier 3  codec, records, events,          │
              │         idempotency, mek-rotation,       │
              │         analytics                        │
              │ Tier 2  secrets, vault, audit            │
              │ Tier 1  types                            │
              └──────────────────────────────────────────┘
```

## Most-Common Reading Paths

| If you are … | Start at |
|---|---|
| New to the codebase | [`overview.md`](overview.md) → this file → [`crates-tier-detail.md`](crates-tier-detail.md) |
| Adding a feature | [`crates-tier-detail.md`](crates-tier-detail.md) for the target crate, then [`tauri-backend.md`](tauri-backend.md) for wiring |
| Auditing the Veilid boundary | [`crates-tier-detail.md`](crates-tier-detail.md) entries for `rekindle-protocol` and `rekindle-transport`; [`decisions/0001-veilid-as-transport.md`](../decisions/0001-veilid-as-transport.md) |
| Tracking the harvest work | [`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md); [`roadmap.md`](../roadmap.md) "Harvest crates" section |

Cross-cutting subsystem docs (`event-dispatch.md`, `services-pattern.md`,
`lifecycle-fsm.md`, `presence.md`, `audit-chain.md`) cover how the
harvest crates integrate with `src-tauri` and the daemon track.
