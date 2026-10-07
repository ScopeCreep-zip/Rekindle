# October 2026 Audit — Harvest Progress, Security Posture, CI/Infra Reality

Companion to `docs/research/alignment-audit.md` / `docs/plans/alignment-completion.md`
(Sept 2026). That round found and fixed real defects in sync responses, relay-route
leaks, DHT watch lifecycle, and three governance bugs, and converged most of the
event vocabulary. This round asks the same question the project keeps asking itself —
"does the doc match the tree?" — across three threads: the services-drain harvest
("breaking the codebase down to be clean"), the security/supply-chain posture
("and secure"), and the CI/infra state ("cleaning broken infra along the way").

Four parallel audits, each required to read actual code and cite file:line, not
trust doc claims. Findings below are organized by thread, each ranked by severity.
Nothing in this document has been fixed yet — it is the inventory, not the plan.

---

## Thread 1 — Security posture

### High

- **`webview-cve-check.yml` can never find anything — permanent false assurance.**
  It queries GitHub's Advisory API with `affects=WebKit`, `affects=WebView2`,
  `affects=WebKitGTK`, `affects=libsoup`, `affects=libwebkit2gtk`,
  `affects=tauri-apps/tauri`. Verified live against the real API: every one of
  those terms returns `[]`, because `affects` only matches a registered ecosystem
  package name (confirmed by testing `affects=lodash`, `affects=openssl` — both
  return real advisories; `affects=tauri` — the actual crates.io name — currently
  also returns `[]`, i.e. the one near-correct term is additionally wrong: it
  uses the repo slug `tauri-apps/tauri`, not the package name `tauri`). This
  workflow exists specifically to cover the WebView-CVE threat named in
  `docs/security/threat-model.md`, and has been silently reporting "no advisories"
  regardless of whether real ones exist. It has also never actually registered/run
  (see Thread 3).

- **No CI job runs `cargo vet`, despite the policy doc stating it's a PR blocker.**
  `docs/security/supply-chain-policy.md` says "Adding a new dependency that fails
  `cargo vet` is a PR blocker." Zero hits for `cargo vet`/`cargo-vet` across
  `.github/workflows/*.yml`. `supply-chain/audits.toml` has zero real recorded
  audits — just the criterion definition and a commented-out example; no
  `imports.lock` exists, so the claimed upstream trust imports (Mozilla, Bytecode
  Alliance, Embark, Google, ISRG, Zcash) have never been fetched/pinned. `cargo vet`
  has apparently never been run against this repo at all.

