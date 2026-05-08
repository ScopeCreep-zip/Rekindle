# CISA Secure by Design — Principle Alignment

[CISA's Secure by Design](https://www.cisa.gov/resources-tools/resources/cisa-secure-design-pledge)
initiative (announced 2023, formal pledge May 2024) asks technology
vendors to build products with security as the default rather than an
add-on. The pledge itself targets commercial vendors, not OSS
publishers, but its principles transfer cleanly. This document maps
Rekindle to each principle.

CISA frames Secure by Design around three core ideas — *take ownership
of customer security outcomes*, *embrace radical transparency*, *build
secure-by-design from the top* — and seven concrete commitments. We
walk through the seven below.

## 1. Memory-safe languages

> Vendors should make demonstrable progress eliminating memory-safety
> vulnerabilities by transitioning to memory-safe programming languages.

**Status: met.**

Rekindle is a Rust workspace plus a TypeScript/SolidJS frontend.

- All protocol logic, cryptography, voice pipeline, file delivery,
  governance merge, and IPC bus implementation are Rust.
- The frontend is TypeScript with strict typing — also memory-safe
  by virtue of the V8 / WebKit runtime.
- Pure-logic crates carry `#![forbid(unsafe_code)]`:
  `rekindle-types`, `rekindle-secrets`, `rekindle-codec`,
  `rekindle-records`, `rekindle-route`, `rekindle-gossip`,
  `rekindle-governance`, `rekindle-dm`, `rekindle-calls`,
  `rekindle-files`, `rekindle-link-preview`, `rekindle-video`,
  `rekindle-cli`, `rekindle-node`, `rekindle-transport`.
- Cross-cutting crates that need `unsafe` (FFI, OS-specific syscalls)
  must annotate every block with `// SAFETY:` per the workspace lint
  `undocumented_unsafe_blocks = "deny"`.

The workspace lint policy (in `Cargo.toml`) enforces this with
`deny(warnings)`, `deny(dead_code)`, `clippy::all = deny`, and
`undocumented_unsafe_blocks = "deny"`.

## 2. Eliminate entire vulnerability classes via secure defaults

> Vendors should publish a roadmap demonstrating how they will
> eliminate entire classes of vulnerabilities at scale via secure
> design choices.

**Status: met.**

Rekindle's architecture eliminates several whole vulnerability
classes by construction:

| Vulnerability class | How it is eliminated |
|---------------------|----------------------|
| SQL injection | Rusqlite parameterised queries throughout. No string-concatenated SQL. |
| Cross-site scripting | No web server. Tauri frontend is local; SolidJS escapes by default. No `dangerouslySetInnerHTML` equivalents. |
| Server-side request forgery | No HTTP server. `rekindle-link-preview` is the only OpenGraph fetcher; runs sandboxed with hard limits (5 s timeout, 256 KB body cap, plain text/html only, max 5 redirects). |
| Deserialisation of attacker-controlled data | Cap'n Proto schemas with explicit field types; generated code is bound-checked. |
| Plaintext fallback | Refused by policy. Every code path is end-to-end encrypted; there is no "couldn't establish session, sending plaintext" branch. |
| Secret-in-stack-trace | Custom `Debug` impls on secret-bearing types redact sensitive fields. |
| Memory-safety bugs | See principle 1. |

## 3. Make products secure by default

> Default settings should be the most secure ones available.
> Security features should not require additional configuration or
> cost.

**Status: met.**

- End-to-end encryption is on by default for every conversation.
  There is no "encrypted mode" toggle because there is no
  unencrypted mode.
- No telemetry. No "diagnostic ping" to scopecreep.zip or anywhere
  else.
- No analytics, no engagement tracking, no behavioural metrics.
- Per-community pseudonyms are derived automatically; the user does
  not need to opt in to unlinkability.
- The Stronghold vault is encrypted at rest by default with the
  user's passphrase; there is no "skip the passphrase" mode.
- Voice is `SafetySelection::Unsafe` for low latency by default for
  voice channels (acceptable because participants are mutually
  known); chat uses safety routes by default for sender anonymity.
- The Tauri capabilities file declares only the APIs the frontend
  actually uses — denied-by-default, not allowed-by-default.

There is no premium tier, no "pro" feature gating security, and no
upsell path. All security features are available to all users at
zero cost.

## 4. Secure update mechanisms

> Vendors should provide free and easy security updates with simple,
> low-friction installation.

**Status: open (pre-1.0).**

The pieces in place:

- A documented release process in
  [`../contributor/release-process.md`](../contributor/release-process.md)
  including signing intent, SBOM attachment, and post-release
  verification.
- Tauri's `notification` plugin for in-app update prompts (post-1.0).

The pieces still to land:

- **Code-signing certificates** (Authenticode for Windows, Apple
  Developer ID + notarisation for macOS, PGP for Linux artefacts).
  Until these exist, downloads are unsigned and SmartScreen / Apple
  Gatekeeper warn the user.
- **Tauri updater wiring.** The current `check_for_updates` command
  is a stub. Production updater behind it is post-1.0.
- **Reproducible builds.** Documented in
  [`reproducible-builds.md`](reproducible-builds.md).

These are blocked on infrastructure (signing certs cost money and
require legal entity), not on architecture.

## 5. CVE / CWE transparency

> Vendors should publish vulnerability information in standard
> formats (CVE, CWE) and demonstrate transparency in their disclosure
> practices.

**Status: met.**

- [`../../SECURITY.md`](../../SECURITY.md) — disclosure policy with
  scope, supported versions, response SLAs, coordinated-disclosure
  norm.
- [`vulnerability-disclosure.md`](vulnerability-disclosure.md) —
  formalised 90-day coordinated disclosure framework.
- [`incident-response.md`](incident-response.md) — internal runbook
  including post-mortem template.
- GitHub Security Advisories enabled and used as the primary GHSA
  channel.
- CVE assignment via GitHub's CNA for the project, or direct MITRE
  request when needed.
- CWE classification expected on every advisory (per CWE list at
  <https://cwe.mitre.org/>).
- Annual transparency report following
  [`transparency-report-template.md`](transparency-report-template.md).

## 6. Phishing-resistant MFA as default

> Authentication should be phishing-resistant by default. Hardware
> tokens, passkeys, or equivalent should be the baseline.

**Status: not applicable (no passwords).**

Rekindle's identity model is an Ed25519 keypair stored in the
Stronghold vault. There are no usernames, no passwords, no
phishable login flow. The Stronghold passphrase is the only thing
the user types — and an attacker who phishes the passphrase still
needs physical access to the device to use it (the vault is
local-only).

If the user pairs a second device, the master secret transfers via
an `app_call` handshake gated by a one-time code + salt + record
key, with the code expiring in 5 minutes. This is closer in spirit
to passkey transfer than to password-based MFA.

There is therefore no phishing-resistant MFA to enable; the entire
authentication surface is non-phishable by construction.

## 7. Reduce vulnerability classes

> Vendors should publish a blog or report annually demonstrating
> they are tracking root causes of vulnerabilities and reducing
> entire classes of bugs over time.

**Status: met (process established; first report after first tag).**

- Workspace lint policy denies `dbg!`, `todo!`, `unimplemented!`,
  `dead_code`, `warnings`, plus `undocumented_unsafe_blocks` —
  closing common bug-introduction vectors at compile time.
- Property-based testing of the CRDT merge engine
  ([`../contributor/testing.md`](../contributor/testing.md)).
- Code-coverage tracking is open work tracked in
  [`../roadmap.md`](../roadmap.md).
- The annual transparency report
  ([`transparency-report-template.md`](transparency-report-template.md))
  is the public artefact for principle 7. It tracks CVEs received /
  triaged / patched, dependency update statistics, and SSDF
  practice maturity year over year.

## Why we don't sign the pledge

The CISA Secure by Design pledge is a commitment from **commercial
software vendors**. Rekindle is an OSS project with no commercial
vendor structure: we don't have customers, we don't sell support,
we don't have an enterprise product line. The pledge form asks for
information that doesn't apply (e.g., progress reports on customer
security outcomes).

We adopt the **principles** without signing the pledge. This
document is the public commitment.

## References

- [CISA Secure by Design](https://www.cisa.gov/securebydesign)
- [CISA Secure by Design Pledge](https://www.cisa.gov/resources-tools/resources/cisa-secure-design-pledge)
- [CISA Secure by Design whitepaper](https://www.cisa.gov/sites/default/files/2023-10/SecureByDesign_1025_508c.pdf)
- [`standards-mapping.md`](standards-mapping.md)
- [`supply-chain-policy.md`](supply-chain-policy.md)
- [`vulnerability-disclosure.md`](vulnerability-disclosure.md)
