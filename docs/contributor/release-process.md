# Release Process

Rekindle is **pre-1.0** — no tagged releases yet. This document
captures the intended release workflow so it is ready when the first
release lands.

## Versioning

Once we tag, we follow [Semantic Versioning 2.0](https://semver.org/):

- **MAJOR** — breaking changes to the wire protocol, schema, or
  public IPC API.
- **MINOR** — backward-compatible features.
- **PATCH** — backward-compatible fixes.

Pre-1.0 versions (`0.x.y`) are allowed to have breaking changes in
minor bumps; this is conventional Rust ecosystem behaviour.

## Branch model

- `main` — protected; PRs are squashed in.
- `codex/communities-*` — long-lived feature branches for the v2.0
  community migration.
- `release-x.y` — release branches cut at tag time, used for bugfix
  backports if needed during the release window.

## Pre-release checklist

Before tagging:

- [ ] `cargo test --workspace` is green.
- [ ] `cargo clippy --workspace -- -D warnings` is clean.
- [ ] `cargo fmt --all -- --check` is clean.
- [ ] `pnpm test:all` is green (E2E + mock-IPC suites).
- [ ] `CHANGELOG.md`'s `[Unreleased]` section is moved into a new
      versioned heading.
- [ ] `Cargo.toml` workspace `version` is bumped.
- [ ] `package.json` is bumped.
- [ ] `src-tauri/tauri.conf.json` `version` is bumped.
- [ ] `src-tauri/migrations/001_init.sql` was edited correctly and
      `SCHEMA_VERSION` in `db.rs` was bumped if the schema changed.
- [ ] [`docs/roadmap.md`](../roadmap.md) is updated.
- [ ] If any cryptographic primitive or wire format changed, the
      relevant ADR is added or updated under [`../decisions/`](../decisions/).
- [ ] If a security-relevant change landed, [`../security/threat-model.md`](../security/threat-model.md)
      is updated.

## Tagging

```sh
# from a clean main
git tag -s vX.Y.Z -m "Rekindle vX.Y.Z"
git push origin vX.Y.Z
```

Tags are signed with the project maintainer's PGP key (once it
exists; see [`../../SECURITY.md`](../../SECURITY.md)).

## Building artifacts

Artifacts are built per-OS via the Tauri bundler:

```sh
# Inside `nix develop .#frontend` on Linux,
# or with all prerequisites manually installed otherwise:
pnpm tauri build
```

This produces:

| Platform | Output |
|----------|--------|
| Windows | `src-tauri/target/release/bundle/nsis/Rekindle_X.Y.Z_x64-setup.exe` |
| macOS | `src-tauri/target/release/bundle/dmg/Rekindle_X.Y.Z_aarch64.dmg` (and x86_64) |
| Linux | `src-tauri/target/release/bundle/appimage/Rekindle_X.Y.Z_amd64.AppImage` and `bundle/deb/rekindle_X.Y.Z_amd64.deb` |

Per-OS builds happen on per-OS runners; cross-compilation is not
the supported path. The release workflow at
`.github/workflows/release.yml` orchestrates this.

## Signing

| Platform | Signing target |
|----------|----------------|
| Windows | Authenticode signature on the NSIS installer (once the project has a code-signing certificate) |
| macOS | Apple Developer ID + notarisation (once the project has a Developer ID) |
| Linux | Detached PGP signature on each artifact |

Until signing certs / Developer IDs are in place, artifacts are
unsigned. Users will see SmartScreen warnings on Windows and
"unidentified developer" warnings on macOS — see [`../user/install.md`](../user/install.md).

## Reproducible builds

The intent is **byte-for-byte reproducible builds** so any user can
verify a downloaded artifact against a from-source build. This
requires:

- Pinned Rust toolchain (`rust-toolchain.toml`).
- Pinned Cargo lockfile (`Cargo.lock`).
- Pinned pnpm lockfile (`pnpm-lock.yaml`).
- Deterministic embedded resources (icon hashes, font hashes).
- Deterministic timestamp in build metadata
  (`SOURCE_DATE_EPOCH`).

CI verification of reproducibility is on the post-1.0 roadmap and
will land alongside `docs/security/reproducible-builds.md`.

## Publishing

After a tag, the release workflow:

1. Builds artifacts on per-OS runners.
2. Generates checksums (SHA-256 + BLAKE3 alongside).
3. Signs each artifact (once signing infrastructure exists).
4. Drafts a GitHub Release with the artifacts, checksums, and the
   relevant section of `CHANGELOG.md`.
5. The maintainer reviews the draft and publishes.

The `gh` CLI handles the manual final step:

```sh
gh release create vX.Y.Z \
  --title "Rekindle vX.Y.Z" \
  --notes-file CHANGELOG-vX.Y.Z.md \
  ./artifacts/*
```

## Post-release

- [ ] Verify each artifact downloads cleanly from the Releases page
      and runs on a clean test VM (one per OS).
- [ ] Update [`../user/install.md`](../user/install.md) if any
      install instructions changed.
- [ ] Bump the `version` in the workspace `Cargo.toml` etc. to the
      next planned version with `-dev` suffix.
- [ ] Re-add an `[Unreleased]` section to `CHANGELOG.md`.
- [ ] If a security advisory accompanies the release, publish it via
      GitHub Security Advisories.

## Hotfixes

For a critical fix that needs to ship without a full release cycle:

1. Branch from the latest tag: `git checkout -b release-X.Y vX.Y.0`.
2. Cherry-pick the fix.
3. Bump the patch version.
4. Tag `vX.Y.Z+1` and follow the release workflow.
5. Forward-port the fix to `main` if it isn't already there.

## Communication

Release announcements go in:

- The GitHub release notes (canonical).
- [`CHANGELOG.md`](../../CHANGELOG.md).
- The project's chosen public channel (TBD — pre-1.0).

For security advisories, also publish via GitHub Security
Advisories — see [`../../SECURITY.md`](../../SECURITY.md).
