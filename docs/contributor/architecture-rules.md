# Architecture Rules — The Binding Hierarchy

This is the binding rule set for Rekindle's tier hierarchy: which
crate may import which, which directory may render what, which
permission may invoke which command. **Every rule has an automated
gate enforcing it.** The gate's status (active / warn-only /
pending) is in the rightmost column.

If you contribute code (with or without an AI assistant), read this
document once and refer back to the gate column when you're not
sure whether something is allowed.

Cross-references:
- [`ai-assisted-contributions.md`](ai-assisted-contributions.md) —
  what AI tools commonly get wrong
- [`style-guide.md`](style-guide.md) — code conventions
- [`testing.md`](testing.md) — test strategy
- [`linting.md`](linting.md) — every linter / formatter
- [`../security/threat-model.md`](../security/threat-model.md) —
  adversary model
- [`../security/standards-mapping.md`](../security/standards-mapping.md) —
  ASVS / DASVS / SCVS / SSDF mapping

---

## 1. Backend tier hierarchy (Rust workspace)

Lower-numbered tiers know nothing about higher tiers. **Lower tiers
never depend on higher tiers — including transitively.** The lowest
tiers are pure logic: zero I/O, zero async, zero side effects.

```
┌──────────────────────────────────────────────────────────────────┐
│  Tier 7   self-contained features                                 │
│           rekindle-dm, rekindle-calls, rekindle-files,            │
│           rekindle-link-preview, rekindle-video                   │
├──────────────────────────────────────────────────────────────────┤
│  Tier 6   pure CRDT governance — zero I/O, zero async             │
│           rekindle-governance                                     │
├──────────────────────────────────────────────────────────────────┤
│  Tier 5   gossip mesh primitives                                  │
│           rekindle-gossip                                         │
├──────────────────────────────────────────────────────────────────┤
│  Tier 4   private route lifecycle, peer cache                     │
│           rekindle-route                                          │
├──────────────────────────────────────────────────────────────────┤
│  Tier 3   wire format + DHT record lifecycle                      │
│           rekindle-codec, rekindle-records                        │
├──────────────────────────────────────────────────────────────────┤
│  Tier 2   sole crypto boundary — Zeroize / ZeroizeOnDrop          │
│           rekindle-secrets                                        │
├──────────────────────────────────────────────────────────────────┤
│  Tier 1   shared IDs, enums, error taxonomy                       │
│           rekindle-types                                          │
└──────────────────────────────────────────────────────────────────┘

Cross-cutting (consumed by whichever frontend needs them):
   rekindle-protocol, rekindle-crypto, rekindle-voice,
   rekindle-game-detect, rekindle-sync, rekindle-utils,
   rekindle-e2e-server

Daemon / CLI track:
   rekindle-transport (sole Veilid boundary on this track),
   rekindle-node, rekindle-cli
```

### Backend rules

