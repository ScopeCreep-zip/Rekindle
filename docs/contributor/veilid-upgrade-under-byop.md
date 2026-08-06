# Reading the Veilid work against the project's actual goals

The previous three documents in this series planned the 0.5.7 upgrade
without accounting for two project-level goals: **Veilid-faithful
first**, and **bring-your-own-protocol** (SimpleX, Matrix, atproto and
other community/social substrates alongside Veilid).

Those goals invert three of my earlier recommendations. This document
records the corrections and the reasoning, so the migration plan isn't
followed off a cliff.

---

## 1. What the project is, as documented today

- **Rekindle is a 1:1 reimplementation of the Xfire client** on Tauri 2
  + SolidJS + Veilid, with no central server (`ARCHITECTURE.md:10`).
- **Communities v2.0 is flat governance** — no coordinator, no
  privileged nodes, every member a full peer. The **Schwarzschild
  Principle**: the genesis keypair "collapses behind a horizon" — it
  remains the community's permanent *address* but carries no governance
  authority. It is shared with all members via the invite. "The keypair
  is an address key, not an authority key"
  (`communities-overview.md:51-63`).
- **Truth is the union of member writes, filtered by independent
  permission validation** — reader-validates, not writer-validates
  (principle 7).
- **Tiers 1–7 are protocol-neutral**; only `rekindle-protocol`
  (desktop) and `rekindle-transport` (daemon) may import `veilid_core`
  (architecture rule B2, ADR 0001 "Boundaries").

The Death Stranding framing is load-bearing vocabulary, not decoration:
**Strand Relay** is mutual-aid forwarding for peers that are online but
unreachable (`protocol/relay.md:21`), the community terminal is a Q-pid
analogy (`communities-overview.md:61`), and the **Schwarzschild bridge**
names the adapter layer in ADR 0008.

---

## 2. BYOP collides with the documented architecture — and not where you'd expect

Two documented statements say Rekindle is single-transport by design:

- **ADR 0001** is titled "Adopt Veilid as the **sole** transport
  substrate", status Accepted and reconfirmed.
- **Design principle 10**: "All roads through Veilid. No external
  transport."

Those are the obvious blockers, and they're the *easy* ones — a
superseding ADR and a principle amendment are ordinary governance.

**The hard blocker is principle 5**, and it outranks 10 (the list is
explicitly priority-ordered, lower number wins):

> **5. Anonymity is a floor, not a slider.** Every path — voice, video,
> text, governance — rides a 3-hop Tor-class `Safe` route
> (`ANONYMITY_HOP_FLOOR`). Classes differ only in latency/ordering
> knobs, never in hop count. **No path gets a lower-hop fast lane.**

Matrix gives the homeserver your IP and your full social graph. atproto
is publicly readable by construction. SimpleX is strong on metadata but
is not onion-routed by default. **None of them can satisfy an anonymity
floor**, so "communities over Matrix" and principle 5 cannot both be
true as currently written.

This is a product decision, not an engineering one, and it needs making
*before* the transport abstraction is designed — because the answer
changes what the abstraction must guarantee. The options:

1. **Scope principle 5 per-transport.** "The anonymity floor applies to
   the Veilid transport; other transports carry their own documented,
   weaker properties." Honest, but it turns the flagship guarantee into
   a per-backend footnote, and principle 11 ("honest about tradeoffs")
   means the UI has to surface it, loudly and per-community.
2. **Tier transports by privacy class** and refuse to run the most
   identifying features (voice, presence, governance writes) on the
   weaker ones. Keeps the floor meaningful where it applies; costs
   feature parity.
3. **Treat BYOP as a distinct product mode** with a hard boundary — a
   Matrix community is a different thing from a Veilid community, never
   silently mixed.

I'd recommend (2): it preserves principle 5 as a real guarantee rather
than a slogan, and it degrades honestly, which is what principles 6 and
11 already commit us to.

---

## 3. The good news: the tier system is already the BYOP architecture

This is the part my earlier documents missed entirely.

BYOP does not need a new architecture. Tiers 1–7 — governance, gossip,
records, route, codec, dm, channel, files, video — are already
protocol-neutral, and **rule B2 is the seam**, not bureaucracy. The
Tier-7 `Deps`-trait pattern from ADR 0008 ("Pure logic (no `AppState`,
no `AppHandle`, no `veilid_core`) → Tier-7 crate, define a `Deps`
trait") is exactly the shape a second transport would implement.

And the Schwarzschild Principle makes the governance layer inherently
portable: because authority lives in reader-validated signatures over
CRDT state rather than in the transport, a substrate only has to supply
three things — durable keyed storage, a message-passing path, and an
addressing primitive. SMPL/`app_message`/route-blobs are Veilid's
answers to those, not the requirements themselves.

**So the BYOP work is mostly negative work: find and remove the places
where Veilid-shaped things have leaked above the boundary.** This audit
found one by accident (§4.1).

---

## 4. Three corrections to my earlier recommendations

### 4.1 The crypto fork must collapse **downward**, not toward transport

[`veilid-provided-vs-home-rolled.md`](./veilid-provided-vs-home-rolled.md)
§5 recommended collapsing the duplicated crypto "toward
`rekindle-transport`, since the daemon-track copy is already on the
permitted side of B2". **Under BYOP that is exactly backwards**, and the
code settles it:

- `crates/rekindle-transport/src/crypto/` contains **zero**
  `veilid_core` imports. It is protocol-neutral code that happens to
  live inside the Veilid boundary crate.