- **(Thread 3 overlap) `audit.yml` would fail hard, immediately, on first real run** —
  see Thread 3. 82 `cargo deny check` errors (3 banned-crate violations already in
  the tree, 18 workspace crates with no `license` field despite CLAUDE.md's "License:
  MIT"), 13 `cargo audit` vulnerabilities, 1 critical `pnpm audit` finding
  (`GHSA-mv8w-475r-vwqw`, `seroval` via `solid-js`, Promise-resolver type confusion).
  These are real, present-tense findings nobody has triaged, because the gate that
  would have surfaced them has never executed.

### Medium

- **`MlKemSecret`'s `Drop` impl zeroes nothing.** `crates/rekindle-secrets/src/pq_keys.rs:41-43,135-159`.
  The crate's own charter (`lib.rs:1-5`) states "the sole crate that handles raw key
  material... every secret type implements `Zeroize + ZeroizeOnDrop`." `MlKemSecret`
  — the ML-KEM-768 PQ private key feeding PQXDH session establishment — is the one
  exception: it has a manual `impl Drop` whose body is entirely comments. Honestly
  documented (the comment explains `libcrux-ml-kem` 0.0.9 doesn't expose a zeroize
  hook), but a real deviation from the stated guarantee for a project whose threat
  model explicitly covers memory forensics against at-risk users. (Every other
  secret-holding struct spot-checked — `RatchetState`, `Identity`, `SyncKey`/
  `PairingKey`, `MasterSecret`/`SlotSeed`/`MediaEncryptionKey`/`CallKey` — correctly
  derives `Zeroize`/`ZeroizeOnDrop`.)

- **`deny.toml`'s Tier-2 crypto-boundary and Veilid-centralization bans are
  commented out**, explicitly marked "PENDING enforcement" with an honest note that
  6 crates currently violate it. The architectural claim CLAUDE.md states as fact
  ("rekindle-secrets is the sole security boundary") is not CI-enforced — only a
  soft, non-blocking `cargo xtask check-boundaries` warning exists.

- **`rekindle-link-preview` has no SSRF guard.** `fetch_link_preview`
  (`crates/rekindle-link-preview/src/lib.rs:37-70`) fetches an arbitrary
  user-supplied URL (5s timeout, 256KB cap, ≤5 redirects) with no blocklist for
  loopback/link-local/private/cloud-metadata ranges, and redirects aren't
  re-validated against those ranges either. Traced the caller chain: sender-initiated
  only (gated by the `EMBED_LINKS` permission; receivers only render pre-fetched
  metadata, never re-fetch) — not a zero-click cross-peer bug, but any member with
  that permission can point their own client at `169.254.169.254` or an internal
  address, directly or via redirect.

### Low

- Stronghold→vault doc drift: `README.md:26,69,120,266`, `ARCHITECTURE.md:58,75`,
  `SECURITY.md:90`, and `src-tauri/src/lib.rs:85-87` all still describe/list
  `iota_stronghold` as current, including `SECURITY.md:90` listing it as an in-scope
  upstream dependency — the actual migration (ADR 0006, `rekindle-vault`/SQLCipher)
  is complete and verified correct (zero `iota_stronghold`/`iota_crypto` in
  `Cargo.lock`); only these peripheral docs, and CLAUDE.md itself, weren't updated.
  `vulnerability-disclosure.md` explicitly says it and `SECURITY.md` "stay in sync" —
  they currently don't, on this exact point.
- `ARCHITECTURE.md:59` claims "22 Rust crates"; actual count is 37.
- Neither `rekindle-crypto` nor `rekindle-secrets` declares `#![forbid(unsafe_code)]`
  at the crate root (no unsafe code exists today; the guarantee just isn't locked at
  the boundary — only one submodule, `session_cache.rs`, does it locally).
- ADR 0007 ("app.emit() appears ONLY inside event_dispatch.rs") has two small,
  reasonable, documented exceptions (`setup.rs:101` pre-dispatch-loop bootstrap,
  `windows.rs:137` direct window-targeted emit) — wording slightly overstated, not
  a violation.
- CLAUDE.md's ADR table lists only 0001–0005; the real directory has 9 (0006–0009
  exist, added through June 2026).

### Verified clean

All 9 ADRs hold against actual code (no-coordinator governance confirmed, Veilid-only
transport confirmed except expected exceptions, Signal/PQXDH shape confirmed,
Tauri frameless/transparent config confirmed, daemon Unix-socket IPC confirmed).
Every non-test `.unwrap()`/`.expect()`/panic in `rekindle-crypto`/`rekindle-secrets`
is provably safe (fixed-length HKDF outputs, in-bounds array literals, or guarded by
explicit bounds checks — `ratchet.rs`'s `deserialize`/`decrypt_step`, which process
attacker-controlled bytes, were hand-traced and are fully `Result`-based with no
reachable panic). `kev-check.yml`'s CISA feed URL and cross-reference logic are both
correct and live-verified. `sbom.yml` and `dependency-review.yml` are both correct,
real implementations. Incident-response and vulnerability-disclosure docs are
substantive and project-specific, not templates.

---

## Thread 2 — Services-drain harvest ("breaking the codebase down to be clean")

**Verdict: the adapter layer itself is genuinely excellent; the aggregate is
flat-to-growing, not shrinking; and the project's own "extract + unit-test the
decision" pattern is applied inconsistently, with one of the inconsistencies being
the direct cause of a bug fixed two sessions ago.**

- `src-tauri/src/services/` is **42,579 LoC today**. `docs/roadmap.md:238` still
  claims "~37.6K LoC" — the tracked total grew ~13% since that line was written,
  the opposite of "drain." `docs/roadmap.md:243`'s "`AppState` now ~47 fields" is
  also stale — the actual struct has **84 fields**. The `state.rs` split itself
  (Phase 23.B) is clean; the checkbox just isn't re-verified against current state.
- `docs/architecture/services-pattern.md` itself has drifted: it lists AutoMod
  regex matching as "still in `services/community/`, not yet harvested" — it's
  already a 3-function thin facade fully delegating to `rekindle_channel::automod`.
- Every adapter sampled (`governance_adapter/`, `presence_adapter/`,
  `voice_adapter/`, `channel_adapter/`, `mek_adapter.rs`, `video_adapter.rs`) is a
  textbook-clean thin `Deps`-trait implementation — zero protocol decisions in the
  adapter body. Two go further and extract their one real decision into a pure,
  unit-tested free function specifically so it's testable without an `AppHandle`.

