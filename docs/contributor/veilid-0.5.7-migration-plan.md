# Migration plan: veilid-core 0.5.3 → 0.5.7

Companion to [`veilid-0.5.7-gap-audit.md`](./veilid-0.5.7-gap-audit.md),
which establishes *what* upstream changed. This is *how* we take it,
and which parts of our tree get smaller as a result.

**Headline:** we are in a much better position for this upgrade than
`feat/cli-tui-restructure-rewrite` was. That branch had to strip 40+
fields from its Veilid network config (`4cd4dc8`). Our tree sets
exactly **three** `VeilidConfig` fields, and all three survive
0.5.4's `footgun-config` move. Our real cost is somewhere else
entirely: a sqlite dependency conflict that hard-blocks the bump.

**What actually gets simpler:** two of the three config fields we set
are workarounds for upstream bugs that are now fixed, and they are
duplicated across two separate node-startup paths. Deleting them is
the moment to unify those paths.

**What does not get simpler:** video fragmentation, 255-slot
segments, and PQXDH. See §7.

---

## Phase 0 — Unblock the dependency graph

**This is a hard blocker, not a cleanup.** Verified:

```
$ cargo update -p veilid-core --precise 0.5.7 --dry-run
error: failed to select a version for `libsqlite3-sys`.
    ... required by package `rusqlite v0.39.0`
    ... which satisfies dependency `rusqlite = "^0.39.0"` of `async-sqlite v0.5.7`
    ... which satisfies dependency `async-sqlite = "=0.5.7"` of `keyvaluedb-sqlite v0.1.8`
    ... which satisfies dependency `keyvaluedb-sqlite = "=0.1.8"` of `veilid-core v0.5.7`
package `libsqlite3-sys` links to the native library `sqlite3`, but it
conflicts with a previous package which links to `sqlite3` as well:
package `libsqlite3-sys v0.36.0`
    ... of package `rusqlite v0.38.0`
```

veilid-core 0.5.7 pulls rusqlite 0.39 → libsqlite3-sys 0.37. We pin
rusqlite 0.38 → libsqlite3-sys 0.36. Only one package in the graph may
declare `links = "sqlite3"`, so nothing resolves until we unify.

Bump rusqlite `0.38` → `0.39` in all six pins:

| Crate | File | Features |
|---|---|---|
| `rekindle-dm` | `crates/rekindle-dm/Cargo.toml:31` | `bundled` |
| `rekindle-analytics` | `crates/rekindle-analytics/Cargo.toml:13,16` | `bundled` (two entries) |
| `rekindle-asql` | `crates/rekindle-asql/Cargo.toml:21,26` | — / `bundled` |
| `rekindle-vault` | `crates/rekindle-vault/Cargo.toml:11` | `bundled-sqlcipher-vendored-openssl` |
| `rekindle-e2e-server` | `crates/rekindle-e2e-server/Cargo.toml:23` | `bundled` |
| `rekindle` (tauri) | `src-tauri/Cargo.toml:87` | `bundled` |

Both feature flags we rely on (`bundled`,
`bundled-sqlcipher-vendored-openssl`) exist on rusqlite 0.39 —
confirmed against the crates.io feature list.

### Verified: this is low risk, not high

An earlier draft called `rekindle-vault` the riskiest item here, on the
theory that a rusqlite major bump might shift the SQLCipher page format
or KDF defaults under the identity keystore. **That is not the case.**
Checked rather than assumed:

- **The bundled SQLCipher is byte-identical.** libsqlite3-sys 0.36 and
  0.37 vendor the same amalgamation — SQLCipher 4.10.0 / SQLite 3.50.4,
  matching SHA-256 (`de78ef08…`) on `sqlcipher/sqlite3.c`. There is no
  format change to migrate across.
- **Our key path doesn't touch the KDF anyway.**
  `crates/rekindle-vault/src/store.rs:37` passes a raw hex key
  (`PRAGMA key = x'…'`), which bypasses SQLCipher's PBKDF2 entirely, and
  `cipher_page_size` is pinned explicitly at line 39 rather than
  inherited from defaults.
