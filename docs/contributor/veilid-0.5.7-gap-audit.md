# Veilid 0.5.4 → 0.5.7 gap audit

Triggered by the "Major upstream-project announcement (Veilid, …)"
row in [`dependency-audit.md`](./dependency-audit.md): read the
announcement, audit our use of the relevant feature, document it.

**Where we are:** this branch pins `veilid-core = "0.5.3"`
(`crates/rekindle-transport/Cargo.toml:22`,
`crates/rekindle-protocol/Cargo.toml:32`, `src-tauri/Cargo.toml:102`;
`Cargo.lock` resolves 0.5.3, released 2026-03-23).

**Where upstream is:** 0.5.7, released 2026-07-19. Four releases
ahead — 0.5.4 (2026-06-23), 0.5.5 (2026-06-27), 0.5.6 (2026-07-13),
0.5.7 (2026-07-19).

`feat/cli-tui-restructure-rewrite` (usrbinkat, 2026-07-30) already
carried out a 0.5.2 → 0.5.7 bump in commit `4cd4dc8`. That branch is
a restructure with a different crate layout, so it is not a
cherry-pick, but it is a worked example of the migration and it
corroborates two of the findings below.

---

## 1. Gaps that upstream closes

### 1.1 UPnP is disabled to dodge a panic — fixed in 0.5.4

We currently ship with automatic port mapping off:

- `crates/rekindle-protocol/src/node.rs:110`
- `crates/rekindle-transport/src/broadcast/node.rs:95`

The ~20-line comment at `node.rs:92-109` records why: when `upnp_task`
can't reach an IGD gateway it sets `network_needs_restart`, and the
second `Network::startup_internal` calls
`refresh_network_state().await?.unwrap_or_log()`, which panics because
`refresh_network_state` returns `Ok(None)` when interfaces are
unchanged — exactly the UPnP-restart case. The node is left
permanently detached. Our comment notes it was "fixed only on
unreleased git main".

0.5.4 shipped that fix: **"Fixed UPNP support"**.

**What it buys us:** the workaround degrades every NAT'd user to
inbound relays via VICE. Re-enabling UPnP restores direct inbound
reachability — lower latency, fewer hops, and less load on the relay
pool we depend on. Corroboration: on the upgraded branch, `upnp`
defaults back to `true` (`crates/rekindle-types/src/config.rs:256`).

### 1.2 Route allocation flapping — substantially improved in 0.5.4/0.5.7

`node.rs:59-71` documents a decision we were forced into: pinning
inbound private routes to 3 hops was **tried and reverted**, because
`new_private_route()` round-trip *tests* each allocation, so 3-hop
tripled the relays that must all answer. Allocations flapped, presence
rows published empty blobs, and voice rosters stopped forming. We
settled for 1-hop inbound.

0.5.4 addresses both halves of that:

- "Simplified route testing, eliminated route exhaustion problems"
- "Improved route optimizer, routes no longer are completely reset
  when switching relays or publishing peer info"

and 0.5.7 adds "Numerous route and fanout performance improvements".

**What it buys us:** the 3-hop inbound decision is worth re-testing —
it would put every compiled path above the architecture §8 target
rather than at it. Separately, our route-heal machinery
(`crates/rekindle-route/src/lifecycle.rs`: `HealGate`,
`ROUTE_WATCHDOG_INTERVAL` 30 s, `HEAL_COOLDOWN` 10 s) exists largely
to absorb route death caused by relay switches. "Routes no longer
completely reset when switching relays" removes a meaningful share of
the churn it was built to survive.

### 1.3 Hand-rolled hybrid encryption — HPKE lands in 0.5.7

`crates/rekindle-crypto/src/group/mek_distribution.rs` implements
MEK distribution as X25519 ECDH → HKDF-SHA256 (label
`rekindle-mek-wrap-v1`) → AES-256-GCM, with an Ed25519→X25519
conversion via `pseudonym_to_x25519`.

