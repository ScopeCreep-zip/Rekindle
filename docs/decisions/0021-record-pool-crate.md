# 0021 — The record pool is its own Veilid-free crate; `rekindle-protocol` keeps its Veilid backend

- **Status:** Accepted (amends [0014](0014-veilid-boundary-is-transitive.md))
- **Date:** 2026-10-21
- **Plan step:** E2.B slice S2 in `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2), after S1
  put the pool on the `rekindle_records::port::DhtBackend` trait.
- **Related:** [0009](0009-crate-harvest-tiers.md) (tiers; a crate at an existing tier),
  [0014](0014-veilid-boundary-is-transitive.md) (the transitive Veilid boundary, rule B22).

## Context and problem statement

ADR 0014 says `rekindle-protocol` "keeps only Veilid calls (the record pool, `RouteImports`, `OwnRoutes`, node
startup)". The record pool (`RecordPool`: leases, the one retry layer, honest writes, durable re-push, the one
watch-death owner, drain and teardown, the closer) is the code every DHT record goes through. The netsim
(`crates/rekindle-node/tests/netsim/`) cannot run it while it lives in a crate that links veilid-core, so the
simulation runs a fake pool instead and tests a pool production never uses (E2.B fidelity findings).

After S1 the pool's own code names no Veilid type: it calls an `Arc<dyn DhtBackend>`, and the Veilid impl is
`rekindle_protocol::dht::backend::VeilidDht`. The question is where the pool's logic lives.

## Decision drivers

- The netsim must run the production pool, with only the backend simulated.
- The transitive boundary (ADR 0014) must prove the pool never links Veilid, not a code review.
- The pool uses `rekindle_lifecycle::SessionScope` (Tier 4), so it cannot go into `rekindle-records` (Tier 3).
- Callers keep `rekindle_protocol::dht::pool::*` and `RecordPort` unchanged.

## Decision outcome

**The pool's logic is the new Tier-4 crate `rekindle-record-pool`** (depends on `rekindle-records`,
`rekindle-lifecycle`, `rekindle-utils`; never veilid-core). It is listed in `VEILID_FREE`
(`xtask/src/veilid_dht_calls.rs`), and the transitive check fails if it ever links Veilid.

ADR 0014's sentence is amended to: `rekindle-protocol` keeps only Veilid calls (**the pool's Veilid backend**,
`RouteImports`, `OwnRoutes`, node startup). `rekindle_protocol::dht::pool` re-exports the pool beside the Veilid
conversions; rule B22's owner of Veilid DHT calls stays `rekindle-protocol/src/dht/backend.rs`.

The pool's timing runs on `tokio::time` (the watch re-arm's backoff and forgiveness included), so the netsim's
paused clock drives it.

### Consequences

- Good: the netsim (E2.B S3 onward) runs `RecordPool` over a simulated `DhtBackend`; the fake pool goes.
- Good: the pool's unit tests run on an in-crate test backend, with no Veilid node.
- Neutral: one more crate at an existing tier (ADR 0009 needs no ADR for that; this one records the 0014 change).
