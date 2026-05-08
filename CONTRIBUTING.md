# Contributing to Rekindle

Thanks for your interest in contributing. Rekindle is a 1:1 faithful
reimplementation of the classic Xfire gaming chat client, built on Tauri 2,
Rust, SolidJS, and the Veilid peer-to-peer network. This document explains
how to set up a development environment, what we expect from a pull request,
and where to ask for help.

> Rekindle ships to vulnerable communities (privacy-conscious users,
> activists, journalists, marginalized groups). We refuse plaintext
> fallbacks, auto-rehandshake, spoofable display fields, deterministic
> timing leaks, and silent telemetry. Read [`docs/security/threat-model.md`](docs/security/threat-model.md)
> before adding any feature that touches identity, encryption, or transport.

## Getting started

| Step | Command |
|------|---------|
| Enter the dev shell (Konductor / Nix flake) | `nix develop .#frontend` |
| Install JS deps | `pnpm install` |
| Run the desktop app in dev mode | `pnpm tauri dev` |
| Run the test suites | `cargo test --workspace` and `pnpm test:all` |

The `frontend` shell provides a hermetic toolchain: Rust 1.92+, Node.js 22,
pnpm, GTK, WebKitGTK, OpenSSL, Playwright, and all linters/formatters. If
you do not use Nix, the [contributor development guide](docs/contributor/development.md)
lists the manual prerequisites.

A quick tour of the codebase lives at [`ARCHITECTURE.md`](ARCHITECTURE.md);
the deeper documentation index is at [`docs/README.md`](docs/README.md).

## How to contribute

### Reporting bugs

File issues at <https://github.com/ScopeCreep-zip/Rekindle/issues>. Use the
**Bug Report** template and include:

- Rekindle version (or commit hash if running from source).
- Operating system and version.
- Veilid network state (LAN-only, behind symmetric NAT, etc.) if relevant.
- Steps to reproduce.

**Do not file security vulnerabilities as public issues.** See
[`SECURITY.md`](SECURITY.md) for the disclosure channel.

### Suggesting features

Use the **Feature Request** template. Before proposing a feature, please
read [`docs/architecture/communities.md`](docs/architecture/communities.md) §13 ("Features Intentionally
Omitted") — many feature ideas conflict with the project's privacy posture
or P2P architecture.

### Submitting code

1. Fork the repo and create a topic branch off `main`.
2. Make your changes, keeping commits focused and self-contained.
3. Run the full test suite and ensure `cargo clippy --workspace -- -D warnings`
   is clean.
4. Open a pull request against `main`. Use the PR template.

We work on long-lived `codex/communities-*` branches for the v2.0 community
migration; if your change touches community code, please coordinate with
maintainers before targeting that branch.

## Coding standards

These are enforced by clippy and CI; they are also load-bearing for the
project's privacy and reliability properties.

### Rust

- **Zero warnings.** The workspace lints set `deny(warnings)`,
  `deny(dead_code)`, `deny(unused-imports)`, `deny(unused-variables)`,
  `clippy::all = deny`, `clippy::pedantic = warn`, plus restriction lints
  `dbg-macro = deny`, `todo = deny`, `unimplemented = deny`,
  `undocumented-unsafe-blocks = deny`.
- **Never use `#[allow(dead_code)]`.** Wire the code fully or delete it.
- **Never silence size/complexity lints with `#[allow]`.** Refactor into
  smaller helpers instead.
- **Forbid `unsafe`** in pure-logic crates (`rekindle-types`, `secrets`,
  `codec`, `records`, `gossip`, `governance`, `dm`, `calls`, `files`,
  `link-preview`, `video`, `route`). Cross-cutting crates may use `unsafe`
  with mandatory `// SAFETY:` comments per the `undocumented-unsafe-blocks`
  lint.
- **No legacy compat.** Rekindle is pre-release; drop replaced columns,
  replace fields outright, delete old code paths.
- **DB schema:** edit `src-tauri/migrations/001_init.sql` directly, bump
  `SCHEMA_VERSION` in `db.rs`. Do not add migration files.
- **Async + parking_lot:** parking_lot guards are `!Send`, so clone data
  out before `.await` points.
- **Veilid types are Arc-based:** clone `RoutingContext` and `VeilidAPI`
  from `NodeHandle` before async DHT/routing calls — do not hold them
  across await points.
- **cpal `Stream` is `!Send` on macOS:** audio streams must live on
  dedicated OS threads and bridge to Tokio via `mpsc` channels.

### TypeScript / SolidJS

- **All business logic lives in the Rust backend.** Frontend is thin and
  performant — render state, forward user actions, no business logic.
- **Tailwind 4 global styles only** in `src/styles/`. No inline class
  utilities in components.
- **Window roots are `display: flex; flex-direction: column;`** with one
  `flex: 1` child holding the footer down. Wrappers must preserve the flex
  contract or use `display: contents`.
- **IPC types use `#[serde(rename_all = "camelCase")]`** on the Rust side;
  the TS side mirrors them in `src/ipc/commands.ts` and `src/ipc/channels.ts`.

### Security-sensitive changes

If your change touches identity, encryption, transport, presence, MEK
distribution, or invite handling:

- Add a regression test that exercises the failure mode you're fixing.
- Update the relevant doc in [`docs/security/`](docs/security/) and, if a
  primitive choice changes, the corresponding ADR in
  [`docs/decisions/`](docs/decisions/).
- Mention the change explicitly in the PR description so a reviewer with
  a security focus can pick it up.

## AI-assisted contributions

Rekindle accepts AI-assisted contributions and requires you to
disclose them via an `Assisted-by:` trailer in the commit message —
the same place `Co-authored-by:` would go:

```
Assisted-by: claude-opus-4.7 (Claude Code 1.0)
```

The full policy, including which AI tools count, what
architectural invariants AI commonly breaks, slopsquatting
defence, and the warn-vs-error rollout schedule for our automated
gates, lives at
[`docs/contributor/ai-assisted-contributions.md`](docs/contributor/ai-assisted-contributions.md).
Read it once before submitting an AI-assisted PR.

## Pull request process

1. **One topic per PR.** A PR that adds a feature *and* refactors three
   modules will be asked to split.
2. **Match the existing style.** Don't reformat untouched code.
3. **Run the full test suite locally.** `cargo test --workspace`,
   `cargo clippy --workspace -- -D warnings`, `cargo fmt --all -- --check`,
   `pnpm test:all`. CI will re-run these.
4. **Keep commits clean.** No merge commits; rebase on top of `main` (or
   the targeted feature branch) before requesting review. Squash WIP
   commits.
5. **Write commit messages that explain the *why*.** The diff explains the
   *what*; the message should give a future reader the context they need
   to understand the change.

We don't currently require a Developer Certificate of Origin sign-off or a
Contributor License Agreement. By submitting a PR you agree your
contribution is licensed under the project's MIT license.

## Where to find help

- **Architecture questions:** [`docs/architecture/`](docs/architecture/) and
  [`docs/architecture/communities.md`](docs/architecture/communities.md).
- **Protocol questions:** [`docs/protocol/`](docs/protocol/) and
  [`docs/glossary.md`](docs/glossary.md).
- **Security questions:** [`docs/security/`](docs/security/). For
  vulnerability reports, [`SECURITY.md`](SECURITY.md).
- **Build / dev environment:** [`docs/contributor/development.md`](docs/contributor/development.md).

## Code of conduct

Participation in this project is governed by the
[Code of Conduct](CODE_OF_CONDUCT.md). By participating, you agree to abide
by its terms.