- `rekindle-transport` **already depends on `rekindle-crypto`**
  (`Cargo.toml:17`).

So `derive_community_pseudonym`, `pseudonym_to_x25519`, `wrap_mek` and
`unwrap_mek` should be **deleted from `rekindle-transport` and used
from `rekindle-crypto`**. The dependency already exists; the fork is
pure redundancy, and it is redundancy in the wrong direction — group
crypto homed inside the Veilid crate is unusable by a second transport.

This is now the highest-value item in the whole series: it is a pure
deletion, it needs no architecture decision, and it removes a real
Veilid-side leak of protocol-neutral logic.

### 4.2 Do **not** adopt `veilid_core::CryptoSystem`

Same document, §3, suggested adopting `CryptoSystem` at call sites
already inside the boundary. Under BYOP that is wrong on principle:
`CryptoSystem` is reachable only from a running node
(`VeilidAPI::crypto()` → component guard, `veilid_api/api.rs:107`), so
routing MEK distribution through it would make **community crypto
require a live Veilid node** — precisely the coupling BYOP must avoid.
Group crypto has to keep working when the substrate is Matrix.

### 4.3 Do **not** pull `veilid_core::tools` into `rekindle-route`

Same document, §4, floated `FlapDetector` as an upgrade over
`HealGate`. It is a better algorithm, but `rekindle-route` is Tier 4
and deliberately Veilid-free. Importing `veilid_core::tools` would
couple a protocol-neutral tier to Veilid to save ~40 lines of decay
maths. Write the decaying-penalty counter in `rekindle-route` if we
want it, or leave `HealGate` alone.

---

## 5. HPKE: how to be Veilid-faithful *and* BYOP-clean

This is the case where both goals are satisfiable at once, and knowing
the project changes the design.

Veilid does not implement HPKE itself — it uses the standard **`hpke`
crate v0.14.0** (`veilid-core/Cargo.toml:273`) with:

| Parameter | Value |
|---|---|
| KEM | `X25519HkdfSha256` (`vld0/mod.rs:232`) |
| KDF | `HkdfSha256` (`crypto_system/hpke.rs:11`) |
| AEAD | `ChaCha20Poly1305` (`crypto_system/hpke.rs:12`) |
| Mode | Base |
| info | `b"veilid-hpke/1"` ‖ 4-byte `CryptoKind` |
| Blob | `[version=1][4-byte kind][enc][ciphertext]` |

So we can depend on **the same `hpke` crate directly from
`rekindle-crypto`** — no `veilid_core`, no live node, no B2 amendment —
and get RFC 9180 base mode with the exact primitives the Veilid team
selected and test against RFC 9180 known-answer vectors. That is
Veilid-faithful in construction while staying transport-neutral.

**Use our own info label, not Veilid's.** MEK distribution is
Rekindle↔Rekindle, never Rekindle↔Veilid-peer, so byte-compatibility
with `veilid-hpke/1` blobs buys nothing and would embed a Veilid-specific
domain-separation string into crypto that must also run over Matrix.
Use `rekindle-mek/1` and our own framing. (Note upstream's own comment
on those constants — "isolated because the HPKE RFC is still in review"
— which is another reason not to bind our wire format to theirs.)

The keys carry over regardless: the Ed25519→X25519 bridge is
byte-identical between our `pseudonym_to_x25519` and Veilid's VLD0
derivation, verified by running both.

---

## 6. Revised sequencing

Unchanged: **phases 0–1 of the migration plan** (rusqlite unification +
the 0.5.7 bump). Those are transport-layer, correctly placed, and
carry the security fixes. Already landed and verified.

Revised after that:

1. **Delete the transport-side crypto fork** (§4.1). Pure deletion, no
   decision needed, removes a BYOP leak. Do this first.
2. **`max_concurrent_operations`, `flush_dht_record`,
   `TransactionNotFound`** — all transport-layer, all inside the
   boundary, all BYOP-neutral. Safe.
3. **Make the principle-5 decision** (§2) before any transport
   abstraction work. It determines what the abstraction must
   guarantee.
4. **HPKE in `rekindle-crypto` via the `hpke` crate** (§5), on the
   single collapsed implementation, versioned envelope, dual-read
   window.
5. **UPnP A/B, route re-tuning** — unchanged from the plan, still
   Veilid-local and BYOP-irrelevant.

Explicitly **dropped** from the earlier plan: adopting `CryptoSystem`,
adopting `veilid_core::tools` above the boundary, and collapsing crypto
toward `rekindle-transport`.

---

## 7. What BYOP would still need (not scoped here)

For whoever picks this up — the gaps this audit noticed but did not
plan:

- **A superseding ADR** for 0001, and an amendment to principles 5 and
  10. Principle 5 is the real work (§2).
- **A transport trait.** The `Deps`-trait pattern already models it;
  what's missing is a single named abstraction for "durable keyed
  storage + message path + addressing" that `rekindle-protocol` and
  `rekindle-transport` both implement.
- **Where SMPL semantics leak.** `o_cnt: 0`, 255-slot segments, subkey
  overflow chains and the three-path delivery model are Veilid DHT
  shapes that Tier 3–6 crates encode directly. A Matrix backend has
  rooms and events, not subkeys. This is the largest unexamined
  surface, and it is where "protocol-neutral tiers" is likely to turn
  out to be aspirational rather than literal — worth an audit of its
  own before committing to BYOP.