- **It builds and passes.** `cargo test -p rekindle-vault` on rusqlite
  0.39: 6/6 green, including `reopen_same_passphrase_decrypts` and
  `wrong_passphrase_rejected`. No API breakage in our usage.

So no pre-upgrade vault file is needed as a test input — identical
engine plus raw-key plus pinned page size means existing vaults open
unchanged. Keep the round-trip test in the PR as a regression guard,
not as a gate.

### One decision to get right here

**`rekindle-asql` stays.** Its docstring says it was vendored because
upstream `tokio-rusqlite` is unmaintained and hard-pins rusqlite 0.37,
"which conflicts with veilid-core 0.5.3's sqlite chain". Half that
reason changes (the chain now wants 0.39) but the load-bearing half
does not — upstream is still unmaintained and still pins 0.37. Bump
its internal pin to 0.39 and **keep the `tokio_rusqlite::` API**, so
all 23 call sites across 10 files compile unchanged. Update the
Cargo.toml description to say 0.39/0.5.7 so it doesn't read as stale.

Note `4cd4dc8` took the other road: dropped the vendored crate and
hand-rolled a `DbPool` over `spawn_blocking`. That made sense on a
branch already restructuring crates. On ours it is a rewrite of
working, tested code (`crates/rekindle-asql/src/tests.rs`, 281 lines)
for no benefit — don't copy it.

**Exit criteria — the first two are already met.** With the six pins at
0.39 and the veilid spec relaxed to `"0.5"`,
`cargo update -p veilid-core --precise 0.5.7` resolves cleanly:
veilid-core 0.5.7, and a single `libsqlite3-sys 0.37.0` / `rusqlite
0.39.0` in the graph. `rekindle-vault` builds and its tests pass.
Remaining: full workspace build and test suite.

**One note from the resolve:** it pulls in a second `x25519-dalek`
(3.0.0 alongside our 2.0.1). Harmless — no `links` conflict, just
binary size — but it means Veilid's KEM types and our PQXDH types are
built against different versions of the same crate and won't interop
directly. Relevant only if Phase 5 happens.

---

## Phase 1 — The bump

Mechanical once Phase 0 lands.

1. **Version spec.** `"0.5.3"` → `"0.5"` in three files:
   `crates/rekindle-transport/Cargo.toml:22`,
   `crates/rekindle-protocol/Cargo.toml:32`,
   `src-tauri/Cargo.toml:102`.
2. **`Sequencing::NoPreference` → `PreferUnordered`** — exactly three
   sites: `crates/rekindle-transport/src/broadcast/node.rs:844`,
   `src-tauri/src/services/voice_adapter/frame_sender.rs:49`,
   `src-tauri/src/state_helpers/mod.rs:80`.
3. **`VeilidConfig` — expected to be a no-op.** We set only
   `protected_store.allow_insecure_fallback`,
   `protected_store.always_use_insecure_storage`, and `network.upnp`.
   All three are still on the public struct at 0.5.7 (the upgraded
   branch sets all three:
   `crates/rekindle-transport-veilid/src/broadcast/node.rs:104,128`).
   Verify rather than assume, but budget nothing here.
