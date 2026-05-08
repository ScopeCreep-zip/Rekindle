# AI-Assisted Contributions

Rekindle accepts AI-assisted contributions. We also believe AI tools
make it easy to accidentally introduce bad code: silenced lints,
fabricated dependencies, mixed-tier business logic, oversized files,
and tests that pass without exercising real behaviour. This document
is the binding policy for using AI assistants when contributing,
plus the practical guidance that keeps your PRs reviewable.

It applies whether you used Claude Code, GitHub Copilot, Cursor,
ChatGPT, Codex, an MCP server-driven agent, or any other generative
or assistive AI tool.

## 1. Attestation requirement

If any part of a commit was authored or substantially shaped by an
AI tool, the commit message **must** include an `Assisted-by:`
trailer naming the tool and its version (the same place
`Co-authored-by:` would go):

```
Subject line summarising the change

Longer explanation if needed.

Assisted-by: claude-opus-4.7 (Claude Code 1.0)
Co-authored-by: Real Person <real@example.com>
```

The attestation requirement is modelled on the
[Linux kernel coding-assistants policy](https://docs.kernel.org/process/coding-assistants.html),
the [Mozilla Firefox AI coding policy](https://firefox-source-docs.mozilla.org/contributing/ai-coding.html),
the [OpenInfra Foundation policy](https://openinfra.org/legal/ai-policy/),
and the [Drupal AI usage disclosure](https://www.drupal.org/about/governance/coding-standards/ai).

Why we require attestation rather than detection: **detection is not
reliable**. Researchers cannot consistently distinguish AI-assisted
code from human-written code. What works is honest disclosure plus
auditable lineage.

### What counts as AI-assisted

| Pattern | Trailer required? |
|---------|-------------------|
| AI wrote / drafted the diff | Yes |
| You wrote the diff but used AI to refactor it | Yes |
| AI wrote tests that you committed | Yes |
| You used AI to write the commit message but the diff is yours | No (the diff is what matters) |
| You asked AI to explain unrelated code; never integrated output | No |
| AI auto-completion for trivial syntax (e.g., `};`) | No |
| AI rewrote a comment for clarity | Borderline — disclose if unsure |

If you are unsure, **disclose**.

### Trailer format

The format is one line, the same as `Co-authored-by:`:

```
Assisted-by: <tool-name>:<version> [<harness>] [<flags>]
```

Examples:

```
Assisted-by: claude-opus-4.7 (Claude Code 1.0)
Assisted-by: github-copilot 1.241.0
Assisted-by: cursor 0.50.0 (claude-sonnet-4)
Assisted-by: gpt-5o-codex 2026-04-15
```

If multiple tools assisted, list them all (one trailer per line).

### Enforcement

The CI workflow `.github/workflows/ai-attestation.yml` flags PRs
where the diff contains common AI tells (placeholder comments,
fabricated package names, suspicious test patterns) but no
`Assisted-by:` trailer. **Failure is a warning, not a block** — the
maintainer reviews. False negatives (you used AI but no obvious
tells) are still a policy violation; please disclose.

## 2. Architectural invariants AI tools commonly break

AI tools optimise for "make the code work" not for "respect this
project's tier hierarchy." The following invariants are protected
by automated CI gates **and** by review. Read them before submitting
an AI-assisted PR.

### Frontend tier (TypeScript / SolidJS)

```
src/components/  ← presentation only
src/stores/       ← reactive signals + IPC calls; no business logic
src/handlers/     ← event subscriptions; dispatch to stores
src/ipc/          ← typed IPC wrappers; the Tauri boundary
src/styles/       ← global Tailwind theme tokens; semantic class names
```

Forbidden patterns that AI tools love to suggest:

| Pattern | Why forbidden | Where to look instead |
|---------|---------------|-----------------------|
| `import { invoke } from "@tauri-apps/api"` in a component | Direct Tauri import bypasses the typed wrapper layer | Use `src/ipc/commands.ts` |
| Business logic in `*.tsx` (validation, encryption, parsing) | Frontend is presentation only | Add a Tauri command in Rust |
| `class="flex bg-xfire-bg-panel p-2 ..."` inline | Project policy: global styles only | Add a semantic class in `src/styles/xfire-theme.css` |
| `localStorage.getItem(...)` direct access | Bypasses Stronghold + tauri-store integrity | Use `@tauri-apps/plugin-store` via `src/ipc/store.ts` |
| `crypto.subtle.*` calls | Frontend crypto breaks the Tier-2 sole-crypto-boundary | Add a Tauri command (`rekindle-secrets`) |
| `console.log(secret)` / logging private keys | Devtools / observability sink | Never |
| `<div innerHTML={x} />` for any peer-controlled content | DOM XSS | Read [`../security/frontend-rendering.md`](../security/frontend-rendering.md) |

The dependency-cruiser config at the repo root
(`.dependency-cruiser.cjs`) enforces these as **forbidden imports**.
The Biome `noRestrictedImports` rule adds further coverage. Semgrep
rules in `.semgrep.yml` cover the literal-pattern cases.

### Backend tier (Rust workspace)

```
Tier 1 — types          (zero deps)
Tier 2 — secrets         (sole crypto boundary)
Tier 3 — codec, records  (DHT, signed envelopes)
Tier 4 — route           (private routes)
Tier 5 — gossip          (mesh primitives)
Tier 6 — governance      (pure CRDT — no I/O, no async)
Tier 7 — features        (dm, calls, files, video, link-preview)
Cross-cutting — protocol, crypto, voice, sync, transport, node, cli
```

Forbidden patterns AI tools commonly suggest:

| Pattern | Why forbidden | Mitigation |
|---------|---------------|------------|
| `use ed25519_dalek::*` outside `rekindle-secrets` | Tier 2 sole crypto boundary | `cargo deny check bans` (rollout: see [§5](#5-pending-enforcement-being-rolled-out)) |
| `use veilid_core::*` outside `rekindle-transport` (daemon track) or `rekindle-protocol` (desktop track) | Veilid integration is centralised | `cargo deny check bans` |
| `tokio::spawn` in `rekindle-governance` | Tier 6 is pure logic — no async | Code review |
| `std::fs::*` in `rekindle-governance` | Tier 6 is pure logic — no I/O | Code review |
| `#[allow(clippy::too_many_lines)]` to silence the size threshold | Lazy escape hatch | Semgrep rule `rekindle-no-bare-allow` |
| `#[allow(dead_code)]` on a "useful later" helper | Wire it up or delete it; never park dead code | `dead_code = "deny"` (workspace) |
| `unwrap()` / `expect()` on `Result` in error-handling crates | Panic surface | Cleanup sweep tracked in [`../roadmap.md`](../roadmap.md) |
| `panic!()` / `todo!()` / `unimplemented!()` in shipped code | Lazy escape hatches | Workspace lint denies all three |
| Adding a dependency the AI fabricated | Slopsquatting risk | `cargo deny`, manual verification |

### Reading the rules

The full architectural rules with cross-references to enforcement
mechanisms live at
[`architecture-rules.md`](architecture-rules.md). Read it once before
submitting an architectural change.

## 3. Slopsquatting and fabricated dependencies

AI assistants frequently invent plausible-but-fake package names
(`crypto-validator`, `auth-helper-pro`, etc.). 58 % of these
fabrications are reproducible across runs, so attackers can pre-
register the names and wait — **slopsquatting**.

### Before adding any new dependency

1. **Verify the package exists** by visiting its registry page
   directly (don't trust the AI-generated `Cargo.toml` line):
   - Rust: `https://crates.io/crates/<name>`
   - npm: `https://www.npmjs.com/package/<name>`
2. **Check publication history.** A real package has multiple
   versions across multiple months. A slopsquat has one version
   from one week ago.
3. **Check maintainer.** A real package has a recognisable
   maintainer with a real GitHub profile.
4. **Check downloads.** Genuine packages have download counts
   proportional to their utility; slopsquats often have suspicious
   patterns.
5. **Run `cargo deny check`** locally before committing — it will
   catch any package that doesn't satisfy our `[sources]`
   allowlist or `[advisories]` checks.
6. **Run `cargo audit`** — RustSec advisory database.

### What CI does

The lint workflow runs `cargo audit`, `cargo deny`, and the
weekly KEV cross-reference. Together they catch published
advisories, license violations, and fabricated git remotes.
They do **not** catch a perfectly-named slopsquat that hasn't been
flagged yet — the manual verification above is the only defence.

## 4. AI-generated tests — anti-patterns to avoid

AI tools like generating tests because they look productive.
Many such tests pass without exercising real behaviour. Watch for:

```ts
// ❌ "I assert nothing meaningful"
expect(result).toBeDefined();
expect(result).not.toBeNull();
expect(typeof result).toBe("object");

// ❌ "I assert what the function literally returns" (tautology)
const result = double(5);
expect(result).toBe(double(5));

// ❌ "I test the mock, not the system"
mockedDeps.foo.mockReturnValue("hi");
expect(systemUnderTest()).toBe("hi");
// (Of course it does — you mocked the dep to return "hi".)
```

Good tests:

```ts
// ✓ Tests behaviour against a fixed expectation
expect(double(5)).toBe(10);
expect(parseDeepLink("rekindle://invite/AAAA#BBBB")).toEqual({
  blob: "AAAA",
  key: "BBBB",
});
```

Mutation testing (e.g., `cargo mutants` for Rust, Stryker for JS)
catches the tautology pattern by mutating the production code and
seeing if any test fails. We don't run it in CI today; please apply
manually before submitting test-heavy PRs.

For E2E security tests, see
[`../../e2e/security/README.md`](../../e2e/security/README.md).

## 5. Pending enforcement being rolled out

A few automated gates are in **soft-launch** mode. They warn today
and will harden over time:

| Gate | Status | When it hardens |
|------|--------|-----------------|
| Semgrep `rekindle-no-bare-allow` (no `#[allow]` without `reason = "…"`) | Active on **new** code via `--baseline-commit` | When the ~35 existing `#[allow]`s have been retrofitted |
| `cargo deny` crypto wrapper rules (`ed25519-dalek` etc. only via `rekindle-secrets`) | Documented in `deny.toml`; not yet enforced | When the 6 crates importing crypto directly are refactored to consume via `rekindle-secrets` |
| `cargo deny` veilid wrapper rule (`veilid-core` only via `rekindle-transport` / `rekindle-protocol`) | Documented; not yet enforced | When the daemon-track migration completes |
| File-size gate (frontend ≤ 500 lines, Rust ≤ 1500 lines) | Warn-only | When the existing oversized files are split |
| Inline-Tailwind class detection | Warn-only | When the 1 128 inline classes are migrated to semantic classes |
| `clippy::unwrap_used` / `expect_used` set to `deny` | `allow` (existing 660+140 instances) | When the panic-reduction sweep lands |

If your PR introduces a **new** violation in any of these,
expect to fix it. The gates are deliberately set to fail-closed
for new code while letting the existing technical debt land in a
dedicated sweep.

## 6. Recommended AI workflow

We are not prescriptive about which AI tool to use, but a few
patterns produce better PRs:

- **Read the architecture docs first** ([`architecture-rules.md`](architecture-rules.md),
  [`../security/threat-model.md`](../security/threat-model.md),
  [`../architecture/communities.md`](../architecture/communities.md)).
  Most "AI made a wrong choice" failures trace to the AI not having
  read these.
- **Prefer local-first AI tools** (Claude Code's local mode,
  any tool with explicit local-only flag). Cloud-resident tools
  send your codebase to a vendor; for a privacy-conscious project,
  that's a real consideration.
- **Make the AI explain its choices.** If it can't articulate why
  it picked a particular dependency, that's a slopsquatting tell.
- **Ask for the negative case.** "What did you consider and reject?"
  catches AI agents that confidently propose the wrong primitive.
- **Run the linters locally before pushing.** `lefthook run pre-commit --all`
  catches every problem CI would catch, faster.
- **Read the diff yourself.** Even if AI wrote every line, you are
  responsible for it. The review is what `Assisted-by:` attests.

## 7. References

- [Linux kernel — Coding-assistants policy](https://docs.kernel.org/process/coding-assistants.html)
- [Mozilla Firefox — AI coding](https://firefox-source-docs.mozilla.org/contributing/ai-coding.html)
- [Drupal — AI usage policy](https://www.drupal.org/about/governance/coding-standards/ai)
- [OpenInfra Foundation — AI policy](https://openinfra.org/legal/ai-policy/)
- [OpenSSF — Security-focused guide for AI code-assistant instructions](https://best.openssf.org/Security-Focused-Guide-for-AI-Code-Assistant-Instructions)
- [NIST AI Risk Management Framework](https://www.nist.gov/itl/ai-risk-management-framework)
- [NIST AI 600-1 — Generative AI profile](https://nvlpubs.nist.gov/nistpubs/ai/NIST.AI.600-1.pdf)
- [NIST SP 800-218A — SSDF for generative-AI-related software](https://csrc.nist.gov/pubs/sp/800/218/a/final)
- [CISA + UK NCSC — Secure AI system development](https://www.cisa.gov/news-events/alerts/2023/11/26/cisa-and-uk-ncsc-unveil-joint-guidelines-secure-ai-system-development)
- [OWASP Top 10 for LLM Applications 2025](https://owasp.org/www-project-top-10-for-large-language-model-applications/)
- [Socket.dev — Slopsquatting research](https://socket.dev/blog/slopsquatting-how-ai-hallucinations-are-fueling-a-new-class-of-supply-chain-attacks)
- [`architecture-rules.md`](architecture-rules.md) — the binding tier hierarchy + how each rule is enforced
- [`testing.md`](testing.md) — what good tests look like
- [`style-guide.md`](style-guide.md) — Rust + TS + Tailwind conventions
- [`../security/threat-model.md`](../security/threat-model.md) — adversary model
- [`../security/supply-chain-policy.md`](../security/supply-chain-policy.md) — dependency policy
