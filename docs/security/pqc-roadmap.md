# Post-Quantum Cryptography Migration Plan

Rekindle's cryptography is currently entirely classical: Ed25519
signatures, X25519 key agreement, AES-256-GCM and XChaCha20-Poly1305
AEADs, BLAKE3 / SHA-256 hashes, HKDF-SHA256, Argon2id. Every primitive
is NIST-approved or CFRG-recommended; none is post-quantum-secure
against a sufficiently large quantum computer.

This document captures the migration plan: what we ship today, what
threat we are tracking, and how Rekindle will adopt post-quantum
primitives in line with the wider messaging-app ecosystem.

## 1. Current state

| Purpose | Primitive | Quantum status |
|---------|-----------|----------------|
| Identity / signatures | Ed25519 | Classical-secure; **not PQ-secure** |
| Key agreement (1:1, DM, MEK wrap, IPC bus) | X25519 (within Signal X3DH, ECDH for DMs, Noise IK) | **Not PQ-secure** |
| Symmetric content encryption | AES-256-GCM | PQ-secure with 256-bit key (Grover halves effective security to ~128 bits — still adequate) |
| Symmetric content encryption | XChaCha20-Poly1305 | PQ-secure (Grover applies; same reasoning) |
| Hash | BLAKE3, SHA-256 | PQ-secure with 256-bit output |
| Passphrase KDF | Argon2id | Memory-hard; not affected by Grover at the relevant scale |

The two non-PQ-secure layers are **signatures** (Ed25519) and **key
agreement** (X25519). Symmetric primitives are essentially fine with a
size bump that we already have.

## 2. The threat we are tracking

**Harvest-now-decrypt-later (HNDL).** A network adversary captures
ciphertext today and stores it for years. When a cryptographically-
relevant quantum computer arrives, the captured X25519 key agreement
becomes recoverable, the session keys derivable, and historical
content readable.

This threat applies asymmetrically to the two primitive classes:

- **Key agreement is the priority.** A captured handshake plus a
  future quantum computer breaks past confidentiality. Migrating key
  agreement to a post-quantum hybrid removes most of the HNDL surface.
- **Signatures matter less for HNDL.** A signature's purpose is
  authenticity at the time of receipt. A future quantum computer
  breaking Ed25519 lets an attacker forge new signatures, but does
  not retroactively make past valid signatures forgeable. Migration
  is still important — for forward authenticity — but less urgent.

For Rekindle's threat model (vulnerable users, harvest-now-decrypt-
later assumed), key-agreement migration is the load-bearing change.

## 3. NIST PQC standards (Aug 2024)

Three FIPS published August 14, 2024:

- **FIPS 203 — ML-KEM** (Module-Lattice-Based Key Encapsulation Mechanism, derived from Kyber). Three parameter sets: ML-KEM-512, ML-KEM-768, ML-KEM-1024.
- **FIPS 204 — ML-DSA** (Module-Lattice-Based Digital Signature Algorithm, derived from Dilithium).
- **FIPS 205 — SLH-DSA** (Stateless Hash-Based Digital Signature Algorithm, derived from SPHINCS+).

ML-KEM is the priority candidate for our key-agreement migration.
ML-DSA is the candidate for eventual signature migration.

## 4. Production deployment in 2024–2026

Three reference points for the messaging ecosystem:

- **Signal — PQXDH** (Sep 2023): hybrid X25519 + Kyber-1024 in the
  initial X3DH handshake. Signal Protocol's authentication remains
  classical Ed25519. Spec:
  <https://signal.org/docs/specifications/pqxdh/>.
- **Apple iMessage PQ3** (March 2024, iOS 17.4+): hybrid ECC +
  ML-KEM. Apple mixes a post-quantum key into the ratchet every 50
  message epochs. Public design paper:
  <https://security.apple.com/blog/imessage-pq3/>.
- **Cloudflare TLS** (2024-2025): X25519+ML-KEM-768 hybrid handshakes
  enabled by default. As of early 2026, ~60% of human HTTPS traffic
  on Cloudflare uses hybrid PQC. Reference:
  <https://blog.cloudflare.com/pq-2025/>.

The pattern is consistent: **hybrid X25519 + ML-KEM-768** is the
de-facto choice for messaging and TLS through 2024–2026. ML-KEM-768
hits the same security level as X25519 (NIST Level 1) without the
computational cost of ML-KEM-1024.

## 5. Rekindle migration plan

### Phase 1 — current (classical only)

What we ship today.

- All primitives are NIST-approved classical.
- The PQ migration *plan* (this document) is published.
- The codebase has no PQ dependencies yet — adding them prematurely
  would commit us to whichever Rust ML-KEM crate stabilises first,
  which is still in flux.

### Phase 2 — pre-1.0 / `v0.1.0`

Goal: be PQ-ready without yet shipping a hybrid handshake.

- Document the PQ posture in user-visible release notes (this doc
  becomes a public reference).
- Track the Rust ML-KEM crate landscape: `pqcrypto-mlkem`, RustCrypto
  `ml-kem`, `liboqs-rust`. Pick the candidate when the API stabilises.
- Add a PQ-handshake feature flag (compiled out by default) so the
  trait surface for hybrid key agreement exists in the codebase but
  is not on the production code path.
- For the daemon-track IPC bus (`rekindle-node` ↔ `rekindle-cli`):
  evaluate whether `snow` will accept a PQ-mode pattern in time,
  given Noise IK is already a 1-RTT handshake.

### Phase 3 — `v0.2.0` (target)

Goal: ship hybrid X25519+ML-KEM-768 for community channel MEK
distribution and DM MEK derivation.

