# Linting and Formatting

Rekindle's code base spans Rust, TypeScript, SolidJS, Tailwind 4 CSS,
TOML, JSON, YAML, Cap'n Proto, SQLite migrations, GitHub Actions
workflows, shell scripts, Nix, and Markdown. Each ecosystem has its
own best-of-breed linter and formatter; this document is the canonical
reference for the full stack as it stands in 2026.

The configs all live at the repo root (or `.github/`):

| Tool | Config file | Covers |
|------|-------------|--------|
| **rustfmt** | [`../../rustfmt.toml`](../../rustfmt.toml) | Rust code formatting |
| **clippy** | [`../../clippy.toml`](../../clippy.toml) + workspace lints in `Cargo.toml` | Rust correctness + style |
| **cargo-deny** | [`../../deny.toml`](../../deny.toml) | Rust dep advisories / licenses / sources / bans |
| **cargo-vet** | [`../../supply-chain/`](../../supply-chain/) | Rust dep audit attestations |
| **Biome v2** | [`../../biome.json`](../../biome.json) | TS/JS/JSX/TSX/JSON formatter + linter (replaces eslint + prettier) |
| **Stylelint** | [`../../.stylelintrc.json`](../../.stylelintrc.json) | CSS / Tailwind 4 |
| **knip** | [`../../knip.config.ts`](../../knip.config.ts) | Unused exports / files / deps in TS |
| **taplo** | [`../../taplo.toml`](../../taplo.toml) | TOML formatter + linter |
| **markdownlint-cli2** | [`../../.markdownlint-cli2.yaml`](../../.markdownlint-cli2.yaml) | Markdown style |
| **lychee** | [`../../lychee.toml`](../../lychee.toml) | Markdown link checker |
| **typos** | [`../../typos.toml`](../../typos.toml) | Source + docs typo checker |
| **sqlfluff** | [`../../.sqlfluff`](../../.sqlfluff) | SQLite migrations |
| **actionlint** | [`../../.actionlint.yaml`](../../.actionlint.yaml) | GitHub Actions workflows |
| **zizmor** | [`../../.zizmor.yml`](../../.zizmor.yml) | GitHub Actions security audit |
| **shellcheck** | (no config — defaults) | `scripts/*.sh` |
| **shfmt** | (no config — `-i 4 -ci -bn`) | shell formatter |
| **nixfmt-rfc-style** | (no config — RFC-48 canonical) | `*.nix` formatter |
| **statix** | (no config — defaults) | Nix anti-pattern lints |
| **deadnix** | (no config — defaults) | Unused Nix bindings |
| **gitleaks** | [`../../.gitleaks.toml`](../../.gitleaks.toml) | Secret scanner |
| **capnp compile** | (built-in) | Cap'n Proto schema validation |
| **lefthook** | [`../../.lefthook.yml`](../../.lefthook.yml) | Local git-hook orchestration |

## How to use

### One-shot local run