4. **`AttachmentState` — a silent runtime break, now fixed.** An
   earlier draft of this plan said "leave it alone, the `OverAttached`
   arm just goes dead." That was wrong, and reading 0.5.7's source
   rather than its changelog is what caught it.

   Upstream's enum
   (`veilid_api/types/veilid_state.rs`) did two things the changelog
   never mentions: it **added `AttachedFair`** and **renamed
   `FullyAttached` → `AttachedFull`**. `Display` emits snake_case, so
   0.5.7 now sends us `attached_fair` and `attached_full`.

   Our parser (`from_veilid_string`) had no arm for either, and its
   `_ => Detached` fail-closed default meant **two of the six attached
   states silently reported as `Detached`** — with a green build,
   because we parse strings rather than matching the upstream type.

   Blast radius was limited but real: `dispatch.rs:80` takes the
   `is_attached` bool from *Veilid's* own `is_attached()`, so network
   gating stayed correct; what broke was our own
   `SharedState::attachment_state()` and every CLI/TUI status surface
   reading it, which would show "Detached" on a healthy fair-signal
   node.

   **The fix is append-only, and that constraint is not obvious.**
   The type doc called the discriminants "a stable ABI contract", but
   the actual IPC codec is **postcard**
   (`crates/rekindle-node/src/ipc/framing.rs:30`), which encodes a
   fieldless enum by **declaration index**, not by the `repr(u8)`
   value. So inserting `AttachedFair` in its natural position between
   `AttachedWeak` and `AttachedGood` would silently reinterpret every
   later variant on a version-skewed daemon/CLI socket. It is appended
   at `= 8` instead, `OverAttached` is retained purely to hold
   `Detaching`'s index, and the now-meaningless derived `Ord` is
   superseded by an explicit `strength()` method.

   A regression test
   (`shared::tests::every_veilid_attachment_string_is_understood`)
   walks upstream's own enum and asserts every variant round-trips
   without hitting the fail-closed arm, so the next upstream rename
   fails the build instead of shipping.
5. **Classify `TransactionNotFound`.** New `VeilidAPIError` variant.
   Our classifier (`crates/rekindle-protocol/src/dht/mod.rs:41-52`)
   matches `KeyNotFound | TryAgain | Timeout | NoConnection` as
   transient and falls everything else into the hard-error arm. A
   background transaction losing to a foreground record open is
   transient — left alone it will suppress retries that would have
   succeeded. Make this an explicit decision with a test.
6. **No action:** capnp 0.25→0.26 (transitive); MSRV 1.89 (toolchain
   pinned 1.92.0); `footgun` → `footgun-nodeid-target` (never
   enabled — but fix the stale comments at
   `broadcast/node.rs:827` and `frame_sender.rs:41`).

**Status — phases 0 and 1 are done and verified in this branch.**
`cargo check -p rekindle-protocol -p rekindle-transport -p rekindle-node
-p rekindle-cli --all-targets` passes on veilid-core 0.5.7, and
`rekindle-types` + `rekindle-transport` + `rekindle-vault` tests are
green (54 / 111 / 6). Across four Veilid-facing crates the entire
compile-time surface of this upgrade was **one line** — the
`Sequencing` rename. Everything else was either a no-op (config) or
invisible to the compiler (`AttachmentState`).

**Still unverified:** `src-tauri` and the full workspace. Not a Veilid
problem — this container lacks the GTK system libraries `gdk-sys`
needs (`libgtk-3-dev`, and the 24.04 apt pool is missing several
`libgdk-pixbuf`/`mesa` debs). Two of the three `Sequencing` sites live
in `src-tauri` and are edited but uncompiled. Build it on a dev machine
or in the Nix shell before merging.

**Remaining exit criteria:** `src-tauri` + full workspace build, full
test suite, and a manual two-node attach/DHT/messaging smoke run.

---

## Phase 2 — Delete the workarounds

This is where the architecture actually gets smaller.

### 2.1 UPnP — make it configurable, then A/B it. Do not just delete.

An earlier draft called this the cheapest win in the plan. It isn't.
See gap-audit §1.1 for the full trace; the short version:

- The specific panic our comment cites is real in 0.5.3
  (`native/mod.rs:750`) and **gone in 0.5.7** (now `let-else` +
  `bail!`).
- But the mechanism the comment describes — restart re-entering
  startup with unchanged interfaces — **cannot be reproduced from the
  code in either version**, because every startup builds a fresh
  `Network` with a fresh `NetworkInterfaces`, whose first `refresh()`
  always reports a change. So we never actually established what UPnP
  was breaking.
- **0.5.7 still sets `network_needs_restart = true` when the IGD tick
  fails**, and the attachment manager still detaches and re-attaches
  in response. The churn path is intact.