**GAP findings** (business logic still misplaced; target crate named):

| file:line | What's wrong | Target |
|---|---|---|
| `friend_runtime/accept.rs:133-138`, `message_service/friend_handlers/session.rs:111-115`, `message_service/friend_handlers/lifecycle.rs:269-274` | The Signal-session idempotency guard (`has_session(...) && is_trusted_identity(...)`) is copy-pasted at 3 separate call sites, each independently tagged "W16.10e" as if it were one fix. **This is the same duplication class that caused the invite-accept session-mismatch bug fixed two sessions ago** — a 4th site (`setup_invite_contact`, in `friend_runtime/invite.rs`) drifted out of sync with the other three precisely because there was no single source of truth to drift *from*. | `rekindle-crypto::signal::session` — one `SignalSessionManager::session_already_established(peer, identity_key) -> bool` method all 4 call sites use. |
| `community/message_notifications.rs:80-176` (`decrypt_message_body`) | Reimplements, inline, the AAD-bound-decrypt-with-legacy-fallback waterfall that `rekindle_channel::receive::decrypt_channel_body_with_legacy_fallback` already implements and unit-tests. | `rekindle-channel` |
| `community/mek_rotation.rs:54-78` (`cache_hit` match in `spawn_mek_request_with_retry`) | Real, security-relevant split-brain/convergence decision left inline and untested — in the *same file*, `should_last_resort_mint` (123-126) is the correctly-extracted, tested version of exactly this kind of decision. | `rekindle-mek-rotation` |
| `community/message_notifications.rs:319-343`, `community/link_previews.rs:117-141`, `community/receiver_limits.rs:113-139`, `veilid/control.rs:290-305`, `veilid/control_sync.rs:29-41` | "decode pseudonym → compute permissions → check capability" boilerplate independently reimplemented at 5 call sites. `state_helpers::my_permissions` exists but only covers the *local* user, not an arbitrary sender. | `rekindle-governance` (new fn taking a pseudonym + capability bit) or a `state_helpers` sibling |
| `idle_service.rs` (~90% of 493 lines) | Pure cross-platform OS-idle FFI (CoreGraphics/`GetLastInputInfo`/`xprintidle`+Wayland) with zero `AppState`/Veilid dependency — same shape already extracted into its own crate for game detection. Lowest severity: consistency, not correctness. | new crate, following `rekindle-game-detect/src/platform/`'s precedent |

**Ambiguous but defensible** (Tauri-specific enough to arguably stay): `community/video_session.rs` (MediaCapabilities aggregation, doc comment argues the case), `veilid/app_message/mod.rs` (explicitly carved out by the pattern doc as the one un-categorizable dispatch loop), `governance_adapter/membership_events.rs::spawn_peer_bootstrap`, `presence_adapter/community_deps.rs::reconcile_voice_roster`.

**Dead code:** zero `#[allow(dead_code)]`, zero `todo!()`/`unimplemented!()`, anywhere — and this is compiler-enforced (`dead-code = "deny"`, `todo = "deny"`, `unimplemented = "deny"` in workspace lints), not just convention.

---

## Thread 3 — CI/infra reality ("cleaning broken infra along the way")

### The headline finding: most of this isn't "red," it's never run at all

`.github/workflows/` has 11 files. `ci.yml`/`lint.yml` were fixed two sessions ago.
Of the other 9 (`ai-attestation.yml`, `audit.yml`, `codeql.yml`,
`dependency-review.yml`, `kev-check.yml`, `pr-size.yml`, `release.yml`, `sbom.yml`,
`webview-cve-check.yml`): **8 have never run on GitHub — not once, ever.**
`gh api repos/.../actions/workflows` lists only 3 registered workflows (CI, Lint,
Release). `git ls-tree -r origin/main -- .github/workflows/` shows only
`release.yml` exists on `main` — the other 8 exist only on feature branches, and
**no PR has ever been opened** from this branch (or any of its ancestors) against
`main` or anything else. Every trigger (`push: main`, `pull_request`, or
`schedule`/`workflow_dispatch`, which GitHub only fires for files present on the
default branch) has simply never been satisfied. This is the root cause tying
Thread 1's security gaps directly to Thread 3: the gates that would have caught
them have never executed, on any commit, ever.

### High

