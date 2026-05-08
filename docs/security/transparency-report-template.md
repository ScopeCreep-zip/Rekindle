# Transparency Report — Template

Rekindle publishes an annual transparency report covering security
incidents, dependency updates, and standards alignment progress. The
first report will publish at the end of the year following the
project's first tagged release. This file is the template — every
report follows the same structure so trends are legible across years.

The template is also referenced from
[`cisa-secure-by-design.md`](cisa-secure-by-design.md) §7 (annual
vulnerability-class reduction reporting) and
[`supply-chain-policy.md`](supply-chain-policy.md) §10 (annual policy
review).

---

## Annual Transparency Report — YYYY

**Reporting period:** January 1, YYYY — December 31, YYYY
**Published:** YYYY-MM-DD
**Prepared by:** <maintainer-handle>

### 1. Executive summary

A 3–5 sentence summary suitable for press, funders, or institutional
reviewers. Headline numbers: how many CVEs received, how many
patched, average response time, any P0 incidents.

Example: "In YYYY, Rekindle received N security reports (M critical),
patched all confirmed issues within the SLA, shipped X tagged releases,
and brought NEW_PRACTICES into the SSDF self-assessment. No P0
incidents occurred. Median time-to-patch for confirmed CVEs was D
days."

### 2. Vulnerability disclosures

| Severity | Received | Confirmed | Patched | Median time-to-patch |
|----------|----------|-----------|---------|---------------------|
| P0 — Critical | N | N | N | D days |
| P1 — High | N | N | N | D days |
| P2 — Medium | N | N | N | D days |
| P3 — Low | N | N | N | D days |
| **Total** | N | N | N | D days |

Per-incident detail (one row per published GHSA):

| GHSA | Title | Severity | Reported | Patched | Reporter |
|------|-------|----------|----------|---------|----------|
| GHSA-xxxx-xxxx-xxxx | <title> | P? | YYYY-MM-DD | YYYY-MM-DD | <reporter handle or "anonymous"> |

For each, link to the published advisory and to the post-mortem if
one was written.

### 3. Dependency updates

| Ecosystem | Routine bumps (Dependabot) | Security-driven bumps | Manual / breaking | Total |
|-----------|----------------------------|----------------------|-------------------|-------|
| Cargo | N | N | N | N |
| npm / pnpm | N | N | N | N |
| GitHub Actions | N | N | N | N |

CISA KEV catalog matches detected by `kev-check.yml` and how they
were resolved:

| KEV CVE | Affected dep | Status | Resolution |
|---------|--------------|--------|------------|
| CVE-YYYY-NNNN | <crate>@<version> | affected / unaffected | <patch date or VEX rationale> |

### 4. Releases and SBOMs