| # | Rule | Gate | Status |
|---|------|------|--------|
| B1 | Only `rekindle-secrets` may import `ed25519-dalek`, `x25519-dalek`, `aes-gcm`, `chacha20poly1305`, `hkdf` | `cargo xtask check-boundaries` (warn) + `cargo deny [bans]` (pending — blocked by cleanup sweep) | Warn-only, see [§5 in ai-assisted-contributions.md](ai-assisted-contributions.md#5-pending-enforcement-being-rolled-out) |
| B2 | Only `rekindle-transport` (daemon track) and `rekindle-protocol` (desktop track) may import `veilid-core` | `cargo xtask check-boundaries` (warn) + `cargo deny [bans]` (pending) | Warn-only |
| B3 | Pure-logic crates carry `#![forbid(unsafe_code)]` | Per-crate `lib.rs` directive | Active for `rekindle-cli`, `rekindle-node`, `rekindle-transport`, `rekindle-calls`, `rekindle-secrets`. Pending: `rekindle-types`, `rekindle-codec`, `rekindle-records`, `rekindle-gossip`, `rekindle-governance`, `rekindle-route`, `rekindle-files`, `rekindle-link-preview`, `rekindle-video`, `rekindle-dm` |
| B4 | `rekindle-governance` has no I/O, no async, no `tokio::*`, no `std::fs`, no `std::net` | Code review + crate `Cargo.toml` excludes runtime deps | Active |
| B5 | Every `#[allow(...)]` (and `#![allow(...)]`) includes `reason = "…"` | Semgrep `rekindle-no-bare-allow` (active on new code via PR diff) + `cargo xtask check-allow-reasons` | Active for new code; existing 35 sites tracked for retrofit (`cargo xtask retrofit-allow-reasons`) |
| B6 | No `dbg!()`, `todo!()`, `unimplemented!()` in shipped code | Workspace lint `dbg_macro = "deny"`, `todo = "deny"`, `unimplemented = "deny"` | Active |
| B7 | No `dead_code` | Workspace lint `dead_code = "deny"` | Active |
| B8 | No `#[allow(dead_code)]` to keep "useful later" helpers | B5 + B7 combined; review-time gate too | Active |
| B9 | `parking_lot::Mutex` / `RwLock` over `std::sync::Mutex` / `RwLock` | `clippy.toml` `disallowed-methods` | Active |
| B10 | `rekindle-utils::time::now_*` over `std::time::SystemTime::now` | `clippy.toml` `disallowed-methods` | Active |
| B11 | Cap'n Proto generated modules go at the consuming crate's root (`pub mod foo_capnp { include!(…); }`) | Project convention | Review |
| B12 | New dependency must be verified on its registry page (slopsquatting defence) | `cargo audit` + `cargo deny [sources]` + Semgrep `rekindle-no-suspicious-extern-crate` | Active |
| B13 | DB schema is a single file (`src-tauri/migrations/001_init.sql`); bump `SCHEMA_VERSION` in `db.rs` on change | Project convention | Review |
| B14 | No legacy compatibility shims (project is pre-release) | Project convention | Review |

### Backend file-size soft cap

| Surface | Soft threshold | Hard cap (warns louder) |
|---------|----------------|-------------------------|
| Any single Rust file | **1 500 lines** | 3 000 lines |

Five files currently exceed 1 800 lines (the largest is `auth.rs` at
2 487 lines); they are tracked for splitting under
[`../roadmap.md`](../roadmap.md). New oversized files added in a PR
will surface in the file-size CI job.

---

## 2. Frontend tier hierarchy (TypeScript / SolidJS)

```
┌─────────────────────────────────────────────────────────────────┐
│  src/windows/        composes per-window UI from components     │
├─────────────────────────────────────────────────────────────────┤
│  src/components/     presentation only — no business logic       │
├─────────────────────────────────────────────────────────────────┤
│  src/handlers/       channel listener registration; dispatch     │
│                       to stores at app start; never per-component │
├─────────────────────────────────────────────────────────────────┤
│  src/stores/         reactive signals + IPC calls; no logic       │
├─────────────────────────────────────────────────────────────────┤
│  src/ipc/            typed invoke / listen wrappers (leaf)        │
│  src/utils/          format / time / colour helpers (leaf)        │
│  src/styles/         global Tailwind theme + semantic classes     │
│  src/icons.ts        icon enum (leaf)                              │
└─────────────────────────────────────────────────────────────────┘
```

### Frontend rules

| # | Rule | Gate | Status |
|---|------|------|--------|
| F1 | `src/components/**` may only import from `src/ipc/`, `src/stores/`, `src/utils/`, other `src/components/`, `src/icons.ts` | `dependency-cruiser.cjs` `components-no-handlers` rule | Active |
| F2 | `src/handlers/**` may only import from `src/stores/`, `src/ipc/`, `src/utils/` — never `src/components/` or `src/windows/` | `dependency-cruiser.cjs` `handlers-no-presentation` | Active |
| F3 | `src/stores/**` may only import from `src/ipc/`, `src/utils/`, types — never `src/components/` or `src/windows/` | `dependency-cruiser.cjs` `stores-no-presentation` | Active |
| F4 | `src/ipc/**` is a leaf — no imports from `src/stores/`, `src/handlers/`, `src/components/`, `src/windows/` | `dependency-cruiser.cjs` `ipc-is-leaf` | Active |
| F5 | No direct `@tauri-apps/api` import outside `src/ipc/` | `dependency-cruiser.cjs` `no-tauri-api-outside-ipc` + Biome `noRestrictedImports` | Active |
| F6 | No direct `@tauri-apps/plugin-*` import outside `src/ipc/` (except `plugin-opener` for safe external links) | `dependency-cruiser.cjs` `no-tauri-plugin-outside-ipc` | Active |
| F7 | No `localStorage` / `sessionStorage` access from components / stores | `dependency-cruiser.cjs` `no-direct-storage` | Active |
| F8 | No raw `fetch()` from components / stores / handlers | Semgrep + dep-cruiser `no-raw-fetch` | Active (warn) |
| F9 | No `crypto.subtle.*` calls anywhere in `src/**` | Semgrep `rekindle-no-frontend-crypto-primitives` | Active |
| F10 | No `innerHTML` outside the audited Stronghold-QR site | Semgrep `rekindle-no-inner-html` (active; `nosemgrep` directive on the audited line) | Active |
| F11 | All Tauri IPC via the typed wrappers in `src/ipc/commands.ts` | Semgrep `rekindle-no-raw-tauri-invoke` | Active |
| F12 | Tailwind utilities live in `src/styles/`; components compose semantic class names | Project convention; bulk inline-class detection planned | Warn-only (1 128 existing matches tracked) |
| F13 | No `console.log` of secret-bearing fields | Biome `noConsole` (warns) + Semgrep `rekindle-no-secret-in-log` | Active |
| F14 | Window roots are `display: flex; flex-direction: column;` with one `flex: 1` child | Project convention; tested via Playwright visual regression | Review |
| F15 | New `innerHTML` exceptions require a `// SAFETY (XSS):` block + audit table entry in [`../security/frontend-rendering.md`](../security/frontend-rendering.md) | Code review + Semgrep | Active |
| F16 | Peer-rendered content (markdown bodies, link previews, custom emoji names, etc.) goes through DOMPurify | Code review + Playwright XSS suite (`e2e/security/xss.spec.ts`) | Active for shipped paths; gate doc at [`../security/frontend-rendering.md`](../security/frontend-rendering.md) |

### Frontend file-size soft cap

| Surface | Soft threshold | Hard cap |
|---------|----------------|----------|
| Any single `.ts` or `.tsx` file | **500 lines** | 1 000 lines |

Three files currently exceed 500 lines and one
(`src/handlers/community.handlers.ts` at 2 756 lines) far exceeds the
hard cap; they are tracked for splitting. The file-size CI job
warns on existing files and will be promoted to error once the
sweep is complete.

---

## 3. Tauri capabilities (ACL)

The `src-tauri/capabilities/default.json` file controls which Rust
commands the WebView can call. See
[`../security/threat-model.md` §5b W4](../security/threat-model.md)
for the threat-model framing.

| # | Rule | Gate | Status |
|---|------|------|--------|
| T1 | Per-window allow-list (e.g., `chat-*` cannot inherit a `community-*` permission) | `windows: [...]` array in `capabilities/default.json` | Active |
| T2 | Plugin `*:default` bundles will be replaced with explicit allow-lists | Pending IPC-call audit | Warn-only — see threat-model §5b W4 |
| T3 | New permission grants require a `description` update in the same file | Code review (the JSON `description` field is the rationale) | Active |
| T4 | Capabilities file changes require security review on the PR | `CODEOWNERS` (when expanded) | Pending — see [§4](#4-codeowners-and-required-review) below |

---

## 4. CODEOWNERS and required review

The current `.github/CODEOWNERS` assigns the project owner to
everything. The intended mature state is per-tier ownership:

| Path | Reviewer(s) — recommendation |
|------|------------------------------|
| `crates/rekindle-secrets/**` | crypto reviewer + maintainer |
| `crates/rekindle-governance/**` | CRDT reviewer + maintainer |
| `crates/rekindle-transport/**`, `crates/rekindle-protocol/**` | network/Veilid reviewer |
| `src-tauri/capabilities/**` | security reviewer |
| `src-tauri/tauri.conf.json` | security reviewer |
| `docs/security/**` | security reviewer |
| `.github/workflows/**` | maintainer (workflow security audited via zizmor) |
| `Cargo.toml`, `deny.toml`, `.dependency-cruiser.cjs`, `biome.json`, `clippy.toml`, `rustfmt.toml` | maintainer |

For now (single maintainer), all of these route to the project
owner. As maintainers are added, the file will expand.

---

## 5. Pull-request guardrails

| # | Guardrail | Gate | Status |
|---|-----------|------|--------|
| P1 | CI green required before review | GitHub branch protection | Active (org-level setting) |
| P2 | At least one human reviewer | `CODEOWNERS` + branch protection | Active |
| P3 | PR template completed | `.github/PULL_REQUEST_TEMPLATE.md` | Active |
| P4 | Size label applied | `.github/workflows/pr-size.yml` | Active |
| P5 | AI attestation hint on PRs with AI tells but no `Assisted-by:` trailer | `.github/workflows/ai-attestation.yml` | Active (warn-only) |
| P6 | Dep-review on every PR | `.github/workflows/dependency-review.yml` | Active |
| P7 | Lint workflow (Biome / Semgrep / dep-cruiser / file-size / Stylelint / TOML / Markdown / lychee / shellcheck / Nix / typos / gitleaks / Cap'n Proto / Semgrep / actionlint / zizmor) | `.github/workflows/lint.yml` | Active |
| P8 | Audit workflow (cargo-audit / cargo-deny / pnpm audit) | `.github/workflows/audit.yml` daily + per-PR | Active |
| P9 | KEV catalog cross-reference (Cargo) | `.github/workflows/kev-check.yml` weekly | Active |
| P10 | WebView CVE check | `.github/workflows/webview-cve-check.yml` weekly | Active |

---

## 6. How to extend the rules

When you encounter a new architectural invariant that should be
enforced, follow this checklist:

1. **Document it in this file** under the appropriate section. Add
   it to the rules table with a row for the rule, the gate, and the
   status (active / warn-only / pending).
2. **Pick the cheapest gate that catches it.** In rough order of
   cheapness:
   - Workspace lint level (`Cargo.toml` `[workspace.lints.*]`).
   - Clippy `disallowed-methods` / `disallowed-types`.
   - Biome `noRestrictedImports`.
   - Semgrep custom rule.
   - Dependency-cruiser `forbidden` rule.
   - cargo-deny `[bans]` / `[sources]`.
   - `xtask` custom check.
   - CI script (`.github/scripts/`).
3. **Land the gate alongside the rule documentation.** A rule
   without a gate is wishful thinking.
4. **Set the status realistically.** If existing code violates the
   rule, mark it warn-only or pending and link to the cleanup-sweep
   tracking issue. Don't break the build to enforce a rule that
   needs a refactor first.

---

## 7. Glossary of gates

| Gate | What it does | Where to add a rule |
|------|--------------|---------------------|
| **Workspace `[workspace.lints.*]`** | rustc / clippy lints applied to every workspace crate | `Cargo.toml` |
| **`clippy.toml`** | configurable clippy thresholds + disallowed-methods/types | `clippy.toml` |
| **`deny.toml` `[bans]`** | crate / version / wrapper bans | `deny.toml` |
| **`deny.toml` `[sources]`** | registry / git remote allowlist | `deny.toml` |
| **`deny.toml` `[licenses]`** | SPDX license allowlist | `deny.toml` |
| **`biome.json` linter rules** | TS/JS/JSX/JSON style + correctness | `biome.json` |
| **`biome.json` `noRestrictedImports`** | per-package import bans | `biome.json` |
| **`.stylelintrc.json`** | CSS lint, including the Tailwind 4 `@theme` allowlist | `.stylelintrc.json` |
| **`.semgrep.yml` custom rules** | Pattern-based SAST in any language | `.semgrep.yml` |
| **`.dependency-cruiser.cjs` `forbidden`** | TS module-graph rules | `.dependency-cruiser.cjs` |
| **`.lefthook.yml`** | Local pre-commit / pre-push orchestration | `.lefthook.yml` |
| **`xtask`** | Custom Rust-side checks (boundaries, file-sizes, allow-reasons retrofit) | `xtask/src/main.rs` |
| **`.github/workflows/lint.yml`** | CI orchestration of every linter | `.github/workflows/lint.yml` |
| **`.github/workflows/audit.yml`** | CI orchestration of every dep-audit | `.github/workflows/audit.yml` |

When you're not sure which gate is right, ask in the PR — adding
the wrong gate is reversible; not adding a gate at all means the
rule will silently drift.

---

## References

- [`ai-assisted-contributions.md`](ai-assisted-contributions.md)
- [`linting.md`](linting.md)
- [`testing.md`](testing.md)
- [`style-guide.md`](style-guide.md)
- [`release-process.md`](release-process.md)
- [`../security/threat-model.md`](../security/threat-model.md)
- [`../security/standards-mapping.md`](../security/standards-mapping.md)
- [`../security/supply-chain-policy.md`](../security/supply-chain-policy.md)
- [`../architecture/crates.md`](../architecture/crates.md)
- [`../architecture/frontend.md`](../architecture/frontend.md)
- [`../architecture/communities.md`](../architecture/communities.md)