So: convert `network.upnp = false` from a hardcoded constant into a
config field in **both** startup paths
(`crates/rekindle-protocol/src/node.rs:110`,
`crates/rekindle-transport/src/broadcast/node.rs:95`), and replace the
~18-line comment with a pointer to gap-audit §1.1 rather than
restating a mechanism that doesn't hold.

**Default it to `false` — unchanged behaviour — and flip it under
measurement.** The win (direct inbound instead of VICE relay fallback)
is real, but the thing to watch is attachment-flap rate on a hostile or
absent IGD gateway, which is plausibly what was observed originally.
Also note `require_inbound_relay` now implicitly disables UPnP in
0.5.7, so the two settings interact.

### 2.2 Unify the two node-startup paths

We have two live Veilid startups:

- `RekindleNode` — `crates/rekindle-protocol/src/node.rs`, used by the
  Tauri host via `src-tauri/src/services/veilid/lifecycle/node.rs`
- `TransportNode` — `crates/rekindle-transport/src/broadcast/node.rs`,
  used by the daemon/CLI via `rekindle-node` and `rekindle-cli`

Their config blocks are near-identical, down to duplicated
twenty-line workaround comments explaining the same two upstream bugs.
That duplication was tolerable while the comments were load-bearing
documentation. Once §2.1 and §2.3 delete them, both blocks collapse to
a handful of lines and the case for a single shared
`build_veilid_config()` becomes obvious.

**This is the "catch the arch up" item** — the upgrade doesn't force
it, but it's the natural moment, because the thing that made the two
copies diverge is gone.

### 2.3 Re-test the ProtectedStore workaround — do not assume

`always_use_insecure_storage = true` at
`crates/rekindle-protocol/src/node.rs:90` and
`crates/rekindle-transport/src/broadcast/node.rs:83`, because
keyring-manager 0.7.1 reaches secret-service's *blocking* zbus API,
which calls `Runtime::block_on` inside our Tokio runtime and panics.

0.5.4 bumps keyring-manager to 0.8.2 (our `Cargo.lock:4607` still
pins 0.7.1). But it is described as fixing "secret-service connection
problems" — a connection fix, not a change to the blocking async
model. **Test it on Linux with gnome-keyring running.** If it still
panics, keep the workaround and update the comment to name 0.8.2 so
the next person doesn't re-litigate it.

Low stakes either way: as the comment correctly notes, Veilid's
ProtectedStore holds only Veilid's own node/route secrets; user
identity keys live in the SQLCipher vault.

---

## Phase 3 — Hand backpressure back to upstream

### 3.1 Adopt `max_concurrent_operations`

0.5.4 added it to `VeilidConfigDHT`. Set one global cap, then revisit
the four hand-guessed semaphores:

- `crates/rekindle-governance-runtime/src/join_stages.rs:471`
  (`SCAN_PARALLELISM`)
- `crates/rekindle-governance-runtime/src/dht_hydration.rs:73`
  (`OPEN_PARALLELISM`)
- `crates/rekindle-protocol/src/dht/community/channel_record.rs:634`
  (hardcoded `Semaphore::new(10)`)
- `src-tauri/src/services/presence_adapter/scan.rs:89`

Keep the ones expressing genuine per-subsystem fairness. Drop the ones
that only existed as global backpressure — that job now belongs to the
config knob, and four independently-guessed numbers don't compose into
a coherent ceiling anyway.

### 3.2 Consolidate the retry ladder

We have at least four near-duplicate retry helpers, all of them
papering over the same upstream flakiness:

- `rekindle_protocol::dht::retry_on_unreachable`
  (`crates/rekindle-protocol/src/dht/mod.rs:61`)
- `open_with_retry`
  (`crates/rekindle-transport/src/broadcast/dht/record.rs:114`)
- `allocate_route_with_retry`
  (`src-tauri/src/services/login_runtime.rs:98`) — 15 × 3 s ≈ 45 s
- the explicit copy at `src-tauri/src/services/veilid/network.rs:346`,
  whose own comment says it "mirrors
  `login_runtime::allocate_route_with_retry`"

