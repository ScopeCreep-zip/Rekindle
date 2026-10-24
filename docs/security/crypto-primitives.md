# Cryptographic Primitives — Selection Rationale

This document lists every cryptographic primitive Rekindle uses, where
it is used, why it was chosen, and what alternatives were considered.
It is the canonical reference for security reviewers asking "why this
algorithm and not that one?"

The implementation is concentrated in **`crates/rekindle-secrets/`**
(Tier 2). No other crate may import `ed25519-dalek`, `x25519-dalek`,
`aes-gcm`, `hkdf`, etc. directly — the boundary is enforced by code
review, not just convention. See [`overview.md`](overview.md) for the
encryption-layer stack and [`threat-model.md`](threat-model.md) for
which primitives address which threats.

## Summary

| Purpose | Primitive | Library |
|---------|-----------|---------|
| Identity / signing | **Ed25519** | `ed25519-dalek` |
| Key agreement | **X25519** | `x25519-dalek` (with `static_secrets`) |
| Post-quantum KEM (PQXDH hybrid) | **ML-KEM-768** (FIPS 203) | `libcrux-ml-kem` (in `rekindle-secrets`) |
| Channel content AEAD | **AES-256-GCM** | `aes-gcm` |
| Real-time voice frames | **SFrame** (RFC 9605, suite 0x0005 `AES_256_GCM_SHA512_128`, HKDF-SHA512) | `aes-gcm`, `hkdf`, `sha2` (in `rekindle-secrets::sframe`) |
| At-rest / transport AEAD (Veilid layer; chunk FEK) | **XChaCha20-Poly1305** | `chacha20poly1305` (via Veilid + Rekindle file FEK) |
| At-rest vault (entry seal) | **AES-256-GCM** + page-level **AES-256-CBC** via SQLCipher | `aes-gcm`, SQLCipher (via `rekindle-vault`) |
| Tamper-evident audit MAC | **BLAKE3-keyed** | `blake3` (via `rekindle-audit`) |
| 1:1 messaging | **Signal Protocol** (PQXDH rev. 3 with X25519 + ML-KEM-768, + Double Ratchet) | spec implementation in `rekindle-crypto` (`signal/`) |
| 1:1 message cipher (Double Ratchet `ENCRYPT`) | **AES-256-CBC + HMAC-SHA256** (DR rev. 4 §7.2, 128-bit tag) | `aes`, `cbc`, `hmac` (in `rekindle-crypto::signal::dr`) |
| Safety numbers | **SHA-512**, 5200 iterations per side (Signal's safety-number scheme) | `sha2` (in `rekindle-crypto::fingerprint`) |
| Pre-session 1:1 frames | **HPKE Base** (RFC 9180) + Ed25519 signature | `rekindle-secrets::hpke` |
| Hash / KDF / dedup | **SHA-256** | `sha2` |
| Hash (rotator selection, content addressing, replay ids, base-key hash, file-cache names, vault key split, governance fork evidence, restore evidence and device-set hashes) | **BLAKE3** | `blake3` |
| Key derivation | **HKDF-SHA256** | `hkdf` |
| Passphrase KDF | **Argon2id** | `argon2` (via `rekindle-vault`) |
| IPC bus handshake | **Noise IK** (`Noise_IK_25519_ChaChaPoly_BLAKE2s`) | `snow` |
| Audio codec (not crypto, but listed for completeness) | **Opus** | `opus` |

## 1. Ed25519 — identity, message signing

**Where used.** Every long-term identity. Every per-community pseudonym.
Every gossip envelope signature. Every governance entry signature.
Every sealed pre-session 1:1 frame signature (`DM_PRESESSION_SIG`) and `IdentityRotated` signature.
Every prekey signature (`PREKEY_SPK_SIG`, `PREKEY_PQ_LR_SIG`, `PREKEY_PQ_OT_SIG`,
`PREKEY_OPK_SIG`; the head's two keys sign its `issued_at` too) and every friend
invite blob (`FRIEND_INVITE_SIG ‖` the JSON of every field but the signature).
Per-community device keys (ADR 0022, Proposed; plan E3.2 S7d-keys) add the signature
contexts `rekindle-gov-device-key-v1` (an endorsement, by the community pseudonym),
`rekindle-gov-device-retired-v1` (a retirement, by the pseudonym),
`rekindle-gov-device-readmit-v1` (a re-admission body, inside an op signed under
`rekindle-gov-op-v1`) and `rekindle-gov-successor-v1` (a pseudonym-signed
`CommunitySuccessor`); ADR 0022 rule 10 gives their layouts.

**Why.**

- IETF-standard ([RFC 8032](https://datatracker.ietf.org/doc/html/rfc8032)),
  widely audited, widely deployed.
- Deterministic signatures (no per-signature randomness needed) —
  removes a class of nonce-misuse / RNG-failure bugs.
- Constant-time, side-channel-resistant implementations are widely
  available (`ed25519-dalek` uses `subtle` for constant-time math).
- Small (32-byte) public keys; small (64-byte) signatures; fast
  verification.
- Curve25519's birational map to X25519 (RFC 7748) lets us reuse the
  same keypair material for ECDH key wrapping (see X25519 below).

**Alternatives considered and rejected.**

- **ECDSA on P-256 / secp256k1.** Required randomness for safety;
  RFC 6979 deterministic ECDSA closes that, but adoption and tooling
  are weaker. P-256 also carries the parameter-choice baggage that
  Curve25519 was designed to avoid.
- **RSA.** Larger keys, slower, more parameter sensitivity. Not
  competitive in 2026 for new code.
- **Ed448.** Higher security margin (224 bits vs 128), but with
  meaningfully larger keys and signatures and weaker tooling. The
  128-bit security target of Ed25519 is sufficient for our threat
  model; Ed448 would be a real cost in bandwidth (every gossip
  envelope carries a signature) without a matching benefit.

## 2. X25519 — Diffie-Hellman key agreement

**Where used.**

- Wrapping channel MEK per recipient during distribution (rotator
  encrypts new MEK to each remaining member with X25519 ECDH +
  XChaCha20-Poly1305).
- PQXDH session establishment, hybrid with ML-KEM-768 (§5), and every
  Double Ratchet DH step.
- HPKE Base sealing of pre-session 1:1 frames to the recipient's key.
- Deriving the per-call `call_key` for direct calls
  (`rekindle-calls`).
- Daemon ↔ client IPC bus key agreement (within Noise IK).

**Why.**

- Same curve family as Ed25519 — converting between the two via the
  birational map (RFC 7748) means a member's pseudonym is one keypair
  used for both signing and key agreement.
- Constant-time scalar multiplication.
- 32-byte public keys and shared secrets.
- IETF-standard, widely deployed, mature implementations.

**`StaticSecret` over `ReusableSecret`.** Rekindle uses
`x25519-dalek`'s `StaticSecret` everywhere it needs DH keys, with the
`static_secrets` feature enabled. `StaticSecret` is `Zeroize` on drop,
which matches our memory-hygiene posture. We do not use `EphemeralSecret`
because most of our DH operations are over long-lived pseudonyms whose
secrets need to survive multiple operations within a session.

**Alternatives considered and rejected.**

- **NIST P-256 ECDH.** Same arguments as for ECDSA — parameter baggage
  and weaker constant-time guarantees in default toolchains.
- **X25519 alone for 1:1 session establishment.** Not used: sessions
  are established with PQXDH rev. 3, which adds ML-KEM-768 (FIPS 203)
  to the X25519 agreements, so a recorded handshake stays confidential
  against a later quantum adversary unless both X25519 and ML-KEM-768
  are broken (§5). The KEM is `libcrux-ml-kem`, a formally verified
  implementation, behind `rekindle-secrets`.

## 3. AES-256-GCM — channel content encryption

**Where used.** Layer 4 content encryption for community channel
messages, encrypted under the channel's current MEK.

**Why.**

- Hardware-accelerated on every modern CPU (AES-NI on x86, ARMv8 AES
  extensions). Throughput is essentially free for the message sizes
  involved.
- Universally supported, standardised in
  [NIST SP 800-38D](https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf).
- AEAD with associated data — we bind ciphertext to its context via
  AAD: `channel_record_key ‖ subkey_index_le32 ‖ lamport_ts_le64`.
  Replays across channels or out-of-position fail decryption.
- Mature, audited implementations.

Voice frames also use AES-256-GCM, through SFrame (RFC 9605 suite
0x0005): deterministic nonces `salt XOR CTR` under per-sender,
per-session keys, so uniqueness rests on the counter and the random
56-bit session tag rather than on random nonces (see
`architecture/voice.md` "Frame encryption").

**Nonces.** 96-bit random nonces from the OS CSPRNG, generated
per-message. The 96-bit space gives ~2⁴⁸ messages before random
collision becomes non-negligible — well beyond what any Rekindle
channel will produce. The MEK rotates aggressively (on every member
departure), so the nonce-reuse exposure window is bounded by the
rotation cadence.

**Alternatives considered and rejected.**

- **ChaCha20-Poly1305.** Equivalent security; slightly slower on AES-NI
  hardware, slightly faster on hardware without it. We use AES-GCM
  here and ChaCha20-Poly1305 elsewhere — the choice is per-context.
  Channel content is hot-path; AES-NI gives a measurable throughput
  win.
- **AES-256-OCB / AES-256-EAX.** Equivalent security; weaker tooling,
  more patent / IPR ambiguity historically.

## 4. XChaCha20-Poly1305 — at-rest, transport, and chunk-FEK AEAD

**Where used.**

- **Veilid transport encryption (Layer 1)** — provided by
  `veilid-core`; not implemented by us.
- **Veilid storage `protected_store_key`** stored in the vault.
- **File chunk encryption** — per-file FEK encrypts each chunk.
- **MEK wrapping** during peer-to-peer key delivery — wraps the new
  MEK with the X25519 ECDH-derived shared secret.

**Why XChaCha specifically (vs. plain ChaCha20-Poly1305).**

- 192-bit nonce (XChaCha) vs 96-bit (ChaCha). With random nonces, the
  larger space removes any need to track "have I seen this nonce
  before?" for long-lived keys (file FEK over many chunks, Veilid
  protected store).
- Constant-time, no timing side channels even on hardware without
  AES-NI.
- IETF-track ([draft-irtf-cfrg-xchacha](https://datatracker.ietf.org/doc/draft-irtf-cfrg-xchacha/));
  widely deployed (libsodium, age, signal-protocol's prekey storage,
  etc.).

**Alternatives considered and rejected.**

- **AES-256-GCM-SIV.** Nonce-misuse-resistant, attractive for the
  same long-lived-key setting. Less mature tooling; we chose XChaCha
  for the same robustness with broader implementation availability.
- **AES-256-GCM with a 96-bit nonce.** Tracking nonce uniqueness over
  the file FEK lifetime is more error-prone than just using a 192-bit
  random nonce. (Note: the vault *does* use AES-256-GCM for per-entry
  sealing, but each entry is sealed with a freshly generated 96-bit
  nonce within a single per-key namespace, so the collision risk is
  bounded by the entry-write rate, not by the file-chunk count.)

## 5. Signal Protocol — 1:1 friend messaging

**Where used.** Layer 4 for 1:1 friend messages. PQXDH (rev. 3) for session
establishment over each device's published prekey record; Double Ratchet for
ongoing per-message forward and backward secrecy. Pre-session kinds
(friend request and reject, `PreKeyRejected`, session reset request and
decline, push-relay kinds) use HPKE Base to the recipient's X25519 key plus an Ed25519
signature over `DM_PRESESSION_SIG ‖ recipient ‖ typeId ‖ enc ‖ ciphertext`
(`rekindle-secrets::hpke`). Safety numbers are Signal's 60-digit and
scannable format: 5200 iterations of SHA-512 per side
(`rekindle-crypto::fingerprint`).

**Why.**

- The gold standard for 1:1 messaging confidentiality. Decade of
  formal analysis, real-world auditing, and adversarial scrutiny.
- Per-message keys via the symmetric ratchet — every message uses a
  unique key, derived from the ratchet state.
- Per-DH-step forward secrecy via the Diffie-Hellman ratchet — even a
  long-term key compromise does not retrospectively decrypt prior
  messages.
- Self-healing: a fresh DH on every send recovers from a compromised
  message key on the next round-trip.
- No central server required. PQXDH operates against a prekey record
  per device (plan E2.4): a DFLT(101) DHT record named from profile
  subkey 5, whose subkey 0 holds the signed prekey and the ML-KEM
  last-resort key and whose subkeys 1..=100 each hold one one-time
  X25519 and ML-KEM key pair. The initiator picks an unused slot
  uniformly, or the head alone when none is left (PQXDH §2.5, §3.3).
  Every id is a random non-zero u31, and every signature is the
  identity's Ed25519 over domain ‖ id ‖ key: `PREKEY_SPK_SIG` (signed
  prekey), `PREKEY_PQ_LR_SIG` and `PREKEY_PQ_OT_SIG` (ML-KEM last-resort
  and one-time), `PREKEY_OPK_SIG` (one-time curve key), so a signature
  binds one key to one id in one role. The head's two signatures also
  cover its `issued_at`, and a reader refuses a head dated more than
  14 days from now either way (`SIGNED_MAX_AGE_MS`,
  `CryptoError::StalePrekeys`) (`rekindle-crypto::signal::pqxdh::verify`).

**Implementation.** `crates/rekindle-crypto/src/signal/{dr,record,pqxdh}`
hold the pure Double Ratchet, `SessionRecord` and PQXDH state machines; the
sessions are stored per peer in `ratchet_sessions` and stepped in one DB
transaction before any send (`crates/rekindle-db/src/ratchet/`), prekeys in
`signal_prekeys`. The record's maintenance and pick are the pure
`rekindle-crypto::signal::prekey_record::{plan_maintenance, pick_slot}`:
the head rotates every 2 days, a consumed slot is refilled, and replaced
keys stay usable for 30 days. A one-time key is consumed only after the
init's first message decrypts, in the same transaction that stores the
session and refills the slot; an init naming a consumed, unknown or
archived key gets a sealed `PreKeyRejected` and the initiator retries once
from another slot; a replayed last-resort-only init is dropped. The ratchet associated data binds both identities and the
frame typeId. Skipped message keys age out at the 30-day
`DELIVERY_HORIZON`. Our types follow the
[libsignal-protocol](https://github.com/signalapp/libsignal) data
shapes.

**Message encryption.** `ENCRYPT` is DR rev. 4 §7.2's recommended
construction (`rekindle-crypto::signal::dr::kdf`): HKDF-SHA256 of the
message key (zero salt, info `rekindle-dr-msg-v2`) gives 80 bytes — an
AES-256 key, an HMAC-SHA256 key and a 16-byte IV; the plaintext is
AES-256-CBC with PKCS#7 padding, and the tag is HMAC-SHA256 over
`AD ‖ ciphertext`, truncated to 128 bits (§8.6). The tag is checked in
constant time before any decryption, and every failure (length, tag,
padding) is the same error. The reason is misuse resistance under state
rollback: a restored or cloned ratchet state (power loss, a backup, a VM
snapshot, a copied data root) reuses message keys, and with them the
IV. Under an AEAD over a counter mode, such as the AES-256-GCM this
replaced (`rekindle-dr-msg-v1`, retired), one reuse gives the XOR of
the two plaintexts and the authentication key; under CBC+HMAC it shows
only whether the two plaintexts share leading blocks. MLS met the
same problem with AES-GCM and had to add a per-message reuse guard
(RFC 9420 §6.3.1). Considered and rejected: **AES-256-GCM-SIV** (it leaks
only plaintext equality under reuse, but the RustCrypto `aes-gcm-siv`
crate states it has never been audited, it is not the Double Ratchet
spec's concrete recipe, and the spec gives a truncation analysis only for
CBC+HMAC, §8.6) and **ChaCha20-Poly1305** (the same keystream-reuse failure
as GCM). libsignal and Olm/vodozemac use this same construction, and its
`aes` and `cbc` crates were already in the lock. The threat model's
rollback row lists the other mitigations.

**PQXDH `info`.** The PQXDH KDF's info is `rekindle-pqxdh-sk-v1`
(`domains::PQXDH_SK`), not the `info_curve_hash_pqkem`-shaped string of
PQXDH rev. 3 §2.2. Every label follows the registry's
`rekindle-<purpose>-v<N>` naming rule, and the suite that string would
name is fixed and recorded here: curve X25519, hash SHA-256, KEM
ML-KEM-768. A different suite takes a new label.

**Alternatives considered and rejected.**

- **Olm / Megolm (Matrix).** Olm is essentially Signal's Double
  Ratchet; Megolm is the group-messaging variant we did not need
  (communities use MEK + reader-validates governance, not Megolm).
- **MLS (Messaging Layer Security, RFC 9420).** Specifically designed
  for groups. Considered for community channels; rejected because
  MLS's ratchet-tree model assumes a coordination point for the group
  state machine, which conflicts with the chiral-network "no
  coordinator" property. v3 may revisit if a CRDT-friendly MLS
  variant matures.

## 6. SHA-256 — hashing

**Where used.**

- Per-chunk hash (`AttachmentOffer.chunk_hashes[i] =
  SHA256(plaintext_chunk_i)`).
- Flat-list Merkle root (`merkle_root = SHA256(chunk_hashes
  concatenated)`).
- HKDF underlying hash (HKDF-SHA256).
- General-purpose content hashing where compatibility with
  widely-deployed standards matters more than raw speed.

**Why.** Industry standard, ubiquitous tooling, FIPS-approved when
that matters to deployment contexts. The construction `merkle_root =
SHA256(concatenated)` will move to a true binary Merkle tree
(BEP-52-style) in v2 of the file format; SHA-256 stays.

## 7. BLAKE3 — fast keyed hashing

**Where used.**

- **Deterministic rotator selection.** `rotator =
  argmin_member(BLAKE3(departed_pseudonym ‖ own_pseudonym))`.
- **Replay ids** of pre-session frames: `blake3::derive_key("rekindle-dm-replay-id-v1",
  sender ‖ nonce)` (`rekindle-crypto::replay`).
- **Base-key hash** a `PreKeyRejected` names: `derive_key` under
  `rekindle-prekey-rejected-base-key-v1`.
- **Invite-code lookup id** (the `code_hash` an invite entry is stored
  under): `derive_key` under `rekindle-invite-code-id-v1` over the code
  (`rekindle_secrets::invite::hash_invite_code`).
- **File-cache names**: every directory under `state/{pk}.files/` is a
  keyed BLAKE3 hash (`FILE_CACHE_COMMUNITY`, `FILE_CACHE_DM`,
  `FILE_CACHE_ATTACHMENT` prefixes) under a key
  HKDF-derived from the identity secret (`FILE_CACHE_NAMES`), so names
  are never ids and are unlinkable across identities (plan E3.0a).
- **Vault key split**: `derive_key` of the Argon2id master under
  `rekindle-vault-{sqlcipher,entry,app-db}-v1` (§9).
- **Governance device keys** (ADR 0022, Proposed; plan E3.2 S7d-keys):
  fork evidence under `rekindle-gov-fork-evidence-v1` over `community ‖
  device key ‖ seq ‖` the two op hashes in byte order; the "retired by
  restore" marker under `rekindle-gov-restore-evidence-v1` over `community
  ‖ deviceKey ‖ incarnation ‖ lastSeq ‖` the published `GovHead` op hash;
  the extended-set hash under `rekindle-gov-device-set-v1` over the sorted
  ids `(kind, deviceKey, incarnation)` of the member's endorsement set.
- **MekTransfer ack tag** (plan E3.5; prod research batch 11, decided 2026-10-24): `ack_tag` is keyed BLAKE3 under
  `derive_key("rekindle-mek-ack-v1", MEK_G)` over `community ‖ scope ‖ generation ‖ transfer_id ‖ inviter ‖
  joiner`, a key-confirmation MAC after MLS `confirmation_tag` (RFC 9420 §12.4.3), compared in constant time; no
  signature, for deniability.
- **Content addressing** in places where we want speed and a wider
  output range (32-byte BLAKE3 vs 32-byte SHA-256, but BLAKE3 is
  variable-length-output friendly without the truncation patterns
  SHA-256 needs).

**Why.**

- 5× to 10× faster than SHA-256 on commodity hardware.
- Tree-mode parallelism (irrelevant for our short inputs but available).
- Modern, well-audited (BLAKE2/3 family, IETF
  [draft-irtf-cfrg-blake3](https://datatracker.ietf.org/doc/draft-irtf-cfrg-blake3/)).
- Distinct from SHA-256 — using both means a single hash-function
  weakness in either does not affect both code paths.

We do **not** use BLAKE3 for HKDF or signature digests — those stay on
SHA-256 for compatibility with the Signal Protocol stack and HKDF-SHA256
ecosystem.

## 8. HKDF-SHA256 — key derivation

**Where used.**

- **Per-community pseudonym derivation:**
  ```
  pseudonym_seed = HKDF-SHA256(
      ikm:  master_secret,
      salt: "rekindle-community-pseudonym-v1",
      info: community_id,
  )
  Ed25519::from_seed(pseudonym_seed)
  ```
- **Slot keypair derivation** (`rekindle-secrets/src/derive.rs`):
  ```
  slot_seed_per_subkey = HKDF-SHA256(
      ikm:  community_slot_seed,
      salt: none,
      info: "rekindle-slot-" ‖ decimal(slot_index),
  )
  ```
- **Group DM MEK:** random, wrapped per recipient in the `GroupDmInvite`
  (the 2-party ECDH MEK went with the 1:1 SMPL path, plan E2.3; 1:1 DMs
  are the friend's ratchet stream). E2.6 replaces it with sender-owned keys.
- **Group DM MEK ratchet:**
  ```
  mek_n+1 = HKDF-SHA256(mek_n, info="rekindle-dm-ratchet-v1")
  ```
- **Direct call secret** (within `rekindle-calls`): the SFrame scope
  secret for the call.
- **SFrame** (`rekindle-secrets::sframe`, RFC 9605 §4.4.2): HKDF-SHA512
  key/salt per KID, and per-sender base keys
  `HKDF-SHA512(scope_secret, "rekindle-voice-sender-key-v1" ‖ sender_key ‖ tag)`
  (§5.1). See `architecture/voice.md` "Frame encryption".

**Why.** RFC 5869 standard. Domain-separated by an explicit `info`
string for every distinct purpose — no two derivations share an
`info`, so the same input keying material cannot accidentally yield
the same output across contexts.

Every label (HKDF info/salt, signature prefixes, BLAKE3 contexts, nonce
prefixes) is declared once in `crates/rekindle-types/src/domains.rs`.
Its tests reject duplicate labels and any label that is a prefix of
another; `cargo xtask check-domain-literals` rejects label-shaped
literals anywhere else ([architecture rule B18](../contributor/architecture-rules.md)).

## 9. Argon2id — passphrase KDF

**Where used.** `rekindle-vault` master-key derivation from the
user's passphrase. The Argon2id output is then split with
`blake3::derive_key` into the vault's SQLCipher page key
(`rekindle-vault-sqlcipher-v1`), its per-entry GCM key
(`rekindle-vault-entry-v1`) and the identity database's SQLCipher key
(`rekindle-vault-app-db-v1`).

**Why.**

- Memory-hard, resistant to GPU and ASIC brute force.
- Argon2id is the recommended variant (combines Argon2i's
  side-channel resistance with Argon2d's GPU-resistance).
- Winner of the [Password Hashing Competition](https://www.password-hashing.net/).
- Standardised as [RFC 9106](https://datatracker.ietf.org/doc/html/rfc9106).

**Implementation note.** `rekindle-vault` depends on the `argon2`
crate directly. Debug builds make Argon2 painfully slow; the
workspace overrides debug optimisation for the relevant crates:

```toml
[profile.dev.package.rust-argon2]
opt-level = 3
```

## 10. Noise IK — IPC bus handshake

**Where used.** Daemon ↔ client IPC bus (`rekindle-node` ↔
`rekindle-cli`).

**Pattern.** `Noise_IK_25519_ChaChaPoly_BLAKE2s`:

| Component | Choice |
|-----------|--------|
| DH | X25519 |
| AEAD | ChaCha20-Poly1305 |
| Hash | BLAKE2s |
| Pattern | IK — initiator's static key transmitted; responder's static key pre-known |

**Why.**

- IK pattern matches the threat model perfectly: the daemon
  (responder) has a static X25519 key, generated at each start and held
  only in memory, whose public half (`bus.pub`) the client reads in
  advance; the client (initiator) is
  ephemeral and freshly generated.
- 1-RTT handshake — minimal latency.
- Forward secrecy (transport-phase ephemerals).
- Mutual authentication: handshake fails if either side's static key
  doesn't verify.
- UCred mixing in the prologue cryptographically binds the OS-level
  process credentials to the encrypted channel — see
  [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md).

**Library.** [`snow`](https://github.com/mcginty/snow), pinned to a
specific git revision to avoid lockfile drift on a cryptographic
dependency.

## 11. Veilid transport (Layer 1) — `XChaCha20-Poly1305` end-to-end-ish

`veilid-core` handles transport encryption transparently. We do not
implement this; we depend on it. Veilid's primitives are documented at
the project's gitlab and in
[`reference_veilid_privacy`](#references) below. The Veilid transport
gives us hop-by-hop encryption between Veilid nodes, plus the safety /
private route system that builds on top.

## 12. Opus — voice codec (non-crypto)

**Where used.** Voice channels. Configured for VoIP (`opus::Application::Voip`),
48 kHz mono, 20 ms frames, 32 kbps, in-band FEC. See
[`../architecture/voice.md`](../architecture/voice.md).

Listed here because every choice in the audio path has security
implications: Opus's algorithmic delay and bitrate behaviour are part
of the timing-side-channel surface that voice traffic analysis would
exploit. We use the standard parameters. Voice is not a privacy
exception: it rides the same 3-hop Tor-class `SafetySelection::Safe`
route as every other path (uniform `ANONYMITY_HOP_FLOOR`), so sender
anonymity holds for voice too — the only per-class difference is that
voice selects `Stability::LowLatency` within that floor. The
mouth-to-ear cost of three relay hops is budgeted to the ITU-T G.114
interactive band.

## What we do not use

| Primitive | Why not |
|-----------|---------|
| **MD5** | Broken. Not used anywhere. |
| **SHA-1** | Deprecated. Not used anywhere. (`git`-ecosystem dependencies that internally use SHA-1 for object IDs are not security-relevant for us.) |
| **3DES** | Obsolete. |
| **RC4** | Broken. |
| **Telegram-style "MTProto"** | Custom protocol with a contested security analysis. Signal Protocol is the conservative choice. |
| **OpenPGP message format** | Heavy; key-handling complexity disproportionate to our needs; modern issues with metadata leakage and lack of forward secrecy. We use it nowhere in the protocol layer. |
| **JWT for IPC tokens** | We do not have HTTP-style sessions. The IPC bus uses Noise IK; the daemon track does not need bearer tokens. |
| **Custom AEADs / homebrew constructions** | Forbidden by policy. Crypto primitives must be reviewed, audited, and standard. |

## References

- [RFC 7748 — Curve25519 / X25519](https://datatracker.ietf.org/doc/html/rfc7748)
- [RFC 8032 — Ed25519](https://datatracker.ietf.org/doc/html/rfc8032)
- [NIST SP 800-38D — GCM](https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38d.pdf)
- [draft-irtf-cfrg-xchacha — XChaCha20-Poly1305](https://datatracker.ietf.org/doc/draft-irtf-cfrg-xchacha/)
- [RFC 5869 — HKDF](https://datatracker.ietf.org/doc/html/rfc5869)
- [RFC 9106 — Argon2](https://datatracker.ietf.org/doc/html/rfc9106)
- [Noise Protocol Framework](http://noiseprotocol.org/)
- [Signal Protocol](https://signal.org/docs/) — X3DH, Double Ratchet
- [BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs)
- [BEP-52 (BitTorrent v2)](https://www.bittorrent.org/beps/bep_0052.html) — referenced for the future binary-tree Merkle layout
- [Veilid developer book](https://veilid.gitlab.io/developer-book/) — primitive semantics