The recommended workflow is to install [lefthook](https://lefthook.dev/)
and run all the appropriate linters on every commit and push:

```sh
brew install lefthook            # or: cargo install lefthook --locked
lefthook install                  # activates hooks for this clone
lefthook run pre-commit --all    # runs every linter against the whole repo
lefthook run pre-push            # cargo clippy + cargo test
```

`lefthook install` replaces the symlinks under `.git/hooks/` so that
every subsequent `git commit` and `git push` runs the configured
linters automatically.

### Per-tool ad-hoc

Each linter is also runnable on its own:

```sh
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cargo deny check
cargo vet check

# Frontend (after pending devDependencies are installed)
pnpm exec biome check src/
pnpm exec stylelint 'src/styles/**/*.css'
pnpm exec knip

# Cross-language
taplo format --check
markdownlint-cli2 '**/*.md'
lychee --config lychee.toml '**/*.md'
typos
sqlfluff lint src-tauri/migrations/
actionlint
zizmor .

# Shell
shellcheck scripts/*.sh
shfmt -d -i 4 -ci -bn scripts/

# Nix
nixfmt --check $(find . -name '*.nix' -not -path './target/*')
statix check .
deadnix --fail .

# Secrets + schemas
gitleaks detect --verbose
capnp compile -o /dev/null --src-prefix=schemas schemas/*.capnp
```

### CI

Two workflows orchestrate these in CI:

| Workflow | Covers |
|----------|--------|
| [`../../.github/workflows/ci.yml`](../../.github/workflows/ci.yml) | `cargo fmt`, `cargo clippy`, `cargo test`, pnpm install + Playwright mock-IPC suite, Tauri build smoke per OS |
| [`../../.github/workflows/lint.yml`](../../.github/workflows/lint.yml) | Everything else — frontend (Biome / stylelint / knip), TOML, Markdown + lychee, SQL, workflows (actionlint + zizmor), shell, Nix, typos, gitleaks, Cap'n Proto |
| [`../../.github/workflows/audit.yml`](../../.github/workflows/audit.yml) | RustSec advisories + cargo-deny + pnpm audit (daily + per-PR) |
| [`../../.github/workflows/dependency-review.yml`](../../.github/workflows/dependency-review.yml) | Per-PR dep diff |
| [`../../.github/workflows/codeql.yml`](../../.github/workflows/codeql.yml) | JS/TS CodeQL |
| [`../../.github/workflows/sbom.yml`](../../.github/workflows/sbom.yml) | CycloneDX SBOM on tag push |
| [`../../.github/workflows/kev-check.yml`](../../.github/workflows/kev-check.yml) | Weekly CISA KEV catalog cross-reference |

## Why these tools

### Rust

`rustfmt` and `clippy` are stdlib — no alternatives. For supply-chain
auditing we use `cargo-audit` (RustSec advisory DB), `cargo-deny`
(license + bans + sources policy), and `cargo-vet` (cryptographic
audit attestations). Specific selection rationale lives in
[`../security/supply-chain-policy.md`](../security/supply-chain-policy.md)
and the dedicated agent reports synthesised here.

### Frontend — why Biome over ESLint+Prettier

We chose **Biome v2** over the ESLint v9 flat-config + Prettier 3.x
combo for three reasons:

1. **Speed.** Biome is Rust-based; 25× faster than Prettier and
   10–25× faster than ESLint on a 50k-line project. The Rekindle
   frontend is small (~3k lines) but linting runs many times per
   commit cycle.
2. **One config file.** Biome's `biome.json` covers TS/JS/JSX/TSX,
   JSON, and import organisation. Eliminates config-file sprawl.
3. **SolidJS rules.** Biome 2.x ships SolidJS-specific rules
   (`useSolidForComponent`, `noReactSpecificProps`) so we get
   framework-aware lints without an `eslint-plugin-solid` install.

The trade-off is a smaller plugin ecosystem than ESLint. If we ever
need a bespoke rule that Biome doesn't ship, the migration path is
[oxlint](https://oxc.rs/) (also Rust-based, ESLint-rule-compatible) or
back to ESLint v9.

CSS lives outside Biome's main scope — Tailwind 4's `@theme` syntax
specifically — so **Stylelint** owns `src/styles/**/*.css` with the
config in `.stylelintrc.json`. Biome's CSS formatter is disabled.

### TOML — taplo

`taplo` is the consensus TOML tool: built-in JSON-Schema validation
for known files (`Cargo.toml`, `dependabot.yml`, etc.), deterministic
formatting, fast (Rust). No real competitor for TOML in the OSS
ecosystem.

### Markdown — markdownlint + lychee, not Prettier

We use **markdownlint-cli2** for *style* and **lychee** for *links*.
Prettier is also capable of formatting Markdown but is opinionated in
ways that fight long-form docs (e.g., it rewrites tables in ways
markdownlint disagrees with). Splitting style from links keeps the
two checks composable.

### Workflows — actionlint + zizmor

`actionlint` catches syntax, expression, and action-input errors that
yamllint misses — and it pipes inline `run:` blocks through
shellcheck automatically. `zizmor` (Trail of Bits, 2024) is a
*security* audit on top: script-injection via untrusted
`${{ github.event.* }}` interpolation, missing `permissions:` blocks,
pwn-request patterns, hard-coded credentials. Use both.

### Nix — RFC-48 nixfmt + statix + deadnix

The 2026-canonical Nix formatter is **nixfmt-rfc-style** (RFC-48),
not the older `alejandra` or `nixpkgs-fmt`. **statix** lints
anti-patterns; **deadnix** finds unused let-bindings and function
arguments.

### Shell — shellcheck + shfmt

Every shell script is a potential remote-code-execution surface.
`shellcheck` is the Bash linter; `shfmt` is the canonical formatter
(`-i 4 -ci -bn`). actionlint already runs shellcheck on inline
`run:` blocks; this layer covers `scripts/*.sh` standalone.

### Secrets — gitleaks

`gitleaks` is fast (Go-based regex engine), runs on the working tree
or on the entire git history. We use it both pre-commit (via
lefthook) and on every CI run (full-history scan).
[trufflehog](https://github.com/trufflesecurity/trufflehog) is the
heavier alternative — it *verifies* leaked credentials are still
active rather than just matching patterns. We don't use it today;
gitleaks alone catches the high-signal cases.

### Typos — typos

`typos` is the Rust-built fast typo checker. Project-specific
vocabulary (Veilid, chiral, porter, schwarzschild, etc.) is allowlisted
in `typos.toml`. cspell and codespell are slower alternatives.

### SQL — sqlfluff

The only viable cross-dialect SQL linter. Locked to the SQLite
dialect because that's what `001_init.sql` targets via rusqlite.

## Pending devDependencies

Three frontend tools are configured but not yet listed in
`package.json` because adding them needs a `pnpm install` round-trip
that's better coordinated with other parallel work:

```json
{
  "devDependencies": {
    "@biomejs/biome": "^2.4.0",
    "stylelint": "^17.0.0",
    "stylelint-config-standard": "^36.0.0",
    "knip": "^5.0.0",
    "markdownlint-cli2": "^0.14.0"
  },
  "scripts": {
    "lint": "biome check src/",
    "lint:styles": "stylelint 'src/styles/**/*.css'",
    "lint:unused": "knip",
    "lint:md": "markdownlint-cli2 '**/*.md'",
    "format": "biome format src/ --write"
  }
}
```

The CI job in `.github/workflows/lint.yml` is gated on the binary
presence (`hashFiles('node_modules/.bin/biome') != ''`) — once
`pnpm install` runs with these deps the job becomes active without
any further config.

## Pending Rust workspace lint upgrades

Three additions to `[workspace.lints.clippy]` in `Cargo.toml` are
recommended but **not yet applied**, because they would each surface
a large number of warnings that need a code-remediation sweep. They
should land alongside the same sweep that addresses the ~660
`unwrap()` and ~140 `expect()` calls noted in the prior security
audit:

```toml
[workspace.lints.clippy]
# Existing lints stay as-is.

# Promote from "allow" to "warn" once the cleanup sweep lands:
unwrap_used = "warn"             # forces handling of Option/Result
expect_used = "warn"             # ditto, with a custom message
indexing_slicing = "warn"        # forces .get() over [..]
arithmetic_side_effects = "warn" # forces overflow handling
as_conversions = "warn"          # forces `try_into()` over `as`
panic = "warn"                   # surfaces explicit panic sites

# Promote restriction lints relevant to crypto code:
cast_possible_truncation = "warn"
cast_possible_wrap = "warn"
cast_lossless = "warn"
```

These are documented here so the next contributor doing a
panic-reduction pass has the configuration ready.

## Pending TypeScript strictness upgrades

`tsconfig.json` currently uses baseline `strict: true`. The 2026
recommended additions are listed below; each will likely surface
type errors that need fixing in the existing TS code:

```json
{
  "compilerOptions": {
    "verbatimModuleSyntax": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitOverride": true,
    "noPropertyAccessFromIndexSignature": true
  }
}
```

Apply once the parallel TS work in flight has settled.

## Editor integration

Most modern editors auto-detect the configs above. Quick references:

| Editor | Recommended extension(s) |
|--------|--------------------------|
| **VS Code** | `biomejs.biome`, `stylelint.vscode-stylelint`, `tamasfe.even-better-toml`, `DavidAnson.vscode-markdownlint`, `bradlc.vscode-tailwindcss`, `mads-hartmann.bash-ide-vscode`, `tekumara.typos-vscode` |
| **JetBrains** | Built-in TOML / Markdown / Bash / TypeScript; install Biome plugin; install Stylelint plugin |
| **Vim / Neovim** | `null-ls.nvim` / `none-ls.nvim` / `conform.nvim` with sources for biome, stylelint, taplo, shfmt, nixfmt, statix, markdownlint |
| **Emacs** | `apheleia` for format-on-save; `flymake` integrations for biome, statix, shellcheck |
| **Helix** | Native LSP integration; configure each language server in `languages.toml` |

The Konductor Nix dev shell (`nix develop .#frontend`) ships with
the binaries pre-installed.

## Adding a new language or tool

1. Add the linter's config file at the repo root (or `.github/`).
2. Add a `commands:` block under the appropriate stage in
   [`../../.lefthook.yml`](../../.lefthook.yml) with a `glob:` filter.
3. Add a job to [`../../.github/workflows/lint.yml`](../../.github/workflows/lint.yml).
4. Add a row to the table at the top of this document and a brief
   "Why this tool" paragraph if the choice isn't obvious.
5. Update the editor-integration table if there's a notable
   extension.

## References

- [Biome](https://biomejs.dev/)
- [Stylelint](https://stylelint.io/)
- [knip](https://knip.dev/)
- [Taplo](https://taplo.tamasfe.dev/)
- [markdownlint-cli2](https://github.com/DavidAnson/markdownlint-cli2)
- [lychee](https://github.com/lycheeverse/lychee)
- [typos](https://github.com/crate-ci/typos)
- [sqlfluff](https://docs.sqlfluff.com/)
- [actionlint](https://github.com/rhysd/actionlint)
- [zizmor](https://docs.zizmor.sh/)
- [shellcheck](https://www.shellcheck.net/)
- [shfmt](https://github.com/mvdan/sh)
- [nixfmt RFC-48](https://github.com/NixOS/nixfmt)
- [statix](https://github.com/oppiliappan/statix)
- [deadnix](https://github.com/nix-community/deadnix)
- [gitleaks](https://github.com/gitleaks/gitleaks)
- [lefthook](https://lefthook.dev/)
- [`../security/supply-chain-policy.md`](../security/supply-chain-policy.md)
- [`../security/standards-mapping.md`](../security/standards-mapping.md)
