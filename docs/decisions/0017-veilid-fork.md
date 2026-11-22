# 0017 — veilid-core is built from a minimal pinned fork until upstream carries our fixes

- **Status:** Accepted (owner request 2026-10-07; recorded as an ADR by owner ruling O4, 2026-10-15)
- **Date:** 2026-10-15
- **Plan step:** C7.21 (`.claude/plans/standards-remediation/00-integration-plan.md`, v2.2)
- **Amends:** [0001](0001-veilid-as-transport.md) (Veilid is the transport; it said nothing about how
  veilid-core is sourced). It reverses the owner's 2026-10-05 position "fix in our layer, not by forking
  dependencies" (`.claude/plans/architecture-audit/history/h3-owner-voice.md:107`) for defects that live inside
  Veilid.
- **Related:** [0014](0014-veilid-boundary-is-transitive.md) (which crates may link veilid-core).
- **Amended 2026-11-14** (plan step VU0/VU1, rule 3): rebased onto Veilid v0.5.8. Branch
  `rekindle/v0.5.8-route-drain` = snapshot `dcf0b31585592b4c24c1071d3b2030ccf2c15cdd` "Veilid v0.5.8 source
  snapshot" (parentless; tree `5e19d78c`, identical to tag `v0.5.8` `759da70b`) plus **one** fix commit,
  `91453e97e8ef467aa7c7c3b5e871a7ed5738f941` (10 files, +428/−50). Upstream does not carry the fix as of 0.5.8.
  The rebased fix also skips routes marked for release in `RouteSpecStore::save`
  (`veilid-core/src/routing_table/route_spec_store/mod.rs:202-208`), so a released route is never reloaded live
  after a restart. Departures from the rebase recipe (`evidence/veilid-0.5.8/3-routes-rpc.md` A4): the extra
  marked-route filter on the pool path is dropped (`is_live_sequencing_match` already refuses a marked route);
  `get_allocated_routes_to_test` is `pub(in crate::routing_table)` for the tests; the allocation-retry test holds
  the allocate lock and inserts the route as an allocation would (the mock has no published peer info). The
  upstream-history twin is `bba17bfd` on `fix/route-select-holds-ref-v0.5.8` in `~/veilid`. The 0.5.7 branch and
  `0cd847de` below are history.
