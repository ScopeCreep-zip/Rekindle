# Standards Mapping

This document maps Rekindle against the security and supply-chain
standards that an open-source P2P chat application is expected to
align with as of late 2025 / 2026:

- **OWASP ASVS 5.0** (May 2025)
- **OWASP DASVS 1.0** (Desktop Application Security Verification Standard, 2025)
- **OWASP SCVS** (Software Component Verification Standard)
- **OWASP Top 10:2025** (released January 2026)
- **NIST SSDF v1.1 / draft v1.2** (SP 800-218)
- **NIST CSF 2.0** (February 2024)
- **NIST 800-53 Rev. 5** (selected control families)
- **NIST 800-161 Rev. 1** (Supply Chain Risk Management, November 2024)
- **CISA Secure by Design** (2023, pledge May 2024)
- **CISA OSS Security Roadmap** (September 2023, updated February 2024)

For each control we mark:

- **Met** — implemented and verifiable in the codebase or repo.
- **Partial** — partially in place; pending work tracked.
- **N/A** — out of scope and why.
- **Open** — gap identified; tracked under [`../roadmap.md`](../roadmap.md)
  or in a referenced security document.

This is **not** a compliance certification — it's the project's
self-assessment so that auditors, contributors, and institutional
funders can quickly see what's covered. Each row points at the actual
implementation file or doc that demonstrates the control.

---

## OWASP ASVS 5.0

Adapted for a P2P desktop application; web-only chapters (V3
Sessions, V4 Access Control, V5 Validation/Encoding/Injection, V8
Data Protection in Storage at the web-app layer, V12 File Handling,
V13 API/Web Service) are scoped out where they assume a HTTP server.
DASVS (below) covers the desktop-specific concerns ASVS does not.