0.5.7 ships exactly this shape as a standard: `hpke_seal`/`hpke_open`
on `CryptoSystem` (RFC 9180 base mode), VLD0 = DHKEM-X25519, plus
`encapsulation_key_from_signing_key` /
`decapsulation_key_from_signing_secret` — the same Ed25519→X25519
bridge we hand-rolled, but upstream, with RFC 9180 known-answer tests
and per-suite data-size sweeps.

**Caveat:** this is a wire-format change (our 68-byte
`[nonce || ct+tag]` vs HPKE's `enc || ct`), so it needs a versioned
envelope and a migration path, not a drop-in swap. The win is
retiring bespoke crypto in favour of an audited standard, not
performance.

### 1.4 Per-call-site DHT throttling — now a config knob (0.5.4)

We hand-roll concurrency caps in at least four places:

- `crates/rekindle-governance-runtime/src/join_stages.rs:471`
  (`SCAN_PARALLELISM`)
- `crates/rekindle-governance-runtime/src/dht_hydration.rs:73`
  (`OPEN_PARALLELISM`)
- `crates/rekindle-protocol/src/dht/community/channel_record.rs:634`
  (hardcoded `Semaphore::new(10)`)
- `src-tauri/src/services/presence_adapter/scan.rs:89`

0.5.4 added `max_concurrent_operations` to `VeilidConfigDHT` "to offer
a simple way to cap concurrent DHT operations for automatic
throttling".

**What it buys us:** one global ceiling that actually reflects node
capacity, instead of four independently-guessed numbers that don't
compose. The local semaphores can stay as per-subsystem fairness
limits, but they stop being the only backpressure in the system.

### 1.5 DHT durability on close — record flush (0.5.4)

0.5.4: "Added record flush on DHT record close to ensure closed
records are fully flushed to the TableStore" and "Add
`flush_dht_record` to VeilidAPI interface".

We call `close_record` (`crates/rekindle-protocol/src/dht/mod.rs:299`)
and have no flush anywhere in the tree — the only `flush` hit is
unrelated peer-reliability counters
(`src-tauri/src/services/veilid/lifecycle/cleanup.rs:97`). Worth a
review of shutdown ordering once upgraded.

### 1.6 Watch and inspect are now transaction-aware (0.5.4)

0.5.4: "Rehydration, change inspection, and watch value notifications
are now transaction-aware", plus a new `TransactionNotFound` variant
on `VeilidAPIError`.

We lean on both paths hard: the watch registry with renewal
(`crates/rekindle-transport/src/subscriptions/watches.rs`) and a
~60 s `inspect_dht_record` poll as the catch-up path
(`INSPECT_INTERVAL`, `src-tauri/src/services/community/inspect.rs:123`).
Transaction-awareness should cut the torn and duplicated reads that
the poll currently papers over.

Note our error classifier
(`crates/rekindle-protocol/src/dht/mod.rs:41-52`) matches
`KeyNotFound | TryAgain | Timeout | NoConnection` as transient.
`TransactionNotFound` is new and needs a deliberate classification —
it will otherwise fall into the `_ =>` hard-error arm and suppress a
retry that would have succeeded.

### 1.7 Security fixes we are currently exposed to

This is the strongest single argument for upgrading at all.

- **0.5.4 — inbound signature validation.** "SECURITY: Reject invalid
  DHT schema and value signatures on inbound set value and value
  change notifications, not just at `get_dht_value` time." On 0.5.3 a
  malicious peer can push value-change notifications with invalid
  signatures that are only rejected at read time. Our entire
  SMPL/gossip model is driven by `ValueChange` events
  (`crates/rekindle-transport/src/subscriptions/watches.rs`,
  `crates/rekindle-dm/src/receiver.rs`,
  `crates/rekindle-friendship/src/watch_trigger.rs`).
- **0.5.7 — remote DoS.** "SECURITY: Deadlock on remote connection
  flood fixed in `ConnectionManager`." A PoC is slated for publication
  in `doc/security/poc` now that the patch is out.
- **0.5.5 — bootstrap.** "Direct replies were broken, resulting in
  bootstrap failures on nodes with new IP addresses." This regression
  was introduced *in 0.5.4*, so it is not a bug we have today — it is
  a reason not to stop the upgrade at 0.5.4.

### 1.8 Network-switch reactivity (0.5.4)

0.5.4 added `OnlineDetector` to `NetworkManager` ("reactive to low
level address switches rather than tick-based"), `FlapDetector` in
veilid-tools hooked into connection/online/relay/route flaps,
"Improved address change detection", and handling for SLAAC, APIPA,
CLAT46/XLAT464 and earlier CGNAT detection.

Our own flap handling is entirely timer-based: `HealGate`'s 10 s
cooldown and 30 s watchdog (`rekindle-route/src/lifecycle.rs`), and
the relay circuit breaker at three consecutive failures / 60 s
cooldown (`rekindle-route/src/relay.rs:27-28`). Upstream now absorbs
part of this, which turns ours into a second layer rather than the
only one.

Also relevant: `VeilidUpdate::Attachment` "now includes much more
visibility into actual attachment conditions and network size
estimations". Our `SharedState`
(`crates/rekindle-transport/src/shared.rs:34-46`) tracks only an
8-value attachment enum plus two bools, so there is new signal on the
table we currently discard.

---

## 2. Gaps that upstream does *not* close

### 2.1 Video quality (~480p @ 15 fps)

Not closed, and our docs misattribute the cause.

`docs/user/faq.md:139` and `docs/architecture/communities-channels.md:224`
blame Veilid's ~32 KB `app_message` cap. The actual binding constraint
is documented in the code at
`crates/rekindle-video/src/fragment.rs:11-22`: veilid-tools segments
every envelope on a UDP hop into 1,272-byte fire-and-forget datagrams
with all-or-nothing reassembly and no retransmit, so a 28 KiB fragment
rode as ~23 datagrams per hop and delivered ~50 % of frames across
~6 onion hops. That is why `FRAGMENT_PAYLOAD_LIMIT` is 4 KiB with
Reed-Solomon parity, not 28 KiB.

I checked `veilid-tools/src/assembly_buffer.rs` on upstream `main`:
`FRAGMENT_LEN = 1280 - HEADER_LEN` is unchanged, and nothing in
0.5.4–0.5.7 or the UNRELEASED section touches reassembly or adds
retransmit. This still needs `veilid-media`. Our 4 KiB + FEC
workaround stays exactly as is.

**Action:** correct the two docs — the 32 KB cap is not the reason.

### 2.2 255-member segments / cross-segment offline catch-up

`SLOTS_PER_SEGMENT = 255`
(`crates/rekindle-governance-runtime/src/segments.rs:39`) follows the
DHT SMPL schema's subkey ceiling. Unchanged across 0.5.4–0.5.7. The
roadmap item (Plate Gates C1-2) stands.

### 2.3 Post-quantum — do not swap Signal/PQXDH for HPKE

0.5.7's HPKE is implemented for `VLD0` (DHKEM-X25519) and `NONE`
only. The `VLD1` types (ML-DSA signing, ML-KEM encapsulation) are
declared but not implemented.

Our PQXDH — X25519 + ML-KEM
(`crates/rekindle-crypto/src/signal/pqxdh/`) — is therefore strictly
stronger than anything Veilid currently offers. §1.3's HPKE
recommendation applies to **group MEK distribution only**, which is
classical X25519 today. It must not be extended to the DM path.

### 2.4 ProtectedStore keyring — probably not closed

`always_use_insecure_storage = true` at
`crates/rekindle-protocol/src/node.rs:90` and
`crates/rekindle-transport/src/broadcast/node.rs:83`, because
keyring-manager 0.7.1 goes through secret-service's *blocking* zbus
API, which calls `Runtime::block_on` inside our Tokio runtime and
panics.

0.5.4 bumps keyring-manager to 0.8.2 "to fix secret-service connection
problems", and our `Cargo.lock:4607` still pins 0.7.1. But the fix is
described as a *connection* fix, not a change to the blocking async
model, so treat this as "worth testing", not "closed".

Low stakes either way: as `node.rs:86-89` notes, Veilid's
ProtectedStore only guards Veilid's own node/route secrets, which live
in our `storage_dir`; user identity keys are in the SQLCipher vault.
The upgraded branch does default `always_use_insecure_storage` to
`false` (`crates/rekindle-types/src/config.rs:274`), which is
suggestive but not proof.

---

## 3. Migration cost, 0.5.3 → 0.5.7

| Change | Impact here |
|---|---|
| `Sequencing::NoPreference` → `PreferUnordered` | 3 sites: `crates/rekindle-transport/src/broadcast/node.rs:844`, `src-tauri/src/services/voice_adapter/frame_sender.rs:49`, `src-tauri/src/state_helpers/mod.rs:80` |
| `VeilidConfig` settings moved behind `footgun-config` into `VeilidConfigInternal` | Largest item. `4cd4dc8` had to strip **40+ fields** from its network config mapping. `upnp`, `protected_store.*`, `rpc.default_route_hop_count` and bootstrap survive on the public struct |
| `AttachmentState::OverAttached` removed upstream | No compile break — we own our enum (`crates/rekindle-types/src/notification.rs:28`) and parse from strings. The arm goes dead, but u8 discriminants are load-bearing in atomics (`shared.rs`), so this needs a review pass, not a delete |
| `VeilidUpdate::Attachment` gained fields | Opportunity (§1.8), not a break |
| Per-node RPC stats removed from `PeerStats` | No usage found |
| `footgun` → `footgun-nodeid-target` | Never enabled. Only stale comments reference it: `broadcast/node.rs:827`, `frame_sender.rs:41` |
| `TransactionNotFound` added to `VeilidAPIError` | Needs a classification decision (§1.6) |
| MSRV 1.89.0 | Fine — toolchain pinned at 1.92.0 |
| capnp 0.25 → 0.26 (0.5.6) | Transitive |
| sqlite chain moves | `crates/rekindle-asql` exists *solely* because upstream tokio-rusqlite hard-pins rusqlite 0.37, which conflicts with veilid-core 0.5.3's sqlite chain. `4cd4dc8` unified on rusqlite 0.39 and dropped tokio-rusqlite entirely — check whether `rekindle-asql`'s vendoring is still needed |

---

## 4. Recommendation

Upgrade to 0.5.7, in this order:

1. **Take the upgrade for the security fixes alone** (§1.7) — inbound
   signature validation in particular, given how much of our model
   rides `ValueChange`. Go to 0.5.7, not 0.5.4, because of the 0.5.5
   bootstrap regression.
2. **Re-enable UPnP** (§1.1) and delete the workaround comment.
   Cheapest real win in the list.
3. **Adopt `max_concurrent_operations`** (§1.4) and classify
   `TransactionNotFound` (§1.6). Small, mechanical.
4. **Re-test 3-hop inbound routes** (§1.2) now that route testing no
   longer exhausts the relay pool.
5. **Defer the HPKE migration** (§1.3) — real cleanup, but it is a
   wire-format change and wants its own branch. Group MEK only.
6. **Correct the video docs** (§2.1) regardless of the upgrade.

Nothing here closes the video-quality or 255-member-segment gaps.
Those still need `veilid-media` and Plate Gates C1-2 respectively.

## References

- [Veilid CHANGELOG](https://gitlab.com/veilid/veilid/-/blob/main/CHANGELOG.md)
- [`veilid-core` on docs.rs](https://docs.rs/veilid-core/0.5.7/veilid_core/)
- [`dependency-audit.md`](./dependency-audit.md)
- [`../decisions/0001-veilid-as-transport.md`](../decisions/0001-veilid-as-transport.md)