| Tag | Date | Highlights | SBOM |
|-----|------|-----------|------|
| vX.Y.Z | YYYY-MM-DD | Major points | [SBOM](https://github.com/ScopeCreep-zip/Rekindle/releases/download/vX.Y.Z/rekindle-cargo-sbom.cdx.json) |

### 5. Standards alignment progress

Movement against [`standards-mapping.md`](standards-mapping.md):

| Standard | Year start | Year end | Notes |
|----------|-----------|----------|-------|
| OWASP ASVS 5.0 (target L2) | <status> | <status> | <commentary> |
| OWASP DASVS 1.0 | <status> | <status> | <commentary> |
| OWASP SCVS (target L2) | <status> | <status> | <commentary> |
| NIST SSDF v1.1 | <status> | <status> | <commentary> |
| NIST CSF 2.0 | <status> | <status> | <commentary> |
| NIST 800-161 Rev. 1 | <status> | <status> | <commentary> |
| CISA Secure by Design | <status> | <status> | <commentary> |
| PQC migration | <status> | <status> | <commentary> |

Concrete shipped items:

- (e.g.) "Reproducible builds: SOURCE_DATE_EPOCH propagation
  shipped; Tauri-bundler determinism still pending upstream patch
  TAURI-NNNNN."
- "Code signing: Authenticode certificate procured; first signed
  Windows release shipped with vX.Y.Z."
- "PQC: hybrid X25519+ML-KEM-768 enabled for community channel MEK
  delivery in vX.Y.Z."

### 6. Vulnerability-class reduction

CISA Secure by Design Principle 7 reporting. Track which classes of
vulnerability were eliminated, and which still appear.

| Class | Year start count | Year end count | How |
|-------|-----------------|---------------|-----|
| `unwrap()` panic surface | N | N | systematic replacement with `?` and `expect_or_return` |
| `unsafe` blocks | N | N | added `// SAFETY:` annotations / refactored to safe API |
| Plaintext fallbacks | 0 | 0 | architecturally impossible — refused by design |
| ... | ... | ... | ... |

### 7. Code health metrics

| Metric | Year start | Year end |
|--------|-----------|----------|
| Workspace crates | N | N |
| Public IPC commands | N | N |
| Test count (cargo test --workspace) | N | N |
| `unwrap()` call count | N | N |
| `expect()` call count | N | N |
| `#[allow(...)]` directive count | N | N |
| Dependency count (direct, Cargo) | N | N |
| Dependency count (transitive, Cargo) | N | N |

### 8. Governance and infrastructure

- Maintainers: <number, handles>
- New maintainers onboarded: <number>
- Org-level changes: 2FA enforcement, branch protection, signed-
  commits requirement, etc.
- Infrastructure changes: GitHub Actions runners, signing
  certificates procured, etc.

### 9. Documentation changes (security-relevant)

Major changes to [`/docs/security/`](.) and adjacent:

- (e.g.) "Updated [`threat-model.md`](threat-model.md) §I7 to reflect
  the new daemon-track IPC bus."
- "Published [`pqc-roadmap.md`](pqc-roadmap.md)."
- "Refreshed [`standards-mapping.md`](standards-mapping.md) for
  ASVS 5.0.1 minor updates."

### 10. Funder and audit acknowledgements

Audits performed:
- (e.g.) "Trail of Bits engagement on the daemon IPC bus —
  see /docs/security/audits/2026-trail-of-bits-ipc.pdf."

Funders / sponsors who supported the project this year (with
permission to acknowledge): list.

### 11. Forward-looking commitments

Three to five concrete commitments for the next year. Tracked
against the next transparency report.

- (e.g.) "Ship hybrid X25519+ML-KEM-768 for DM MEK derivation in
  v0.3.0, target Q2."
- "Bring `unwrap()` count below 200 across the workspace by year-end."
- "Publish first independent security audit results."

### 12. Contact

Security reports: see [`../../SECURITY.md`](../../SECURITY.md).

Comments / questions about this transparency report:
`security@scopecreep.zip`.

---

## Notes on writing the report

- **Be concrete.** Real numbers, real dates, real GHSA IDs. No
  marketing language.
- **Include negative results.** If we missed an SLA, say so, and say
  what changed in the runbook.
- **Don't redact unless legally required.** Vulnerability detail
  beyond what GHSA already publishes is fine to publish in full;
  reporters' identities only with their consent.
- **Link, don't copy.** Reference [`standards-mapping.md`](standards-mapping.md),
  [`supply-chain-policy.md`](supply-chain-policy.md), GHSAs, etc.
  rather than duplicating their content.
- **Keep it short.** A transparency report shouldn't be a wall of
  text — readers come back year over year and want to see deltas
  quickly. Aim for 1500–3000 words.

## References

- [`cisa-secure-by-design.md`](cisa-secure-by-design.md)
- [`supply-chain-policy.md`](supply-chain-policy.md)
- [`incident-response.md`](incident-response.md) — post-mortem template feeds §2
- [`standards-mapping.md`](standards-mapping.md)