| Chapter | Control area | Status | Where |
|---------|--------------|--------|-------|
| V1 — Architecture, Design & Threat Modeling | Documented threat model | **Met** | [`threat-model.md`](threat-model.md) (STRIDE + LINDDUN) |
| V1 | Security architecture documented | **Met** | [`../architecture/communities.md`](../architecture/communities.md), [`overview.md`](overview.md), [`crypto-primitives.md`](crypto-primitives.md) |
| V1 | ADRs for architectural decisions | **Met** | [`../decisions/`](../decisions/) (5 ADRs published; format MADR 4.0) |
| V2 — Authentication | Password-based authentication | **N/A** | Identity is an Ed25519 keypair; no passwords. See `../decisions/0002-signal-protocol-for-1to1.md`. |
| V2 | Multi-factor authentication | **N/A** | Same as above. The Stronghold passphrase + device possession is the local equivalent. |
| V2 | Credential storage | **Met** | Stronghold vault (Argon2id + XChaCha20-Poly1305). [`overview.md`](overview.md) §"Layer 5". |
| V6 — Cryptography | Approved primitives only | **Met** | Ed25519, X25519, AES-256-GCM, XChaCha20-Poly1305, BLAKE3, SHA-256, HKDF-SHA256, Argon2id. [`crypto-primitives.md`](crypto-primitives.md). |
| V6 | Authenticated encryption (AEAD) | **Met** | All content uses AEAD (AES-GCM or XChaCha20-Poly1305). |
| V6 | Forward secrecy | **Met** | Signal Double Ratchet (1:1), MEK rotation (community), per-call key (direct calls). |
| V6 | Post-quantum readiness | **Open** | Hybrid X25519+ML-KEM-768 migration plan. [`pqc-roadmap.md`](pqc-roadmap.md). |
| V6 | Side-channel resistance | **Met** | `subtle` crate via `ed25519-dalek`, `x25519-dalek`. Zeroize-on-drop on every secret type. |
| V7 — Errors & Logging | No secret leakage in logs | **Met** | `Debug` impls on `IpcRequest::Unlock` and `IdentityCreate` redact secrets. [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md). |
| V7 | Structured logging | **Met** | `tracing` + `tracing-subscriber` in every crate. |
| V7 | No `dbg!` / `todo!` / `unimplemented!` in shipped code | **Met** | Lints `dbg_macro = "deny"`, `todo = "deny"`, `unimplemented = "deny"` in workspace `Cargo.toml`. |
| V9 — Communications | Transport encryption | **Met** | Veilid hop-by-hop + Signal/MEK end-to-end. [`overview.md`](overview.md). |
| V9 | Certificate / key validation | **Met** | Ed25519 envelope signatures verified before processing. [`../architecture/communities.md`](../architecture/communities.md) §4. |
| V10 — Malicious Code | Dependency vetting | **Partial** | `cargo-audit` + `cargo-deny` in CI; `cargo-vet` config in [`/supply-chain/`](../../supply-chain/). [`supply-chain-policy.md`](supply-chain-policy.md). |
| V10 | Build provenance | **Open** | Reproducible-build CI verification deferred to post-1.0. [`reproducible-builds.md`](reproducible-builds.md). |
| V11 — Business Logic | Replay prevention | **Met** | Veilid sequence numbers per subkey + Lamport ordering + dedup cache. |
| V11 | Rate limiting | **Met** | Per-sender gossip rate limit (10 msg/s default token bucket); per-IPC-connection rate limit (100 req/s). |
| V14 — Configuration | Secure defaults | **Met** | E2E encryption is on, no plaintext fallback, no telemetry. [`privacy-properties.md`](privacy-properties.md). |
| V14 | Configuration in source control | **Met** | `tauri.conf.json`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml`, `flake.nix` all checked in. |

**Target level:** L2 (with V6/V9/V11 controls at L3).

---

## OWASP DASVS 1.0

DASVS is the desktop-app-specific complement to ASVS, addressing
controls ASVS skips because it assumes a web context.

| Chapter | Control area | Status | Where |
|---------|--------------|--------|-------|
| Bootstrapping | Window decorations / chrome | **Met** | Frameless transparent windows, custom Xfire titlebar. [`../architecture/ui-skin.md`](../architecture/ui-skin.md). |
| Local storage | At-rest encryption of secrets | **Met** | Stronghold vault (Argon2id + XChaCha20-Poly1305). |
| Local storage | At-rest encryption of message data | **Open** | SQLite is not encrypted by default; users get coverage from OS-level FDE. Documented gap in [`threat-model.md`](threat-model.md) §I6. |
| OS integration | Secure URL-scheme handling | **Met** | `rekindle://` deep links validated before action; no shell-out to invite payloads. |
| OS integration | Single-instance enforcement | **Met** | Tauri `single-instance` plugin registered first. |
| OS integration | Native notification handling | **Met** | Tauri `notification` plugin; no remote-content fetches. |
| IPC boundary | Authenticated IPC (between desktop processes) | **Met** | Daemon track: Noise IK over Unix socket / named pipe + UCred binding. [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md). |
| Update mechanism | Signed updates | **Open** | Pre-1.0; signing infrastructure documented in [`../contributor/release-process.md`](../contributor/release-process.md). |

---

## OWASP SCVS

Software Component Verification Standard. Three levels (L1, L2, L3).

| Control | Status | Where |
|---------|--------|-------|
| Component inventory (SBOM) | **Met** | CycloneDX SBOMs generated on tag push by `.github/workflows/sbom.yml`; published with each release. |
| Component-origin verification | **Met** | `deny.toml` `[sources]` restricts to crates.io + explicitly-named git remotes. `cargo-vet` config in [`/supply-chain/`](../../supply-chain/). |
| Vulnerability scanning | **Met** | `cargo-audit` (RustSec) + `cargo-deny check advisories` in `audit.yml`; `pnpm audit` for the JS side. |
| License compliance | **Met** | `deny.toml` `[licenses]` allowlist + `dependency-review.yml` PR check. |
| Pinned versions | **Met** | `Cargo.lock`, `pnpm-lock.yaml`, `flake.lock` all committed. |
| Audit-trail / provenance | **Partial** | CI runs are public; reproducible-build verification still open. [`reproducible-builds.md`](reproducible-builds.md). |
| VEX statements | **Open** | When the first SBOM-affecting CVE is triaged, we'll emit VEX alongside the SBOM. Process in [`supply-chain-policy.md`](supply-chain-policy.md). |