Consolidate to one policy with one place to tune it. Do this *after*
Phase 4's measurements, so the budgets reflect post-upgrade reality
rather than being carried over unexamined.

---

## Phase 4 — Re-tune policy upstream now handles

**Measure before changing.** These are behaviour changes, not
refactors, and each one has a recorded failure mode that justified the
current value. The upgrade makes them worth revisiting; it does not
make them wrong.

### 4.1 Retest 3-hop inbound routes

`node.rs:59-71` records that 3-hop inbound was **tried and reverted**:
`new_private_route()` round-trip-tests each allocation, so 3 hops
tripled the relays that must all answer — allocations flapped,
presence rows published empty blobs, voice rosters stopped forming.

0.5.4: "Simplified route testing, eliminated route exhaustion
problems." That is precisely the mechanism that broke us.

Retest, and **use the original failure symptoms as the pass/fail
criteria** — allocation flap rate, empty presence blobs, voice roster
formation. Success moves every compiled path above the architecture §8
3-hop target instead of sitting at it. Revert cleanly if the symptoms
return; this is a revert of a revert, so keep it isolated.

### 4.2 Relax route-heal timers — after measuring

`crates/rekindle-route/src/lifecycle.rs`: `HealGate` 10 s cooldown,
`ROUTE_WATCHDOG_INTERVAL` 30 s, `PEER_ROUTE_CACHE_MAX_AGE` 900 s.
These absorb churn that upstream now damps itself — `OnlineDetector`,
`FlapDetector`, and "routes no longer are completely reset when
switching relays or publishing peer info".

Don't delete this layer. Ours is *policy* (backend-owns-policy,
identical across Tauri host and daemon); upstream's is *mechanism*.
Re-measure heal frequency post-upgrade and relax the constants to
match.

**Correction:** an earlier draft said the relay circuit breaker
(`crates/rekindle-route/src/relay.rs:27-28`) "overlaps `FlapDetector`".
Reading `veilid-tools/src/flap_detector.rs`, it does not — that is a
decaying-penalty accumulator over an observed value, with no half-open
state, while ours is a send-path routing decision encoding §14.5
observed peer reliability. Keep the circuit breaker. `FlapDetector` is
however a genuine upgrade over `HealGate`'s fixed 10 s cooldown. See
[`veilid-provided-vs-home-rolled.md`](./veilid-provided-vs-home-rolled.md)
§4.

### 4.3 Use the richer attachment signal

`VeilidUpdate::Attachment` now carries "much more visibility into
actual attachment conditions and network size estimations", and
`OnlineDetector` makes address switches reactive rather than
tick-based.

Our `SharedState`
(`crates/rekindle-transport/src/shared.rs:34-46`) tracks an 8-value
enum plus two bools and discards the rest. That coarseness is why
`wait_for_network_ready`
(`src-tauri/src/services/login_runtime.rs:~145`) is a bounded *timeout*
and why route allocation needs a 45 s retry budget — we can't tell
"not ready yet" from "not going to be ready". A real readiness signal
could replace both with an event.

### 4.4 Revisit the 60 s inspect poll

`INSPECT_INTERVAL = 60s` (`crates/rekindle-sync/src/inspect.rs:5`) is
the catch-up path for missed watch notifications. 0.5.4 made watch
value notifications transaction-aware, which should make misses rarer.

**Instrument the miss rate first** — how often does the poll surface
something the watch didn't? Relax the cadence only against that
number. This poll is the backstop for "message X isn't showing up"
(`docs/user/faq.md:150`); loosening it on optimism is how that
regresses.

---

## Phase 5 — HPKE (separate branch, optional)

Not part of catching up. Real cleanup, but it is a wire-format change
and deserves its own branch.

`crates/rekindle-crypto/src/group/mek_distribution.rs` (286 lines)
hand-rolls X25519 ECDH → HKDF-SHA256 (`rekindle-mek-wrap-v1`) →
AES-256-GCM with an Ed25519→X25519 bridge. 0.5.7 ships exactly this
as RFC 9180 base mode: `hpke_seal`/`hpke_open` on `CryptoSystem`, VLD0
= DHKEM-X25519, plus `encapsulation_key_from_signing_key` /
`decapsulation_key_from_signing_secret` — the same bridge, audited,
with RFC 9180 known-answer tests.

