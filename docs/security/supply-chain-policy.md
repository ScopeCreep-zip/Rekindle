# Supply-Chain Risk Policy

This document is Rekindle's commitment to supply-chain risk
management, aligned with **NIST SP 800-161 Rev. 1** (Cybersecurity
Supply Chain Risk Management Practices for Systems and Organizations,
Nov 2024) and **CISA Secure by Design** principles. It complements
[`vulnerability-disclosure.md`](vulnerability-disclosure.md) and
[`incident-response.md`](incident-response.md).

## 1. Principles

1. **Pinned, auditable dependency graph.** Every direct and transient
   dependency is locked to a specific version. Lockfiles are committed.
2. **Provenance over reputation.** Crates come from crates.io plus an
   explicitly-allowlisted set of git remotes. Off-registry sources
   are forbidden.
3. **Vulnerability monitoring is automated, daily.** Humans triage,
   not scan.
4. **CISA KEV gets emergency response.** A transitive dependency CVE
   appearing in the CISA Known Exploited Vulnerabilities catalog is
   a P1 incident.
5. **Reader-validates extends to dependencies.** We don't blindly
   trust upstream maintainers — we require audits, vetting, or both
   for crypto-handling crates.

## 2. Dependency selection

Before adding a new direct dependency, the contributor proposing it
must answer:

- **What problem does this dependency solve that we can't solve in
  ~50 lines of our own code?**
- **Is the maintainer active?** GitHub commits in the last 6 months;
  unanswered issues are not piling up.
- **Is the license MIT-compatible?** Permissive licenses only:
  MIT, Apache-2.0, BSD-2/3, ISC, Zlib, 0BSD, Unicode-DFS-2016,
  Unicode-3.0, CC0-1.0, MPL-2.0, BSL-1.0, Unlicense. The full
  allowlist lives in [`../../deny.toml`](../../deny.toml).
- **Does the crate handle secrets, ciphertext, or signatures?** If
  yes, an additional `crypto-reviewed` `cargo-vet` audit is required
  (see [`/supply-chain/audits.toml`](../../supply-chain/audits.toml))
  before the dependency lands.
- **Has the crate appeared in RustSec advisories or CISA KEV?** If
  yes, evaluate whether the relevant version is unaffected, and
  document the rationale.

Rejected categories:

- **OpenSSL bindings.** Use Rustls / RustCrypto / `ring` instead.
  Enforced by `deny.toml` `[bans]`.
- **Crates with custom crypto.** Anything implementing primitives
  outside the well-vetted set listed in
  [`crypto-primitives.md`](crypto-primitives.md).
- **Crates with no maintenance signals.** Last release > 2 years ago,
  no commits, archived repo.

## 3. Vulnerability monitoring

Three automated systems run continuously:

| System | Cadence | Scope | Workflow |
|--------|---------|-------|----------|
| **Dependabot** | Weekly (Monday 09:00 UTC) | Cargo + npm + GitHub Actions | [`/.github/dependabot.yml`](../../.github/dependabot.yml) |
| **`cargo-audit` + `cargo-deny`** | Daily (04:13 UTC scheduled) + every PR + every push | Rust workspace | [`/.github/workflows/audit.yml`](../../.github/workflows/audit.yml) |
| **Dependency-review (PR-time)** | Every PR | Cargo + npm dep diff | [`/.github/workflows/dependency-review.yml`](../../.github/workflows/dependency-review.yml) |
| **CISA KEV cross-reference** | Weekly (Wed 06:00 UTC) | Cargo workspace | [`/.github/workflows/kev-check.yml`](../../.github/workflows/kev-check.yml) |
| **`pnpm audit`** | Daily (within audit.yml) | npm prod deps, `--audit-level high` | same |

A KEV match auto-opens an issue tagged `security`, `kev`,
`priority-high` and pages the maintainer. See [`incident-response.md`](incident-response.md)
for the response runbook.

## 4. CISA KEV escalation SLA