**Target level:** L2.

---

## OWASP Top 10:2025

Most categories are web-app-centric. The two that map cleanly onto a
P2P desktop are listed.

| Category | Status | Where |
|----------|--------|-------|
| A02 — Cryptographic Failures | **Met** | Five-layer encryption stack ([`overview.md`](overview.md)); approved primitives ([`crypto-primitives.md`](crypto-primitives.md)); reader-validates governance ([`../architecture/communities.md`](../architecture/communities.md)). |
| A03 — Software Supply Chain Failures | **Met** | `cargo-audit`, `cargo-deny`, `cargo-vet`, `dependabot`, SBOM, KEV monitoring. [`supply-chain-policy.md`](supply-chain-policy.md). |
| A06 — Vulnerable & Outdated Components | **Met** | Same as A03 plus weekly Dependabot bumps. |
| A09 — Logging & Monitoring Failures | **Partial** | Structured logging is in place; central log aggregation is N/A (no servers). [`incident-response.md`](incident-response.md). |
| A10 — Mishandling of Exceptional Conditions | **Partial** | `unwrap()` and `expect()` reduction is open work tracked in [`../roadmap.md`](../roadmap.md). |
| A01, A04, A05, A07, A08 | **N/A** | Web-centric — broken access control, IDOR, security misconfig of HTTP servers, XSS, deserialization in the web context. |

---

## NIST SSDF v1.1 (SP 800-218)

The high-value subset for an OSS project: PW.2/.3/.4/.6 and RV.1/.2.
Other practices apply when targeting federal procurement.

| Practice | Status | Where |
|----------|--------|-------|
| PO.1 — Define security requirements | **Met** | [`threat-model.md`](threat-model.md), [`crypto-primitives.md`](crypto-primitives.md), [`privacy-properties.md`](privacy-properties.md). |
| PO.5 — Implement and maintain secure environments for software development | **Met** | Konductor Nix flake provides a reproducible dev environment. CI in `.github/workflows/`. |
| PS.1 — Protect all forms of code from unauthorized access and tampering | **Partial** | Branch protection + signed commits/tags are documented as prerequisites in [`supply-chain-policy.md`](supply-chain-policy.md); GitHub-org configuration is the user's responsibility. |
| PS.2 — Provide a mechanism for verifying software release integrity | **Open** | Pre-1.0; SHA-256 + BLAKE3 checksums planned, PGP signatures from v0.1.0. [`../contributor/release-process.md`](../contributor/release-process.md). |
| PS.3 — Archive and protect each software release | **Met** | GitHub Releases (post first tag); CHANGELOG.md tracks every release. |
| PW.1 — Design software to meet security requirements and mitigate security risks | **Met** | Threat model + ADRs document this. |
| PW.2 — Review the design to verify it meets security requirements | **Met** | ADRs go through PR review. Architecture docs are public. |
| PW.4 — Reuse existing, well-secured software | **Met** | We use Veilid (Apache 2.0), Signal Protocol (libsignal lineage), `iota_stronghold`, `ed25519-dalek`. We do not write our own crypto. |
| PW.5 — Create source code by adhering to secure coding practices | **Met** | Clippy + workspace lints (`deny(warnings)`, `dbg_macro = deny`, `todo = deny`, `undocumented_unsafe_blocks = deny`). [`../contributor/style-guide.md`](../contributor/style-guide.md). |
| PW.6 — Configure the compilation, interpreter, and build processes to improve executable security | **Met** | `RUSTFLAGS = "-D warnings"` in CI; `#![forbid(unsafe_code)]` in pure-logic crates. |
| PW.7 — Review and/or analyze human-readable code | **Met** | PR template requires test results, security review questions, subsystem checklist. [`../../.github/PULL_REQUEST_TEMPLATE.md`](../../.github/PULL_REQUEST_TEMPLATE.md). |
| PW.8 — Test executable code | **Met** | `cargo test --workspace`, property tests for CRDT merge, Playwright E2E + mock IPC, full CI in `.github/workflows/ci.yml`. [`../contributor/testing.md`](../contributor/testing.md). |
| PW.9 — Configure the software to have secure settings by default | **Met** | E2E on, no telemetry, no plaintext fallback, no auto-recovery of compromised sessions. |
| RV.1 — Identify and confirm vulnerabilities on an ongoing basis | **Met** | `cargo-audit` daily + KEV check weekly + Dependabot weekly. |
| RV.2 — Assess, prioritize, and remediate vulnerabilities | **Met** | [`incident-response.md`](incident-response.md) defines P0–P3 categories and SLAs. |
| RV.3 — Analyze vulnerabilities to identify their root causes | **Met** | Post-mortem template in [`incident-response.md`](incident-response.md). |

