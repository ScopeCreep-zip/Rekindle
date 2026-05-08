# Security Policy

Rekindle is pre-release software shipping to vulnerable users. Take
security reports seriously; we do.

This file is the **disclosure policy**. For the technical security model
(encryption layers, identity, threat model), see the corresponding docs
in [`docs/security/`](docs/security/).

## Reporting a vulnerability

**Please do not file public issues for security vulnerabilities.**

Instead, use one of the following private channels:

- **GitHub private vulnerability report:** open a report at
  <https://github.com/ScopeCreep-zip/Rekindle/security/advisories/new>.
  This is the preferred channel — GitHub will route the report to the
  maintainers and provide a coordinated-disclosure workspace.
- **Email:** `security@scopecreep.zip`. PGP-encrypt sensitive details
  with the maintainer key (fingerprint published below; if it is missing
  or you cannot find a current copy, send a minimal initial mail asking
  for the current key and we will reply with a fresh one).

When reporting, please include:

1. A description of the issue and the security impact you anticipate.
2. The version, commit hash, or pre-built artifact you tested against.
3. Reproduction steps. A self-contained proof-of-concept is appreciated
   but not required.
4. Your preferred name or handle for credit (or "anonymous" if you'd
   rather not be named).

We will acknowledge receipt within **3 business days** and aim to provide
a substantive response (triage outcome, expected timeline) within **10
business days**. Critical issues affecting release artifacts will be
prioritized over feature work.

## Maintainer PGP key

A current maintainer PGP key fingerprint will be published here once the
project has its first signed release. Until then, GitHub's private
vulnerability reporting flow provides equivalent confidentiality.

```
Fingerprint: TBD — first published with v0.1.0 release tag
```

## Coordinated disclosure

We follow [coordinated disclosure](https://en.wikipedia.org/wiki/Coordinated_vulnerability_disclosure)
norms:

- We will work with you to confirm and patch the issue.
- We will request a reasonable embargo period, typically **90 days from
  initial report**, extendable by mutual agreement if a fix is complex.
- We will credit you in the advisory and changelog (unless you ask us
  not to).
- We will publish a GitHub Security Advisory when the fix ships.

If a vulnerability is being actively exploited, we will accelerate the
timeline.

## Scope

The following are **in scope** for security reports:

- Cryptographic primitives, key derivation, key storage, MEK rotation,
  Signal Protocol session handling.
- Veilid integration: route lifecycle, DHT record handling, gossip mesh
  validation.
- Privacy properties: pseudonym unlinkability across communities,
  metadata leakage, timing channels, traffic analysis surface.
- Authentication and authorization: invite handling, slot claims,
  CRDT permission validation, reader-validates enforcement.
- Memory safety in Rust crates that use `unsafe`, FFI, or custom
  serialization.
- Supply chain: dependency confusion, lockfile tampering, build script
  hijacking.
- The Tauri desktop app: IPC permission boundaries (capabilities), file
  system access, deep link handling.
- The CLI / daemon track: Noise IK IPC bus, OS keyring use, daemon
  privilege boundaries.

The following are **out of scope**:

- Reports against `legacy/` artifacts. The `legacy/` directory contains
  reverse-engineering material from the original Xfire installer and is
  static reference data, not running code.
- Vulnerabilities in upstream dependencies (Veilid, iota_stronghold,
  rusqlite, Tauri) — please report those directly to the upstream
  project. We are happy to help coordinate.
- DoS via excessive resource use that requires an already-trusted peer
  (community member with `MANAGE_*` permission). Misbehavior by trusted
  peers is governed by the moderation system.
- Issues that require physical access to an unlocked, logged-in device.
- Theoretical issues in standard primitives (Ed25519, Curve25519,
  AES-256-GCM, SHA-256, BLAKE3) without a concrete implementation flaw
  in our code.

## Supported versions

Rekindle is pre-1.0. We do not yet maintain backports or LTS branches.
The supported version is **`main`** (and the active feature branches
listed in [`docs/roadmap.md`](docs/roadmap.md)).

| Version | Supported |
|---------|-----------|
| `main` | Yes |
| `codex/communities-*` (active feature branches) | Yes |
| Anything older | No |

Once the first tagged release lands, this table will be updated to track
the latest minor version with a supported window.

## Threat model

The architectural threat model — what we protect against, what we do not,
and what assumptions we make — lives at
[`docs/security/threat-model.md`](docs/security/threat-model.md). Reports
that align with documented threat-model gaps are still welcome (we may
have under-estimated the risk).

## Hall of fame

Security researchers who responsibly disclose issues will be credited
here once the project has its first published advisory. We do not
currently run a paid bug-bounty program; that may change once Rekindle
is past its first stable release.
