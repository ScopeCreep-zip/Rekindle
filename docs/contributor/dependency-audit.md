# Dependency Audit Process

This document is the manual / quarterly counterpart to the automated
checks running in CI. CI catches the high-confidence cases (RustSec
advisory in a dep, license violation, Dependabot bump, KEV match);
the periodic audit catches the cases that need a human eye —
soft-deprecation of an upstream, a maintainer becoming inactive, a
dep we should be replacing for principled reasons.

The cadence is **quarterly**, run by the project maintainer (or
delegated) on the first Monday of the quarter.

## Pre-audit checklist

Before starting, make sure:

- [ ] CI on `main` is green (`cargo test --workspace`, `cargo clippy
  --workspace -- -D warnings`, `cargo fmt --all -- --check`).
- [ ] `cargo audit` and `cargo deny check` from the most recent
  scheduled `audit.yml` run are green (or known issues are tracked).
- [ ] No open issues tagged `security`, `kev`, or `dependency` are
  unresolved.

## Audit steps

### 1. Run the automated checks locally

```sh
# Rust
cargo audit
cargo deny check

# JavaScript
pnpm audit --prod --audit-level high

# cargo-vet (manual audits)
cargo vet check

# Outdated crates
cargo install cargo-outdated --locked
cargo outdated --workspace
```

Compare results against the most recent CI run. Anything new since
last quarter should be triaged in this audit.

### 2. CISA KEV catalog cross-reference

Even though `kev-check.yml` runs weekly, do a manual cross-reference
once a quarter as a defence-in-depth check:

```sh
# Pull the current KEV catalog
curl -fsSL \
  -o /tmp/kev.csv \
  https://www.cisa.gov/sites/default/files/csv/known_exploited_vulnerabilities.csv

# Extract our CVE-format aliases from cargo-audit JSON
cargo audit --json > /tmp/audit.json
jq -r '.vulnerabilities.list[].advisory.aliases[]?' /tmp/audit.json \
  | grep -E '^CVE-[0-9]{4}-[0-9]+$' \
  | sort -u > /tmp/our-cves.txt

# Compare
tail -n +2 /tmp/kev.csv | cut -d, -f1 | sort -u > /tmp/kev-cves.txt
comm -12 /tmp/our-cves.txt /tmp/kev-cves.txt
```

Any output is a P1 issue per
[`../security/incident-response.md`](../security/incident-response.md).

### 3. Outdated dep review

Walk through `cargo outdated --workspace` output. For each dep that's
behind:

- Is the upstream still active? (last commit, open issues, CI
  status).
- Is the upgrade non-breaking? Note major-version bumps for follow-up.
- Does the upgrade introduce new transitive deps? Run `cargo tree
  --duplicates` after the bump to check.

Same exercise for the JS side: `pnpm outdated --recursive`.

Open a tracking issue for each dep that needs a non-mechanical
upgrade, and tag it `dependency`.

### 4. Maintainer-activity check

For each direct dependency that handles cryptographic key material
(see the list in [`../security/crypto-primitives.md`](../security/crypto-primitives.md)):

- Visit the upstream repository.
- Check: last release (within 12 months?), last commit (within 6
  months?), open advisories on RustSec / GitHub Advisory Database,
  any maintainer-departure notices.
- Document any concerns in the audit log (§6 below).

### 5. License audit

```sh
cargo deny check licenses
```

Verify the result matches expectations. New licenses introduced in
the dep graph since last quarter need explicit allowlist entries in
[`../../deny.toml`](../../deny.toml) — or a different dep choice.

### 6. Audit log

Maintain a running log at `/audit-logs/YYYY-QN.md` (not yet
created — first quarter post-1.0 starts the log). Each entry:

```
## YYYY-QN audit — YYYY-MM-DD

Auditor: <handle>

### Findings
- (none) — or per-finding bullet

### Actions taken
- bumped X to vY
- opened issue #N for Z
- documented concern about W

### Notes
Anything noteworthy for the next auditor.
```

The audit log feeds the annual
[`../security/transparency-report-template.md`](../security/transparency-report-template.md).

### 7. Update SBOM

If any dep was bumped, regenerate the SBOM locally and verify it
parses:

```sh
cargo install cargo-cyclonedx --locked
cargo cyclonedx --format json --override-filename rekindle-cargo-sbom
```

Upload the SBOM to the next release; the
[`/.github/workflows/sbom.yml`](../../.github/workflows/sbom.yml)
workflow will do this automatically on tag push.

### 8. Cross-check `deny.toml` against `Cargo.toml`

Make sure the `[sources]` allowlist in `deny.toml` matches the actual
`git = "..."` entries in `Cargo.toml`. If a git source has been
removed (we replaced a git dep with a crates.io release), remove the
allowlist entry. If a git source has been added, add the allowlist
entry — `cargo deny check sources` will fail otherwise.

## Quick reference: when to act outside the quarterly cycle

| Trigger | Action |
|---------|--------|
| `audit.yml` fails on `main` | Out-of-cycle audit immediately. Fix in private branch if RCE / key compromise. |
| `kev-check.yml` opens an issue | P1 incident per [`../security/incident-response.md`](../security/incident-response.md). |
| `dependency-review.yml` blocks a PR | Block the PR until resolved — usually the contributor needs to bump or remove the offending dep. |
| Dependabot opens a security-tagged PR | Review and merge promptly. The grouped-security PR aggregates these. |
| Major upstream-project announcement (Veilid, Signal, Tauri) | Read the announcement, audit our use of the relevant feature, document in the next audit log. |

## Tools install reference

```sh
cargo install cargo-audit --locked
cargo install cargo-deny --locked
cargo install cargo-vet --locked
cargo install cargo-outdated --locked
cargo install cargo-cyclonedx --locked
cargo install cargo-tree     # may already be in cargo
```

## References

- [`../security/supply-chain-policy.md`](../security/supply-chain-policy.md)
- [`../security/incident-response.md`](../security/incident-response.md)
- [`../../deny.toml`](../../deny.toml)
- [`../../supply-chain/`](../../supply-chain/) — cargo-vet config
- [`../../.github/workflows/audit.yml`](../../.github/workflows/audit.yml)
- [`../../.github/workflows/kev-check.yml`](../../.github/workflows/kev-check.yml)
- [`../../.github/workflows/dependency-review.yml`](../../.github/workflows/dependency-review.yml)
- [RustSec Advisory DB](https://rustsec.org/)
- [CISA KEV catalog](https://www.cisa.gov/known-exploited-vulnerabilities-catalog)
