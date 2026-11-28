# 0021 — The record pool is its own Veilid-free crate; `rekindle-protocol` keeps its Veilid backend

- **Status:** Accepted (amends [0014](0014-veilid-boundary-is-transitive.md))
- **Date:** 2026-10-21
- **Plan step:** E2.B slice S2 in `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2), after S1
  put the pool on the `rekindle_records::port::DhtBackend` trait.
- **Related:** [0009](0009-crate-harvest-tiers.md) (tiers; a crate at an existing tier),
  [0014](0014-veilid-boundary-is-transitive.md) (the transitive Veilid boundary, rule B22).
- **Amended 2026-11-15** (plan step VU, Veilid 0.5.8 = the pinned fork at `91453e97`, ADR 0017; decisions 1, 4
  and 6; see "Port contracts after Veilid 0.5.8" below).

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

## Port contracts after Veilid 0.5.8 (amendment 2026-11-15)

Veilid 0.5.8 changed four contracts below the `DhtBackend` port. The port stays Veilid-free; each change is
recorded here because the netsim's `SimDht` must honour it too (citations are the 0.5.8 fork tree).

- **Open intent (plan VU decision 1, built at VU3).** `DhtBackend::open(key, writer, intent: OpenIntent)`, with
  `OpenIntent { Hold, Check }` in `rekindle-records/src/port/dht.rs`. `Hold` (own records, the keepalive, the
  leases a watch or a sweep holds) maps to `OpenDHTRecordOptions { rehydrate: Some(true) }`; `Check`
  (`read_once`, `inspect_once`, existence probes) maps to `rehydrate: Some(false)`, so a probe never counts as
  interest. Veilid queues a rehydration on a re-open only when asked, for a record not created by this node this
  session, at most once per 300 s (`storage_manager/open_record.rs:62-81`, `storage_manager/mod.rs:178-198`). The
  intent is plumbed through `RecordPool::acquire` and `RecordPort::{acquire, read_once, inspect_once,
  write_once}`.
- **`SetOutcome` meanings (decision 4, VU1 and VU2 item 15).** 0.5.8's set returns the settled value whenever it
  differs from ours **or** another node holds a rival at its seq (`storage_manager/set_value.rs:753-756`). The pool
  classifies `Ok(Some(v))` in one place (`rekindle-record-pool/src/io.rs`): `Some(ours)` (our data and writer) is
  `Landed` (or `Unchanged` when the local seq did not move), logged at debug as contested; `Superseded` is only a
  different value. `Ok(None)` is ours at consensus. A set below consensus with `AllowOffline(false)` is a
  `TryAgain` that stores nothing (`set_value.rs:731-738` → `local_record_store_interface.rs:60-62`), mapped to
  `Offline`; `SetOutcome::BelowConsensus` is deleted.
- **The hint contract (VU3).** A watch hint obliges a network read of the hinted subkeys. Veilid marks every
  reported subkey as reported and unread, and a later coalesced (range-only) push reports only unmarked subkeys;
  only a get that reaches the network clears a mark (`outbound_watch_manager/outbound_watch_state.rs:251-273`,
  `get_value.rs:199-200`, `watch_value.rs:2275-2297`). Every `ValueChange` handler therefore reads with
  `RecordPort::read_hinted`, which force-reads each hinted subkey (`rekindle-records/src/port/record.rs`, the
  `ValueChange` and `read_hinted` docs).
- **Veilid `mock-api` stays out of the default test graph (decision 6).** Cargo's feature unification would mock
  the real node in every crate built with it, and it is no netsim replacement. Its use is a separate contract-test
  workspace `contract/veilid/`, excluded from the root workspace, that tests `rekindle-protocol`'s real adapters
  (the `DhtBackend` impl, the `VeilidAPIError` → port error mapping, `VeilidUpdate` → `PortEvent`): the code the
  netsim bypasses. Built 2026-11-15 (`547880fa`; run `cargo test --manifest-path contract/veilid/Cargo.toml`). The 0.5.8 mock ignores `rehydrate` and never emits `RouteChange`, `Network` or `Log`, and answers an unresolved route `InvalidTarget`, so those contracts are tested on the values (`open_options`, veilid-core's own update values, the error value). Its CI job joins at plan phase F.