Requires a versioned envelope and a dual-read window: our 68-byte
`[nonce || ct+tag]` and HPKE's `enc || ct` are not interchangeable, and
members will be on mixed versions.

**Group MEK distribution only.** Never the DM path — 0.5.7 implements
HPKE for VLD0 and NONE only; VLD1 (ML-DSA / ML-KEM) is declared but
unimplemented, so our PQXDH
(`crates/rekindle-crypto/src/signal/pqxdh/`) remains strictly
stronger.

**Two prerequisites, both established after this plan was first
written** — see
[`veilid-provided-vs-home-rolled.md`](./veilid-provided-vs-home-rolled.md):

- **Keys carry over.** Upstream's Ed25519→X25519 bridge is
  byte-identical to our `pseudonym_to_x25519` (verified by running both
  derivations, §1.1 there). This is an envelope migration, not a
  re-key.
- **`wrap_mek`/`unwrap_mek` are currently forked across
  `rekindle-transport` and `rekindle-crypto`** with the same HKDF
  label (§2 there). Collapse that fork *first* — otherwise the risky
  part gets done twice. And note `CryptoSystem` is only reachable from
  a running node, so this also forces the architecture-rule B2
  decision (§3 there).

---

## 6. Sequencing and risk

| Phase | Blocking? | Risk | Payoff |
|---|---|---|---|
| 0 — rusqlite 0.39 | **Yes, hard blocker** | Low — SQLCipher amalgamation is byte-identical, tests green | None on its own |
| 1 — the bump | Yes | Low — 1 line, plus the `AttachmentState` fix | Security fixes (audit §1.7) |
| 2 — workarounds | No | Medium — UPnP is an A/B, not a deletion | One config path instead of two; *maybe* direct inbound |
| 3 — backpressure | No | Low | Four guessed numbers → one real ceiling |
| 4 — retune policy | No | Medium — behaviour change | Above-target routing; less self-managed churn |
| 5 — HPKE | No | Medium — wire format | Retires bespoke crypto |

Phases 0–1 are one PR and should land together: Phase 0 alone is a
dependency bump with no user-visible benefit, and Phase 1 alone
doesn't resolve. Phase 2 can follow immediately. Phases 3–4 want
measurement between them, so they should not be rushed into the same
PR.

**The upgrade is worth doing for Phase 1 alone**, independent of every
simplification below it: 0.5.3 accepts DHT value-change notifications
with invalid signatures (rejected only at read time), and our entire
SMPL/gossip model is `ValueChange`-driven. 0.5.7 also fixes a
remote-connection-flood deadlock with a PoC slated for publication.

## 7. What this does not fix

Setting expectations, since "catch up to the newest Veilid" invites
the assumption that the known gaps close:

- **Video quality (~480p @ 15 fps).** Unchanged. The binding
  constraint is veilid-tools' 1,272-byte fire-and-forget datagrams
  with all-or-nothing reassembly and no retransmit —
  `FRAGMENT_LEN = 1280 - HEADER_LEN` is untouched on upstream `main`,
  and nothing in 0.5.4–0.5.7 or UNRELEASED touches reassembly. Our
  4 KiB + Reed-Solomon fragmentation stays exactly as is. Still needs
  `veilid-media`.
- **255-slot segments / cross-segment offline catch-up.** Follows the
  DHT SMPL schema's subkey ceiling. Unchanged. Plate Gates C1-2 stand.
- **Post-quantum DMs.** Ours already exceeds what Veilid offers.

Separately, and independent of the upgrade: `docs/user/faq.md:143` and
`docs/architecture/communities-channels.md:224` attribute the video
ceiling to the 32 KB `app_message` cap. That is not the reason (see
`crates/rekindle-video/src/fragment.rs:11-22`). Worth correcting
whether or not we upgrade.
