# What we hand-roll that veilid-core already provides

> **Superseded in part.** This document was written without accounting
> for the bring-your-own-protocol goal (SimpleX / Matrix / atproto
> alongside Veilid). Under BYOP, §3's suggestion to adopt
> `CryptoSystem`, §4's `FlapDetector` note, and §5's "collapse toward
> `rekindle-transport`" are all **wrong** — see
> [`veilid-upgrade-under-byop.md`](./veilid-upgrade-under-byop.md) §4.
> The factual findings (§1 byte-identical bridge, §2 the in-tree fork,
> §3's constraint that `CryptoSystem` needs a live node) all stand.

Companion to [`veilid-0.5.7-gap-audit.md`](./veilid-0.5.7-gap-audit.md)
and [`veilid-0.5.7-migration-plan.md`](./veilid-0.5.7-migration-plan.md).

Every claim here is checked against veilid-core 0.5.7 / veilid-tools
0.5.7 sources and our tree — not against changelogs or docs.rs, both of
which have already been wrong twice in this investigation.

**The short version:** three real duplicates, one of which we maintain
*twice over* inside our own tree. But there is a hard architectural
limit on how much of this we can hand back, and it is not a style
preference — see §3.

---

## 1. Confirmed duplicates

### 1.1 Ed25519 → X25519 bridge — byte-identical, verified

- **Ours:** `pseudonym_to_x25519`, `StaticSecret::from(key.to_scalar_bytes())`
  (`crates/rekindle-transport/src/crypto/pseudonym.rs:28`, and again in
  `crates/rekindle-crypto/src/group/pseudonym.rs`).
- **Upstream:** `encapsulation_key_from_signing_key` /
  `decapsulation_key_from_signing_secret` on `CryptoSystem`, VLD0-only
  bridge (`crypto/crypto_system/vld0/mod.rs:282,303`), implemented via
  `secret_to_x25519_sk` = raw unclamped SHA-512 prefix
  (`vld0/mod.rs:46`) and `public_to_x25519_pk` = Edwards→Montgomery
  (`vld0/mod.rs:35`).

Upstream's source carries a pointed warning — "`ed::SigningKey.to_scalar()`
does not produce an unreduced scalar, we want the raw bytes here" — so
whether our `to_scalar_bytes()` agrees is a real question, not a
formality. **It does.** Running both derivations against
ed25519-dalek 2.2 / x25519-dalek 2 on the same seed:

```
ours   sk: 2fad39fe…3928f8      veilid sk: 2fad39fe…3928f8   equal: true
ours   pk: 761d88ec…6edf74      veilid pk: 761d88ec…6edf74   equal: true
ed→montgomery pk: 761d88ec…6edf74   both match: true
DH shared secret equal: true
```

**Consequence:** existing community pseudonym keys work with upstream's
bridge unchanged. Adopting it costs no re-keying. This also retires the
open question flagged at the end of the last round.

### 1.2 MEK hybrid encryption vs RFC 9180 HPKE

- **Ours:** `wrap_mek`/`unwrap_mek` — X25519 ECDH → HKDF-SHA256 (info
  `rekindle-mek-wrap-v1`) → AES-256-GCM, output `[12-byte nonce ||
  ct+tag]`.
- **Upstream:** `hpke_seal`/`hpke_open` on `CryptoSystem` (0.5.7), RFC
  9180 base mode, VLD0 = DHKEM-X25519, with RFC 9180 known-answer tests.

Same construction, standardised. Because §1.1 is byte-identical, the
*keys* carry over; only the ciphertext envelope changes (ours vs HPKE's
`enc || ct`). Still needs a versioned envelope and a dual-read window,
since members will be on mixed versions.

Applies to **group MEK distribution only**. Not the DM path: 0.5.7
implements HPKE for VLD0 and NONE only, and `VLD1` (ML-KEM) is declared
but unimplemented, so our PQXDH stays stronger.

### 1.3 AEAD, RNG, hashing, password KDF

`CryptoSystem` also exposes `encrypt_aead`/`decrypt_aead`,
`crypt_b2b_no_auth`, `random_bytes`, `random_nonce`, `generate_hash`,
`derive_shared_secret`, and `hash_password`/`verify_password`. We use
`aes-gcm`, `rand::RngCore`, `blake3`, `hkdf` and `argon2` directly
across `rekindle-crypto` and `rekindle-vault`.

Most of this is **not** worth handing back — see §3.

---

## 2. The duplicate we own: our crypto is forked in-tree

Independent of anything upstream, these exist **twice**, in two crates,
maintained separately:

| Function | Daemon track | Desktop track |
|---|---|---|
| `derive_community_pseudonym` | `rekindle-transport/src/crypto/pseudonym.rs` | `rekindle-crypto/src/group/pseudonym.rs` |
| `pseudonym_to_x25519` | same | same |
| `wrap_mek` | `rekindle-transport/src/crypto/mek.rs:118` | `rekindle-crypto/src/group/mek_distribution.rs` |
| `unwrap_mek` | same | same |

Both MEK implementations derive their wrapping key with the **same HKDF
info label** — `rekindle-mek-wrap-v1`
(`mek.rs:24`, `mek_distribution.rs:23`) — i.e. they are wire-compatible
forks of one algorithm, kept in sync by hand.

This is the most actionable finding in this document, and it needs no
decision about Veilid at all. A nonce-handling fix, a constant-time
comparison, or the §1.2 HPKE migration currently has to be made twice,
correctly, or the two tracks silently diverge on the wire.

**Recommendation:** collapse to one implementation before attempting
§1.2. Migrating a forked algorithm to HPKE means doing the risky part
twice.

---

## 3. The hard limit: veilid crypto is a node component, not a library

This is why "use what Veilid provides" cannot be applied uniformly, and
it is a structural fact rather than a preference.

`CryptoSystem` is not reachable as a free function. It is obtained from
a **running node**:

- `VeilidAPI::crypto()` → `VeilidAPIResult<VeilidComponentGuard<'_, Crypto>>`
  (`veilid_api/api.rs:107`)
- `Crypto` holds a `VeilidComponentRegistry` and is registered via
  `impl_veilid_component!` (`crypto/mod.rs:77-86`)
- `Crypto::get(kind)` → `Option<CryptoSystemGuard<'_>>` (`crypto/mod.rs:157`)

So every call site that adopts it inherits a live-Veilid dependency.
That collides with two things in our tree:

1. **Architecture rule B2** (`architecture-rules.md:72`): only
   `rekindle-transport` (daemon track) and `rekindle-protocol` (desktop
   track) may import `veilid-core`. `rekindle-crypto`, `rekindle-route`,
   `rekindle-vault` and the other pure-logic crates deliberately do not
   — and `rekindle-crypto`'s dependency list confirms it.
2. **Code that must work before or without attach.** Vault unlock
   (`rekindle-vault`, Argon2 + SQLCipher) happens at login, before a
   node exists. Invite decryption, pseudonym derivation, and every
   crypto unit test in `rekindle-crypto` run with no node at all.

So the split is:

- **Actionable now** — call sites already inside the Veilid boundary
  (`rekindle-transport/src/crypto/*`, `rekindle-protocol`). These can
  use `CryptoSystem` without touching B2.
- **Requires an architecture decision** — `rekindle-crypto/src/group/*`.
  Either amend B2, or move group crypto inside the transport boundary,
  or leave it hand-rolled deliberately. Note that §2's fork means the
  daemon-track copy is already on the permitted side of B2, so
  collapsing the fork *toward* `rekindle-transport` resolves §2 and
  unblocks §1.2 in one move.
- **Should stay hand-rolled** — `rekindle-vault`'s Argon2/SQLCipher key
  derivation, and anything needed pre-attach. Handing these to a node
  component would make login depend on the network coming up.

---

## 4. veilid-tools: available at no dependency cost, but check the fit

`veilid-core` re-exports the whole of veilid-tools —
`pub use veilid_tools as tools;` (`lib.rs:93`) — so
`veilid_core::tools::*` is reachable from any crate already allowed to
import veilid-core. No new dependency, no `deny.toml` entry.

**Correction to the migration plan.** It said our relay circuit breaker
"overlaps `FlapDetector`". Having read
`veilid-tools/src/flap_detector.rs`, that is wrong — they solve
different problems:

- **`FlapDetector`** is a BGP-damping-style *decaying penalty
  accumulator* over an observed value: `record(now_us, value)` counts a
  transition when the value changes, the penalty halves every
  `half_life_us`, and crossing a threshold latches a `flapping` state.
- **Our relay circuit breaker**
  (`rekindle-route/src/relay.rs:27-28`) is a *send-path routing
  decision*: three consecutive failures open the circuit for 60 s, then
  half-open admits exactly one probe. It also encodes §14.5 observed
  peer reliability. `FlapDetector` cannot express half-open.

Where `FlapDetector` **is** a genuine upgrade is `HealGate`
(`rekindle-route/src/lifecycle.rs`), whose fixed 10 s cooldown is a
crude rate-limiter over exactly the kind of oscillation a decaying
penalty models properly.

Both live in `rekindle-route`, which is deliberately Veilid-free
("free of any Veilid / state / persistence coupling", `relay.rs:20`),
so this lands in the same §3 bucket as `rekindle-crypto`: a real
improvement gated on an architecture decision, not a free swap.

---

## 5. Recommended order

**Revised — see [`veilid-upgrade-under-byop.md`](./veilid-upgrade-under-byop.md) §6.**

1. **Collapse the in-tree crypto fork (§2)** — but **downward, into
   `rekindle-crypto`**, deleting the `rekindle-transport` copy. That
   copy imports no `veilid_core` at all and `rekindle-transport`
   already depends on `rekindle-crypto`, so this is a pure deletion.
   (This document originally said to collapse the other way. That was
   wrong: it would home group crypto inside the Veilid boundary, where
   a second transport could not reach it.)
2. **Then HPKE (§1.2)** — via the standard `hpke` crate **directly from
   `rekindle-crypto`**, not via `CryptoSystem`. Veilid itself is just a
   consumer of that crate, so we get the same RFC 9180 construction
   with no node dependency. Keys carry over (§1.1), so this is an
   envelope migration, not a re-key.
3. **Leave §1.3 and the vault alone.**

Do not adopt `CryptoSystem` anywhere: it needs a live Veilid node, so
it breaks both pre-attach code paths and any non-Veilid transport.