**Self-attestation:** When the first tag ships, we can complete the
[CISA Secure Software Development Attestation Form](https://www.cisa.gov/secure-software-attestation-form)
referencing this document.

---

## NIST CSF 2.0

The new **Govern** function added in CSF 2.0 maps directly onto our
supply-chain and policy gaps.

| Function | Categories addressed | Status | Where |
|----------|---------------------|--------|-------|
| Govern (GV) | Cybersecurity strategy, policy, supply-chain risk | **Met** | [`supply-chain-policy.md`](supply-chain-policy.md), [`incident-response.md`](incident-response.md), this doc. |
| Identify (ID) | Asset inventory, data classification | **Met** | SBOM + [`threat-model.md`](threat-model.md) §2 (asset list). |
| Identify | Supply-chain risk | **Met** | [`supply-chain-policy.md`](supply-chain-policy.md). |
| Protect (PR) | Identity / authentication / encryption | **Met** | [`overview.md`](overview.md), [`crypto-primitives.md`](crypto-primitives.md). |
| Protect | Access control (IPC bus) | **Met** | Noise IK + UCred binding. [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md). |
| Detect (DE) | Anomaly detection, monitoring | **Partial** | Local logging only — no central aggregation by design (no servers). |
| Respond (RS) | Incident response procedures | **Met** | [`incident-response.md`](incident-response.md). |
| Recover (RC) | Recovery planning | **Partial** | Cross-device sync provides recovery via paired devices. Master-secret rotation is open work. [`../architecture/sync.md`](../architecture/sync.md). |

---

## NIST 800-53 Rev. 5 (selected families)

OSS projects do not formally conform to 800-53; we cherry-pick the
high-value families and document where each is addressed.

| Family | Relevance | Where |
|--------|-----------|-------|
| AC — Access Control | Daemon IPC bus (Noise IK, UCred); per-community permissions (CRDT bitfield). | [`../architecture/daemon-cli.md`](../architecture/daemon-cli.md), [`../architecture/communities.md`](../architecture/communities.md) §9 |
| AU — Audit & Accountability | Structured `tracing` logging; governance entries are signed/durable; post-mortem template captures incident audit. | [`incident-response.md`](incident-response.md) |
| CM — Configuration Management | `Cargo.lock` / `pnpm-lock.yaml` / `flake.lock` pinned; `rust-toolchain.toml`; CI enforces fmt and lint. | repo root |
| CP — Contingency Planning | Cross-device sync (DFLT personal record). | [`../architecture/sync.md`](../architecture/sync.md) |
| IR — Incident Response | Disclosure policy + runbook + post-mortem template. | [`vulnerability-disclosure.md`](vulnerability-disclosure.md), [`incident-response.md`](incident-response.md) |
| RA — Risk Assessment | Documented threat model. | [`threat-model.md`](threat-model.md) |
| SA — System & Services Acquisition | Dependency vetting via `cargo-vet`. | [`../../supply-chain/`](../../supply-chain/) |
| SC — System & Communications Protection | Five-layer encryption stack. | [`overview.md`](overview.md) |
| SI — System & Information Integrity | `cargo-audit`, `cargo-deny`, KEV check, Dependabot. | `.github/workflows/audit.yml`, `kev-check.yml` |
| SR — Supply Chain Risk Management | See NIST 800-161 below. | [`supply-chain-policy.md`](supply-chain-policy.md) |

Out of scope: PE (Physical), PS (Personnel), PM (Program Management
at the org level) — applicable to operating organisations, not OSS
projects.

---

## NIST 800-161 Rev. 1 (SCRM)

| Control area | Status | Where |
|--------------|--------|-------|
| Supplier risk strategy | **Met** | [`supply-chain-policy.md`](supply-chain-policy.md) §"Dependency selection". |
| Component identification (SBOM) | **Met** | `.github/workflows/sbom.yml`. |
| Vulnerability monitoring | **Met** | `audit.yml` daily + `kev-check.yml` weekly + Dependabot. |
| Change management | **Met** | Dependabot grouped PRs + `dependency-review.yml`. |
| Incident response | **Met** | [`incident-response.md`](incident-response.md). |

---

## CISA Secure by Design

The pledge is targeted at commercial vendors; OSS projects adopt the
principles. See [`cisa-secure-by-design.md`](cisa-secure-by-design.md)
for our principle-by-principle alignment.

| Principle | Status |
|-----------|--------|
| 1. Memory-safe languages | **Met** (Rust workspace; pure-logic crates `#![forbid(unsafe_code)]`) |
| 2. Eliminate vulnerability classes via secure defaults | **Met** (no SQL injection surface, no eval, no plaintext fallback) |
| 3. Secure by default | **Met** (E2E on, no telemetry, no plaintext fallback) |
| 4. Secure update mechanism | **Open** (signed releases pending first tag) |
| 5. CVE / CWE transparency | **Met** ([`vulnerability-disclosure.md`](vulnerability-disclosure.md), KEV check) |
| 6. Phishing-resistant MFA default | **N/A** (no passwords; identity is a keypair) |
| 7. Reduce vulnerability classes | **Met** (lint policy + workspace `deny(warnings)`) |

---

## CISA OSS Security Roadmap

Four objectives; we map against the two that apply to OSS publishers
(visibility and ecosystem hardening).

| Objective | Status | Where |
|-----------|--------|-------|
| 2 — Drive visibility | **Met** | Public SBOMs, public CHANGELOG, public security advisories via GitHub. |
| 4 — Harden the OSS ecosystem | **Partial** | 2FA + branch protection are user-applied org settings (documented as prerequisites). Code signing pending v0.1.0. |

---

## Summary

| Standard | Status |
|----------|--------|
| OWASP ASVS 5.0 (L2) | Mostly met; PQC migration is the main open item |
| OWASP DASVS 1.0 | Mostly met; SQLite at-rest encryption + signed updates are open |
| OWASP SCVS (L2) | Met |
| OWASP Top 10:2025 | Met for applicable categories |
| NIST SSDF v1.1 | Met (self-attestable on first tag) |
| NIST CSF 2.0 | Mostly met; Detect/Recover have partial coverage by design |
| NIST 800-53 (selected) | Met for the families that apply to OSS |
| NIST 800-161 Rev. 1 | Met |
| CISA Secure by Design | 6/7 principles met; secure-update is open |
| CISA OSS Roadmap | Met for OSS-publisher objectives |

The remaining gaps — PQC migration, signed releases, reproducible
builds, panic-prone code reduction — are tracked in
[`../roadmap.md`](../roadmap.md) and the dedicated security docs in
this directory.