- **Community channel MEK delivery (rotator → recipient):**
  Wrap the MEK using a hybrid scheme:
  ```
  shared = X25519(rotator, recipient) ‖ ML-KEM-768(rotator → recipient)
  mek_wrap_key = HKDF-SHA256(shared, info="rekindle-mek-pq-wrap-v1")
  wrapped_mek = XChaCha20-Poly1305(mek_wrap_key, mek)
  ```
  Both halves of the shared secret must be broken to compromise the
  MEK — adversary needs both classical X25519 and ML-KEM-768.

- **DM MEK derivation:**
  Today derived as `HKDF(X25519(alice, bob))`. Move to:
  ```
  ikm = X25519(alice, bob) ‖ ML-KEM-768(alice → bob)
  dm_mek = HKDF-SHA256(ikm, salt=SHA256(sorted(pubkeys)), info="rekindle-dm-mek-v2")
  ```

- **Direct-call key:**
  Same hybrid construction in `rekindle-calls`.

- **Bump invite-format version** so v2 invites carry both X25519 and
  ML-KEM-768 public-key material; v1 invites continue to work for
  classical-only sessions.

### Phase 4 — post-1.0

Goal: ship hybrid for **everything**, including signatures.

- **Signal Protocol X3DH:** mirror PQXDH — drop the Kyber bake-in and
  replace it with the standardised ML-KEM-768 once the libsignal
  crate exposes it.
- **Identity signatures:** evaluate ML-DSA (FIPS 204) for new
  identities. Migration of existing identities is a multi-version
  effort because the identity key is the root of every per-community
  pseudonym.
- **Noise IK IPC bus:** if the upstream Noise specification
  standardises a PQ-mode (current proposals: hybrid `Noise_IKpq`),
  adopt it.

### Phase 5 — long-term

Watch the standards. ML-KEM-1024 may become the default if attacks on
ML-KEM-768 advance. Lattice-based cryptanalysis is still maturing;
a serious break (e.g., dimension reduction below the Level 1 bound)
could push the ecosystem to a different PQC family entirely.

## 6. Crate selection criteria

When picking the ML-KEM Rust crate for Phase 2/3:

1. **Constant-time implementation.** No timing side channels on
   secret inputs.
2. **`Zeroize + ZeroizeOnDrop`** on all secret types. Matches our
   `rekindle-secrets` posture.
3. **No `unsafe`** in the implementation, or `unsafe` confined to a
   small audited module with `// SAFETY:` annotations.
4. **Active maintenance.** PQ crates are young; we want one with a
   responsive maintainer and a clean RustSec advisory history.
5. **Audit history.** Bonus if the crate has been independently
   audited (Trail of Bits, Cure53, NCC Group).

Candidates as of late 2025:

| Crate | Notes |
|-------|-------|
| `ml-kem` (RustCrypto) | Pure Rust, follows RustCrypto patterns we already use for `aes-gcm` / `chacha20poly1305`. Best fit if API stabilises in time. |
| `pqcrypto-mlkem` | Wraps the reference C implementation. Production-tested but uses C bindings — adds an `unsafe` surface. |
| `liboqs-rust` | OQS Project's Rust binding. Comprehensive PQ-suite coverage; same FFI tradeoff as pqcrypto. |

We will commit to a choice during Phase 2 and pin via `cargo-vet`.

## 7. FIPS 140-3 disclaimer

Rekindle uses NIST-approved primitives but **does not claim FIPS
140-3 module validation**. FIPS 140-3 certifies cryptographic
modules; Rekindle is an application that consumes cryptographic
modules. Users with regulatory FIPS requirements should consult their
auditor; the relevant validation lives with the upstream crates
(`ed25519-dalek`, `chacha20poly1305`, `ring`, etc.) and their
respective certification programmes.

When PQ migration lands, this disclaimer extends to ML-KEM/ML-DSA: we
use NIST-approved primitives; module validation is upstream.

## 8. Why now (a published plan), even though we don't ship hybrid yet

Three reasons:

1. **HNDL is real today.** Captured ciphertext from 2026 is
   recoverable when CRQC arrives. Documenting the migration plan
   tells users that we take the threat seriously even before we
   ship the fix.
2. **Funder credibility.** Institutional funders (EFF, OTF, Mozilla,
   privacy-focused foundations) increasingly ask "what is your PQC
   posture?" A no-promises-no-plan answer is worse than a documented
   migration timeline.
3. **Researcher engagement.** Cryptography researchers contribute
   more readily to projects that have a clear PQC roadmap they can
   target their work against.

## 9. References

- [NIST FIPS 203 — ML-KEM](https://csrc.nist.gov/pubs/fips/203/final)
- [NIST FIPS 204 — ML-DSA](https://csrc.nist.gov/pubs/fips/204/final)
- [NIST FIPS 205 — SLH-DSA](https://csrc.nist.gov/pubs/fips/205/final)
- [NIST PQC project](https://csrc.nist.gov/projects/post-quantum-cryptography)
- [Signal PQXDH specification](https://signal.org/docs/specifications/pqxdh/)
- [Apple iMessage PQ3 design](https://security.apple.com/blog/imessage-pq3/)
- [Cloudflare PQC adoption (2025)](https://blog.cloudflare.com/pq-2025/)
- [IETF draft — Post-quantum hybrid key exchange in TLS](https://datatracker.ietf.org/doc/draft-ietf-tls-hybrid-design/)
- [`crypto-primitives.md`](crypto-primitives.md) — current primitive choices
- [`threat-model.md`](threat-model.md) — adversary model including HNDL
