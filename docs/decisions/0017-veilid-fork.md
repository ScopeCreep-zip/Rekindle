# 0017 — veilid-core is built from a minimal pinned fork until upstream carries our fixes

- **Status:** Accepted (owner request 2026-10-07; recorded as an ADR by owner ruling O4, 2026-10-15)
- **Date:** 2026-10-15
- **Plan step:** C7.21 (`.claude/plans/standards-remediation/00-integration-plan.md`, v2.2)
- **Amends:** [0001](0001-veilid-as-transport.md) (Veilid is the transport; it said nothing about how
  veilid-core is sourced). It reverses the owner's 2026-10-05 position "fix in our layer, not by forking
  dependencies" (`.claude/plans/architecture-audit/history/h3-owner-voice.md:107`) for defects that live inside
  Veilid.
- **Related:** [0014](0014-veilid-boundary-is-transitive.md) (which crates may link veilid-core).

## Context and problem statement

Calls and rehydration requests were dropped with "Invalid target: route key does not exist". The cause is in
veilid-core 0.5.7, not in Rekindle: `select_single_route` returned bare ids and keys, the route was used only
after an `await` (`Destination::get_respond_to`, `compile_safety_route`), and `release_allocated_route` removed
a route unconditionally. A release in between (the API's `release_route`, the ping validator, private-route
management) made the use fail. No Rekindle-side workaround can hold a route that Veilid frees underneath it
without re-implementing route management, which the owner rejected. On 2026-10-07 the owner asked for the fix
at the source, to be contributed back.

## Decision outcome

**Build veilid-core from a private fork, pinned by commit:**

- `Cargo.toml` `[patch.crates-io]` → `veilid-core = { git = "https://github.com/radicalkjax/veilid.git",
  rev = "0cd847debfaee427eeceeb5046ad48cd85540d63" }`.
- Branch `rekindle/v0.5.7-route-drain` = commit `1cabfbe1` "Veilid v0.5.7 source snapshot" (the upstream
  v0.5.7 tree, `f5cdcca3`, identical tree hash; history is not carried because upstream history holds a 266 MB
  file GitHub refuses) plus **one** fix commit, `0cd847de` "Hold a selected route until it is used; a release
  drains a held route".
- The same change on upstream history is `c9e29651` on branch `fix/route-select-holds-ref` in `~/veilid`
  (remote `origin` = `gitlab.com/veilid/veilid`), ready for the upstream merge request.

**The patch** (7 files, +252/−44, all in `veilid-core/src/routing_table/route_spec_store/` and
`rpc_processor/destination.rs`): `select_single_route` returns a `SelectedRoute` that holds an
`AllocatedRouteSetRef` taken under the same read lock as the choice; the reply path and `compile_safety_route`
hold it until done; `release_allocated_route` marks a held route for release instead of removing it, and the
last drop releases it; selection skips routes marked for release. It reuses Veilid's existing RAII handles
(`AllocatedRouteSetRef`, `mark_for_release`), which only `route_stats`' dead-route path used before. The public
API and `RouteIdAndKeys` are unchanged. Test: `test_route_drain`; veilid-core's suite passes. Precedent for
"no longer selectable" vs "gone": Tor `circuit_mark_for_close`/`unusable_for_new_conns`, arti (selection
returns an `Arc`), i2pd `eTunnelStateExpiring`.

**Policy:**

1. **Upstream it.** Every fork commit is written as an upstream merge request on upstream history. Submission
   waits on the owner's go-ahead (owner, 2026-10-07: "no MR or upstream contact before then").
2. **Keep the fork minimal.** Only fixes for defects in Veilid itself, each one upstreamable as-is. No Rekindle
   features, no config defaults (those are set app-side, e.g. `remote_max_records` through
   `VeilidStartupOptions`), no API additions Rekindle alone needs. A new fork commit needs its own plan step
   naming the defect, the upstream-history twin commit and a veilid-core test.
3. **Rebase rules.** When upstream tags a release: take a fresh source snapshot of that tag, re-apply only the
   commits upstream has not merged, run veilid-core's suite and Rekindle's gates, and move the `rev` pin.
   **Drop the `[patch]` entirely** when a Veilid release carries every fix. The pin is always a full commit
   hash, never a branch.
4. **Keep the three places in sync:** root `Cargo.toml` `[patch.crates-io]`, `deny.toml` `allow-git`, and
   `.cargo/config.toml` `net.git-fetch-with-cli = true` (the repository is private).
5. **Source of truth.** Veilid facts are read from `~/veilid` at the pinned rev, never from the registry copy
   in `~/.cargo/registry`, which lacks the patch.

## Consequences

- Every machine that builds Rekindle needs GitHub credentials for the private fork; the Linux (Pop!_OS) build
  fetches through the git CLI.
- Rekindle no longer builds against stock veilid-core 0.5.7. Text that says "matches veilid-core 0.5.7" means
  0.5.7 plus this fork commit.
- Any further fork change (for example the vanguards-style hop pinning that `AllocateRouteParams` cannot express
  today) goes through rule 2 and gets its own ADR amendment line here.

## More information

- [Veilid developer book](https://veilid.gitlab.io/developer-book), `~/veilid` at `0cd847de`
- `.claude/plans/standards-remediation/00-integration-plan.md` C7.21
