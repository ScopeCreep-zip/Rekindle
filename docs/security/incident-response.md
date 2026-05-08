# Incident Response

This document is Rekindle's incident-response runbook. It covers the
classification, communication, and remediation steps for security
incidents — vulnerability reports, CVE matches in dependencies,
maintainer-key compromise, and accidental secret exposure.

It complements [`vulnerability-disclosure.md`](vulnerability-disclosure.md)
(the public-facing disclosure policy), [`supply-chain-policy.md`](supply-chain-policy.md)
(the dependency-risk side), and [`../../SECURITY.md`](../../SECURITY.md)
(the contact channel).

## 1. Severity classification

| Severity | Definition | Response time |
|----------|-----------|---------------|
| **P0** — Critical | Active exploitation, key compromise, RCE, or auth bypass. Users actively at risk now. | Acknowledge in **2 hours**, hotfix in **48 hours**. |
| **P1** — High | High-impact CVE in critical dependency (CISA KEV match, exploitable RCE, plaintext leak). No active exploitation. | Acknowledge in **24 hours**, patch in **7 days**. |
| **P2** — Medium | Logic flaw, info-disclosure with limited scope, denial-of-service against a single user. | Acknowledge in **3 days**, patch in **30 days**. |
| **P3** — Low / Informational | Hardening opportunity, defence-in-depth gap, doc inaccuracy on security posture, minor information leak. | Acknowledge in **7 days**, patch in **90 days**. |

Severity is assigned by the maintainer on triage; reporters can
suggest a severity but the final call is the project's. Disagreements
are resolved by erring toward the higher severity.

## 2. Communication channels

| Channel | When |
|---------|------|
| **GitHub Security Advisories** | Always. Every confirmed P0/P1 gets an advisory, drafted private and published when the fix ships. |
| **`security@scopecreep.zip`** | Initial reporter contact, ongoing coordination with security researchers, PGP-encrypted exchange when sensitive details apply. |
| **Public release notes / `CHANGELOG.md`** | Every fixed advisory gets a one-line entry referencing the GHSA ID. |
| **GitHub Issues (private)** | Internal triage and tracking before the advisory is published. |
| **GitHub Issues (public)** | Auto-opened by `kev-check.yml` when a transitive dep CVE matches CISA KEV. Tagged `security`, `kev`, `priority-high`. |

We do **not** maintain a separate mailing list, Slack, Discord, or
Matrix channel for security incidents — concentration of channels
reduces miscommunication risk for a small project.

## 3. Triage workflow

Every report follows the same five-step flow:

```
┌──────────────────────────────────────────────────────────────┐
│  1. Receive (GHSA private report / email / KEV-check issue)   │
├──────────────────────────────────────────────────────────────┤
│  2. Acknowledge — within the SLA above                        │
├──────────────────────────────────────────────────────────────┤
│  3. Reproduce — confirm the issue applies to current main     │
├──────────────────────────────────────────────────────────────┤
│  4. Classify — assign severity, draft GHSA                    │
├──────────────────────────────────────────────────────────────┤
│  5. Remediate — fix in a private branch, ship, publish        │
└──────────────────────────────────────────────────────────────┘
```

### Step 1 — Receive

Sources: GitHub private-vulnerability advisory, security email
inbox, automated KEV-check issue, observed exploitation, downstream
distributor flag, dependency advisory (RustSec / GitHub Advisory
Database).

The receiving maintainer checks the `security` label first thing
each working day to avoid missing reports.

### Step 2 — Acknowledge

A short acknowledgement to the reporter, no technical content, within
the SLA in §1. Set the GHSA into "draft" status if not already.

### Step 3 — Reproduce

Verify the report applies to current `main`. If the report is against
an older release, identify whether the regression is still present.
Build a minimal reproducer if the report doesn't include one.

### Step 4 — Classify

Assign P0–P3. Author the GHSA description with:

- The vulnerability description (one paragraph).
- Affected versions.
- Patched version (filled in at publish time).
- Severity (CVSS 3.1 score plus our P0–P3 assignment).
- Workaround if any.
- Credit line for the reporter (with their consent).

If the issue is a CVE in a transitive dependency, author a VEX
statement (`affected` / `unaffected` / `not_affected_due_to_X`) and
prepare it for the next SBOM update.

### Step 5 — Remediate

- For P0: fix in a **private** branch off `main` so the public
  diff doesn't tip off attackers. Ship a hotfix release. Publish
  the GHSA at the same moment as the release tag.
- For P1: fix in a private branch, but a public branch is acceptable
  if the fix can be made non-obvious. Ship within 7 days.
- For P2/P3: fix in the open like any other issue. Land in the next
  scheduled release.

After ship: ensure the GHSA is linked from the release notes and
`CHANGELOG.md`. If the fix changes the wire format or schema, also
update the relevant docs in
[`../architecture/`](../architecture/) and
[`../protocol/`](../protocol/).

## 4. Specific runbooks

### 4.1 Maintainer signing-key compromise

**Signal:** A release is published that nobody in the maintainer
team authored. A signed commit appears on `main` from a key the
maintainer did not approve. The OS keyring entry holding the daemon's
long-term identity is missing or mutated.

**Response:**

1. Treat as **P0**. Acknowledge internally; no public statement yet.
2. Revoke the compromised key in the GitHub org (rotate personal
   access tokens, sign-out all sessions, revoke the SSH/PGP key).
