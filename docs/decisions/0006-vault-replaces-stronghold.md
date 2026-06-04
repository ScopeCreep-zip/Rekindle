# 0006 — Replace `iota_stronghold` with the SQLCipher-backed `rekindle-vault`

- **Status:** Accepted
- **Date:** 2026-06

## Context and problem statement

The original at-rest store for identity secrets, Signal sessions,
MEKs, and slot seeds was `iota_stronghold`, used directly (not via
the Tauri plugin) so we could control per-identity snapshot files
and Argon2 parameters. Stronghold worked, but three pressures
accumulated against keeping it:

1. **Schema rigidity.** Stronghold is a key-value vault whose
   namespacing API is awkward to use as more secret categories
   appeared (Signal stores, per-community MEKs, per-channel MEKs,
   audit MAC key, slot keypairs, slot seeds, PQXDH secrets,
   cross-device master secret). Every category needed its own
   bespoke serialisation wrapper.
2. **Build cost.** Stronghold's transitive deps pulled in
   `iota_stronghold`, `iota_crypto`, `rust-argon2`, `scrypt`,
   `bee-message`, and several others, inflating build time on
   workspace `cargo check`.
3. **Maintenance posture.** Upstream Stronghold development has
   slowed; the project's primary maintainers have shifted focus. We
   would be carrying a critical security dependency without active
   upstream review.

We need an at-rest store that gives us:

- Per-identity isolated files.
- Argon2id-based KDF from the user's passphrase.
- Authenticated encryption at rest.
- A flexible schema that can accommodate new secret kinds without
  growing the dependency graph.
- An actively maintained upstream.

## Decision drivers

- **Threat model parity.** The new store must be at least as
  resistant to offline attack as Stronghold.
- **Familiar surface.** Engineers should not need to learn a new
  cryptographic protocol to audit it.
- **Pre-ship-acceptable migration.** Rekindle has no installed user
  base; we can break compatibility cleanly without a migration
  path.

## Considered options

### Option A — `rekindle-vault` (SQLCipher double-encryption) — selected

A purpose-built Tier-2 crate that owns a SQLCipher database per
identity. Two encryption layers:

1. SQLCipher page-level AES-256-CBC, keyed by
   `BLAKE3-keyed("rekindle v1 vault-sqlcipher", master)`.
2. Per-entry AES-256-GCM seal, keyed by
   `BLAKE3-keyed("rekindle v1 vault-entry-gcm", master)`.

`master = Argon2id(passphrase, salt)`. The salt lives in a
plaintext sidecar (`{vault}.salt`) because salts only need to be
unique per install.

The schema is a single `entries(namespace, key, nonce, ciphertext)`
table — new secret categories add rows, not migrations.

### Option B — Stay on `iota_stronghold`

The do-nothing alternative. Rejected for the three pressures above.

### Option C — OS-native keychains (macOS Keychain, Windows DPAPI, Linux Secret Service)

Rejected because:

- The same code must run on three platforms; abstracting over
  three keychains adds complexity comparable to keeping our own
  store.
- The keychain APIs are not always available (headless Linux,
  daemon track running under systemd without a user session).
- Backup, export, and cross-device sync need the vault to be a
  file, not a keychain entry.

### Option D — `age`-encrypted file with a fresh password each session

Rejected because `age` is designed for at-rest file encryption with
one shot; iterative updates would be O(N) on every write.

## Decision outcome

Chosen: **Option A — `rekindle-vault`**.

The crate is Tier 2 alongside `rekindle-secrets`. The
`src-tauri/src/keystore/` module wraps it with domain adapters
(signal, community_keys, channel_mek, audit) so call sites do not
need to know about the underlying SQLCipher / GCM split.

## Consequences

**Positive.**

- Single SQL table is much easier to extend than Stronghold's
  namespaced KV.
- Build graph shrinks: we drop `iota_stronghold`, `iota_crypto`,
  `bee-message`, and `rust-argon2` in favour of `argon2` +
  `rusqlcipher` + `aes-gcm` + `blake3`.
- The double-encryption (page + per-entry) gives defence in depth
  even against partial database leaks (e.g., a SQL injection on a
  hypothetical future remote-admin surface — there isn't one
  today, but the property is free).
- SQLCipher is a battle-tested codebase (Signal Desktop, WhatsApp,
  many others).

**Negative.**

- We now own the schema; new categories must follow the project
  pattern (BLAKE3-keyed namespace + GCM seal). Previous Stronghold
  callers got that for free.
- SQLCipher's per-page CBC adds a small per-read overhead vs.
  Stronghold's in-memory cache. Benchmarks show this is
  sub-millisecond on the hot path; not a concern.
- We require Argon2id parameters to be tuned per platform; debug
  builds need `[profile.dev.package.argon2] opt-level = 3` so
  unlock does not take seconds.

**Boundaries.**

- `rekindle-vault` is the sole on-disk store for secret material.
  Every other crate that touches secrets goes through the keystore
  adapters in `src-tauri/src/keystore/`. Enforced by code review
  and grep gauntlet.

## More information

- [`../architecture/data-layer.md`](../architecture/data-layer.md) — vault layout.
- [`../architecture/crates-tier-detail.md`](../architecture/crates-tier-detail.md) — crate-level description.
- [`../security/overview.md`](../security/overview.md) — Layer 4 details and threat-model implications.