The CISA Known Exploited Vulnerabilities catalog
(<https://www.cisa.gov/known-exploited-vulnerabilities-catalog>)
is the federal-government list of CVEs confirmed exploited in the
wild. BOD 22-01 requires federal agencies to remediate within 15
days.

We adopt the spirit of BOD 22-01 with adjusted timelines for an OSS
publisher:

| Timeline | Action |
|----------|--------|
| **0–48 hours** | Triage. Confirm the CVE applies to a code path Rekindle reaches. Author a VEX statement (`affected` / `unaffected` / `under_investigation`). |
| **≤ 7 days** | Patch shipped, regardless of release cadence, if the CVE is `affected`. Hotfix process per [`../contributor/release-process.md`](../contributor/release-process.md). |
| **≤ 14 days** | If `unaffected`, the rationale is published in the SBOM's VEX section so downstream consumers know they don't need to act. |

## 5. License compliance

Every dependency's license must match the allowlist in
[`../../deny.toml`](../../deny.toml). Three enforcement points:

- **PR time** — `dependency-review.yml` blocks the PR if a new
  dependency violates the allowlist.
- **Daily audit** — `audit.yml` re-checks the full graph.
- **Pre-release** — the release checklist in
  [`../contributor/release-process.md`](../contributor/release-process.md)
  re-runs `cargo deny check licenses`.

Copyleft licenses (GPL, AGPL, LGPL) are not on the allowlist.
"Custom" or "non-standard" licenses are not on the allowlist.

## 6. Cargo-vet audit policy

The [`/supply-chain/`](../../supply-chain/) directory holds our
`cargo-vet` configuration:

- **Imports** from Mozilla, Bytecode Alliance, Embark Studios,
  Google, ISRG, and Zcash — these projects' audits cover most of the
  Rust ecosystem.
- **Local audits** for crates not covered by the imports, especially
  any crate that handles cryptographic key material.
- **The `crypto-reviewed` criterion** for crates that touch
  cryptography: this requires correct primitive choice, constant-time
  implementations where applicable, `Zeroize-on-drop` for secret
  types, and no panics on attacker-controlled input.

Adding a new dependency that fails `cargo vet` is a PR blocker.

## 7. SBOM generation and publication

Every tagged release ships with two SBOMs:

- **Cargo SBOM** — `rekindle-cargo-sbom.cdx.json`, CycloneDX 1.6+,
  generated by `cargo-cyclonedx`.
- **pnpm SBOM** — `rekindle-pnpm-sbom.cdx.json`, CycloneDX 1.6+,
  generated by `cdxgen`.

Both attach to the GitHub Release. CI workflow:
[`/.github/workflows/sbom.yml`](../../.github/workflows/sbom.yml).

When a CVE is triaged for a transitive dependency, a VEX
(Vulnerability Exploitability eXchange) statement extends the SBOM —
declaring whether Rekindle is `affected`, `unaffected`,
`under_investigation`, or `not_affected_due_to_<reason>`. Until the
first SBOM-affecting CVE arrives we have no VEX statements; the
process is documented here so the response is mechanical when one
does arrive.

## 8. Build provenance

Reproducible-build CI verification is open work tracked in
[`reproducible-builds.md`](reproducible-builds.md). The pieces in
place today:

- Pinned Rust toolchain ([`../../rust-toolchain.toml`](../../rust-toolchain.toml))
- Pinned Cargo lockfile ([`../../Cargo.lock`](../../Cargo.lock))
- Pinned pnpm lockfile ([`../../pnpm-lock.yaml`](../../pnpm-lock.yaml))
- Pinned Nix flake lockfile ([`../../flake.lock`](../../flake.lock))
- Konductor Nix dev shell for hermetic local builds

Pieces still to land:

- `SOURCE_DATE_EPOCH` propagation through Tauri's bundler
- Deterministic timestamp / file ordering in NSIS / DMG / AppImage / `.deb`
- CI verification that two independent builds of the same commit
  produce byte-identical artefacts

## 9. Maintainer prerequisites (org-level)

The following are GitHub organisation-level settings the project
maintainer must apply (and document publicly):

- **2FA enforcement** for every member of the GitHub organisation
  with write access. We recommend hardware passkeys (FIDO2) over TOTP.
- **Branch protection** on `main`: required PR review, required
  status checks (CI, audit, dependency-review), no force-push.
- **Required signed commits** on `main` once the maintainer key is
  published (deferred to v0.1.0 per
  [`../../SECURITY.md`](../../SECURITY.md)).
- **Restricted token scopes** for `crates.io` and `npm` publishing
  (when applicable).
- **Private vulnerability reporting** enabled in GitHub Security
  settings (already enabled — referenced in
  [`../../SECURITY.md`](../../SECURITY.md)).

These settings are checked by the [OpenSSF Scorecard](https://securityscorecards.dev/)
once enabled; the badge will be added to the README at that point.

## 10. Annual review

This policy is reviewed annually as part of the transparency report
([`transparency-report-template.md`](transparency-report-template.md)).
Material changes to NIST 800-161, CISA Secure by Design, or RustSec /
crates.io security infrastructure trigger an out-of-cycle review.

## 11. References

- [NIST SP 800-161 Rev. 1](https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-161r1.pdf)
- [CISA Secure by Design pledge](https://www.cisa.gov/resources-tools/resources/cisa-secure-design-pledge)
- [CISA OSS Security Roadmap](https://www.cisa.gov/sites/default/files/2024-02/CISA-Open-Source-Software-Security-Roadmap-508c.pdf)
- [CISA KEV catalog](https://www.cisa.gov/known-exploited-vulnerabilities-catalog)
- [CycloneDX](https://cyclonedx.org/)
- [`cargo-vet`](https://mozilla.github.io/cargo-vet/)
- [`cargo-deny`](https://embarkstudios.github.io/cargo-deny/)
- [RustSec Advisory Database](https://rustsec.org/)
- [`standards-mapping.md`](standards-mapping.md)
- [`incident-response.md`](incident-response.md)
- [`vulnerability-disclosure.md`](vulnerability-disclosure.md)