3. Publish a CVE-quality GHSA explaining the scope: which releases
   are potentially compromised, what could have been substituted in
   them, what users should do.
4. Ship a new release signed with a fresh key. Publish the new
   key fingerprint via every public channel (project README, social
   posts, GitHub release notes).
5. Encourage downstream consumers to audit the prior releases for
   any unexplained changes.
6. Post-mortem within 14 days; publish findings in the next
   transparency report.

### 4.2 Dependency compromise (RustSec / KEV match)

**Signal:** `audit.yml` fails. `kev-check.yml` opens an auto-issue.
A CVE is announced in a dep we ship.

**Response:**

1. Triage. Read the advisory and our code. Does the vulnerable code
   path get reached? Author a VEX statement (`affected` /
   `unaffected` / `under_investigation`).
2. If `affected`:
   - P0/P1 if the CVE is actively exploited or in CISA KEV.
   - P1 if the CVE is high-severity (CVSS ≥ 7) but not yet exploited.
   - P2 otherwise.
3. Ship the patched-dependency release per the SLA in §1.
4. If `unaffected`, publish the rationale in the next SBOM /
   transparency report so downstream consumers can stop tracking it.

### 4.3 Accidental secret-in-commit

**Signal:** A maintainer pushes a commit containing a real
production secret (PGP key, API token, signed SBOM key, OS keyring
content, identity vault contents).

**Response:**

1. Treat as **P0** even if it was a test secret — assume real.
2. Rotate the secret immediately at its source (revoke the API
   token, rotate the signing key, etc.).
3. Force-rewrite the affected branch only if the project hasn't been
   force-pushed by external collaborators. **GitHub caches don't
   forget**: assume the secret is permanently disclosed once pushed,
   even if the commit is rewritten.
4. Publish a GHSA describing what was exposed and what users should
   do.
5. Audit `git log -p` for the affected branch range to confirm no
   other secrets leaked.

### 4.4 User-reported privacy regression

**Signal:** A user reports that Rekindle leaked information that the
threat model said was protected (cross-community pseudonym
correlation, presence visible to non-friends, etc.).

**Response:**

1. P1 by default. Privacy-property regressions for vulnerable users
   are critical regardless of CVSS score.
2. Reproduce. If reproducible:
   - Patch the regression.
   - Update [`threat-model.md`](threat-model.md) and
     [`privacy-properties.md`](privacy-properties.md) if the issue
     reveals a previously-undocumented gap.
   - Author a GHSA with the reporter's consent.
3. If the regression existed in a published release, identify
   whether users need to take action (rotate keys, re-create
   identities) and document it in the GHSA.

### 4.5 Wire-format / schema bug discovered post-release

**Signal:** A protocol bug is found that lets a peer corrupt another
peer's local state, evade governance permissions, or read content
they shouldn't.

**Response:**

1. P0 if the bug allows reading content the peer shouldn't have
   access to. P1 if it allows unauthorized writes that honest peers
   would reject anyway.
2. Patch in a private branch.
3. Coordinate with anyone running their own daemon
   (`rekindle-node`) — the daemon track means there could be
   long-running infrastructure outside the maintainer's view.
4. Bump `SCHEMA_VERSION` if the database schema needs to change. Per
   the project's pre-1.0 status, this triggers a wipe-and-rebuild on
   next launch — see [`../architecture/data-layer.md`](../architecture/data-layer.md).
5. Publish the GHSA, release the patch, and add a regression test
   per [`../contributor/testing.md`](../contributor/testing.md).

## 5. Post-mortem template

After every P0 or P1 incident, a post-mortem is written within 14 days
and published in the next transparency report
([`transparency-report-template.md`](transparency-report-template.md)).

```
# Post-mortem: <one-line description>
Date: YYYY-MM-DD
Severity: P0 / P1
GHSA: GHSA-xxxx-xxxx-xxxx
CVE: CVE-YYYY-NNNN (if assigned)

## Summary
A 2–3 sentence summary suitable for the transparency report.

## Timeline
- YYYY-MM-DD HH:MM UTC — Reported / detected
- YYYY-MM-DD HH:MM UTC — Acknowledged
- YYYY-MM-DD HH:MM UTC — Reproduced
- YYYY-MM-DD HH:MM UTC — Patch landed
- YYYY-MM-DD HH:MM UTC — Release published
- YYYY-MM-DD HH:MM UTC — GHSA published

## Root cause
What was the underlying mistake or assumption that led to this issue?

## Detection
How was the issue found? Could we have found it earlier?

## Impact
Who was affected? For how long? What data, if any, was exposed?

## Remediation
What changed in the code / docs / processes?

## Lessons learned
What is the project doing differently going forward? Update this
runbook, the threat model, or the lints if needed.
```

## 6. References

- [`vulnerability-disclosure.md`](vulnerability-disclosure.md) — public disclosure policy
- [`supply-chain-policy.md`](supply-chain-policy.md) — dependency-risk side
- [`../../SECURITY.md`](../../SECURITY.md) — disclosure channel
- [`threat-model.md`](threat-model.md) — adversary model
- [`privacy-properties.md`](privacy-properties.md) — privacy invariants
- [GitHub Security Advisories](https://docs.github.com/en/code-security/security-advisories)
- [CVSS 3.1 calculator](https://www.first.org/cvss/calculator/3.1)
- [VEX (Vulnerability Exploitability eXchange)](https://www.cisa.gov/sites/default/files/publications/VEX_Use_Cases_Aprill2022.pdf)