- **`audit.yml` would fail all three of its jobs immediately, reproduced locally:**
  - `cargo-deny check --workspace --all-features`: **82 errors.** 3 already-present
    banned-crate violations (`ed25519 2.2.3`, `ed25519 3.0.0`, `openssl-sys 0.9.116`
    — all explicitly banned in `deny.toml`, with a comment anticipating exactly this
    and asking someone to file an upstream issue, which never happened because the
    check never ran) + 18 workspace crates with no `license` field (contradicting
    CLAUDE.md's "License: MIT," and no `[workspace.package] license` exists for them
    to inherit).
  - `cargo audit --deny warnings`: **13 vulnerabilities, 28 denied warnings.**
  - `pnpm audit --prod --audit-level high`: **1 critical** — `GHSA-mv8w-475r-vwqw`,
    `seroval` (via `solid-js`), Promise-resolver type confusion.
  - None of these three results have been triaged by anyone, because this is the
    first time they've been surfaced at all.
- **`webview-cve-check.yml`** — covered in Thread 1; also never registered.

### Medium

- **`release.yml`'s Node.js 20 actions already crossed their EOL date.** The latest
  real run (tag `v0.0.2-alpha`, success) carries GitHub's own annotation that
  Node20 actions (`actions/checkout@v4`, `actions/setup-node@v4`,
  `pnpm/action-setup@v4`) would be removed from the runner **September 16, 2026**.
  Today is October 3 — 17 days past that date. Verified via web search this has
  actually happened industry-wide (not a hypothetical future notice):
  `actions/checkout` is at v6 now, `pnpm/action-setup` at v6 (v5 added the Node24
  cutover support specifically for this), `actions/dependency-review-action` at
  v5.0.0 (released in May specifically to move off Node20). The next real
  release-tag push is at genuine risk of failing at the runtime level, not just
  getting a deprecation warning.
- **Repo-wide action-pin drift**, same class found in `lint.yml` last session:
  every workflow in this batch pins `actions/checkout@v4` (2 majors behind);
  `codeql.yml` pins `github/codeql-action@v3` (v4 has existed since Oct 2025; v3's
  own deprecation lands ~December 2026 — not broken yet, same clock).

### No bug found (verified, not just "looked fine")

- `kev-check.yml` — live CISA feed URL confirmed working, cross-reference logic sound.
- `ai-attestation.yml` — suspected `local` keyword outside a function under
  `set -euo pipefail`; reproduced in isolation, doesn't actually fail.
- `dependency-review.yml`, `sbom.yml`, `pr-size.yml`, `codeql.yml` — no logic bugs
  found by inspection; share the Node20/pin-drift exposure above; `sbom.yml`'s exact
  `cdxgen`/`cargo-cyclonedx` flags weren't independently executable in the time
  budget, flagged as unverified rather than claimed-broken.

### Phase 4.2 daemon-parity claim (`docs/plans/alignment-completion.md`): unchanged, zero progress

Exact current counts, re-measured: **258** `#[tauri::command]`s, **64**
`IpcRequest` daemon variants, `router.rs::dispatch()` matches exactly those 64 with
no extras — both numbers byte-identical to the plan's September baseline. Spot-checked
27 command names spanning all 14 waves (A–N) plus the "needs a decision" bucket:
every single one is still present exactly once on the Tauri side and has zero
presence anywhere in the daemon. The 41-item "deliberately not ported" and 5-item
"needs a decision" buckets both still check out as accurately categorized. **Nothing
in this 211-command gap has moved since the plan was written.**

---

## Cross-thread read

The three threads aren't independent — they're one story. The services-drain audit
found the Signal-session idempotency guard duplicated across 3 call sites instead
of centralized; that exact duplication class is what let a 4th, uncentralized copy
drift into the bug fixed two sessions ago. The infra audit found that 8 of 11
safety-net workflows have never executed on this branch because it's never been
through a PR; the security audit's "real but never-surfaced" findings (82
cargo-deny errors, 13 cargo-audit vulnerabilities, 1 critical pnpm vulnerability)
are the direct, measurable consequence of that. None of this is about code written
carelessly — the crypto core, the adapter layer, the policy docs are all
unusually careful. It's about verification infrastructure that was built and then
never actually exercised against this branch.

## What this document does not contain

Fixes, prioritized phases, or exact diffs — this is the audit, not the plan, matching
the project's own `alignment-audit.md` → `alignment-completion.md` split. A completion
plan should be written next, phase-ordered by the severities above, before any of
this is implemented.
