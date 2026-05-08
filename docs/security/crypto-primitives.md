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
| Channel content AEAD | **AES-256-GCM** | `aes-gcm` |
| At-rest / transport AEAD (Veilid layer; Stronghold; chunk FEK) | **XChaCha20-Poly1305** | `chacha20poly1305` (via Veilid + Stronghold + Rekindle file FEK) |
| 1:1 messaging | **Signal Protocol** (X3DH + Double Ratchet) | `libsignal-protocol`-derived (in `rekindle-crypto`) |
| Hash / KDF / dedup | **SHA-256** | `sha2` |
| Hash (rotator selection, content addressing) | **BLAKE3** | `blake3` |
| Key derivation | **HKDF-SHA256** | `hkdf` |
| Passphrase KDF | **Argon2id** | `rust-argon2` (via `iota_stronghold`) |
| IPC bus handshake | **Noise IK** (`Noise_IK_25519_ChaChaPoly_BLAKE2s`) | `snow` |
| Audio codec (not crypto, but listed for completeness) | **Opus** | `opus` |

## 1. Ed25519 — identity, message signing

**Where used.** Every long-term identity. Every per-community pseudonym.
Every gossip envelope signature. Every governance entry signature.
Every `MessagePayload` 1:1 envelope signature.

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
- Deriving the DM MEK directly from `X25519(alice_private,
  bob_public)` — no separate key-exchange round-trip.
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
- **Post-quantum hybrid (X25519+Kyber768 or similar).** Considered but
  not yet adopted. The TLS ecosystem is still settling on which hybrid
  to standardise. We will revisit once IETF and Signal take a
  definitive position; jumping early would lock us into a hybrid that
  may be deprecated. **Open work.**

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
- **Stronghold at-rest encryption (Layer 5)** — passphrase-derived
  key.
- **File chunk encryption** — per-file FEK encrypts each chunk.
- **MEK wrapping** during peer-to-peer key delivery — wraps the new
  MEK with the X25519 ECDH-derived shared secret.

**Why XChaCha specifically (vs. plain ChaCha20-Poly1305).**

- 192-bit nonce (XChaCha) vs 96-bit (ChaCha). With random nonces, the
  larger space removes any need to track "have I seen this nonce
  before?" for long-lived keys (Stronghold vault, file FEK over many
  chunks).
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
  the file FEK / Stronghold lifetimes is more error-prone than just
  using a 192-bit random nonce.

## 5. Signal Protocol — 1:1 friend messaging

**Where used.** Layer 4 for 1:1 friend messages. X3DH for session
establishment over DHT-published prekey bundles; Double Ratchet for
ongoing per-message forward and backward secrecy.

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
- No central server required. X3DH operates against published prekey
  bundles, which Rekindle stores in the DHT under the user's profile
  record.

**Implementation.** `crates/rekindle-crypto/src/signal/` contains the
session manager, with stores backed by Stronghold. Our types follow
the [libsignal-protocol](https://github.com/signalapp/libsignal) data
shapes.

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
- **Slot keypair derivation:**
  ```
  slot_seed_per_subkey = HKDF-SHA256(
      ikm:  community_slot_seed,
      salt: "rekindle-slot-keypair-v1",
      info: subkey_index_le32,
  )
  ```
- **DM MEK derivation:**
  ```
  dm_mek = HKDF-SHA256(
      ikm:  X25519(alice_priv, bob_pub),
      salt: SHA256(sorted(alice_pub ‖ bob_pub)),
      info: "rekindle-dm-mek-v1",
  )
  ```
- **DM MEK ratchet:**
  ```
  mek_n+1 = HKDF-SHA256(mek_n, info="rekindle-dm-ratchet-v1")
  ```
- **Direct call `call_key`** (within `rekindle-calls`).

**Why.** RFC 5869 standard. Domain-separated by an explicit `info`
string for every distinct purpose — no two derivations share an
`info`, so the same input keying material cannot accidentally yield
the same output across contexts.

## 9. Argon2id — passphrase KDF

**Where used.** Stronghold vault key derivation from the user's
passphrase.

**Why.**

- Memory-hard, resistant to GPU and ASIC brute force.
- Argon2id is the recommended variant (combines Argon2i's
  side-channel resistance with Argon2d's GPU-resistance).
- Winner of the [Password Hashing Competition](https://www.password-hashing.net/).
- Standardised as [RFC 9106](https://datatracker.ietf.org/doc/html/rfc9106).

**Implementation note.** `iota_stronghold` uses `rust-argon2`
internally. Debug builds make Argon2 painfully slow; the workspace
overrides debug optimisation for that crate:

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
  (responder) has a long-term Ed25519 / X25519 key in the OS keyring
  that the client knows in advance; the client (initiator) is
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
exploit. We use the standard parameters; users with extreme threat
models route voice through higher-hop safety routes (trading latency
for sender anonymity).

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