- **Amended 2026-11-22** (lead decision 62): two more fix commits on top of `91453e97`, branch
  `rekindle/v0.5.8-route-pool`, pin `bc7416334ee0b94fe30ac9fec14dee98493aea4a`. `e1158844` "Keep API-allocated
  private routes out of the safety route pool": `route_select` takes only automatic routes, so a route the API
  allocated (Rekindle's own inbound private route) is never chosen as someone's safety route.
  `bc741633` "Keep testing API-allocated routes after a peer info change": `private_route_management` still
  tests manual routes after our peer info changes, so a dead published route is found. Tests in
  `veilid-core/src/routing_table/route_spec_store/tests_route_spec_store/test_route_pool.rs`:
  `test_manual_route_is_not_a_pool_route`, `test_published_manual_route_stays_out_after_peer_info_change`,
  `test_preferred_manual_route_is_selected`, `test_automatic_route_is_selected_beside_manual`,
  `test_manual_route_is_tested_after_peer_info_change`. Upstream contact (MR, push to any Veilid remote) is held
  per Policy 1. `SCHEMA_VERSION` 152 → 153 in the same change (Consequences).
- **Amended 2026-11-22** (VF5, three more fix commits on `rekindle/v0.5.8-route-pool`, pin
  `18b3cbdf2c43caf53f12d6c2e0ce4ff354663eba`). Evidence: Linux real-app runs on round 23 (Veilid debug) logged five
  General-route deaths in two runs, each `Remote route test failed (no response): <id>` → `releasing allocated route
  <id>` → `own route died death=class=General cause=veilid` within about 1 s, while that allocated route's loopback test
  passed every ~10 s. The id is shared because Rekindle's self-test imports our own route blob, so a failed test of the
  import released the allocated route. `a2c01232` "Release only the tested import when a remote route test fails";
  `a48b66aa` "Release an expired import before an allocated route with the same id" (`release_route` removes the
  remote cache entry, live or expired, before it releases an allocated route); `18b3cbdf` "Type deferred route tests
  so a remote retest never tests an allocated route" (`RouteTestTarget` Allocated/Remote). Tests in
  `tests_route_spec_store/test_remote_routes.rs`: `test_failed_remote_test_releases_only_the_import`,
  `test_failed_remote_test_after_release_keeps_allocated_route`, `test_release_of_expired_import_keeps_allocated_route`,
  `test_deferred_retests_target_only_their_own_route_set`. Upstream contact held per Policy 1. `SCHEMA_VERSION`
  153 → 154 in the same change (Consequences).

## Context and problem statement

Calls and rehydration requests were dropped with "Invalid target: route key does not exist". The cause is in
veilid-core (0.5.7 when found; unchanged in 0.5.8), not in Rekindle: `select_single_route` returned bare ids and keys, the route was used only
after an `await` (`Destination::get_respond_to`, `compile_safety_route`), and `release_allocated_route` removed
a route unconditionally. A release in between (the API's `release_route`, the ping validator, private-route
management) made the use fail. No Rekindle-side workaround can hold a route that Veilid frees underneath it
without re-implementing route management, which the owner rejected. On 2026-10-07 the owner asked for the fix
at the source, to be contributed back.

## Decision outcome

**Build veilid-core from a private fork, pinned by commit:**

- `Cargo.toml` `[patch.crates-io]` → `veilid-core = { git = "https://github.com/radicalkjax/veilid.git",
  rev = "18b3cbdf2c43caf53f12d6c2e0ce4ff354663eba" }` (since the 2026-11-22 VF5 amendment; `bc741633…` from the
  first 2026-11-22 amendment; `91453e97…` from the
  2026-11-14 amendment; originally `0cd847debfaee427eeceeb5046ad48cd85540d63`).
- Branch `rekindle/v0.5.8-route-drain` = commit `dcf0b315` "Veilid v0.5.8 source snapshot" (the upstream
  v0.5.8 tree, `759da70b`, identical tree hash; history is not carried because upstream history holds a 266 MB
  file GitHub refuses) plus **one** fix commit, `91453e97` "Hold a selected route until it is used; a release
  drains a held route" (since 2026-11-22 branch `rekindle/v0.5.8-route-pool` adds `e1158844`, `bc741633`,
  `a2c01232`, `a48b66aa` and `18b3cbdf`, see the amendments above). Before the 2026-11-14 amendment: branch `rekindle/v0.5.7-route-drain` = `1cabfbe1` (the v0.5.7
  tree, `f5cdcca3`) plus `0cd847de`.
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
   in `~/.cargo/registry`, which lacks the patch (since 2026-11-14 the 0.5.8 fork worktree `~/veilid-wt-058`).

## Consequences

- Every machine that builds Rekindle needs GitHub credentials for the private fork; the Linux (Pop!_OS) build
  fetches through the git CLI.
- Rekindle no longer builds against stock veilid-core 0.5.8. Text that says "matches veilid-core 0.5.8" means
  0.5.8 plus these fork commits; older text that says 0.5.7 is history from before the 2026-11-14 rebase.
- **A pin bump can be one-way, so `SCHEMA_VERSION` bumps with every pin bump.** 0.5.8's table store migrates
  every row on first open (`veilid-core/src/table_store/mod.rs:639-646`, `table_store/migration/v1.rs`: rows
  re-encrypted LZ4-compressed, the routing table dropped). A 0.5.7 build cannot read the migrated rows, and its
  default `wipe_on_invalid_device_encryption_key` wipes the whole Veilid namespace (new node id, local record
  store and offline writes lost) while the DB and vault survive. Rekindle therefore bumps
  `SCHEMA_VERSION` (`crates/rekindle-db/src/open.rs`) in the same commit as each pin bump, so all three stores
  reset together, and no older build (an old worktree) runs against a data root a newer Veilid has opened.
- Any further fork change (for example the vanguards-style hop pinning that `AllocateRouteParams` cannot express
  today) goes through rule 2 and gets its own ADR amendment line here.

## More information

- [Veilid developer book](https://veilid.gitlab.io/developer-book), the fork at `18b3cbdf` (worktree
  `~/veilid-wt-vf2`; `91453e97` in `~/veilid-wt-058`; `~/veilid` at `0cd847de` for 0.5.7)
- `.claude/plans/standards-remediation/00-integration-plan.md` C7.21
